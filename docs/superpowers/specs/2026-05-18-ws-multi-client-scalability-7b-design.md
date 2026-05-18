# Phase 7b — WS Multi-Client Scalability (Per-Chunk Broadcast Channels)

**Status:** Design approved, awaiting implementation plan
**Date:** 2026-05-18
**Predecessor:** Phase 7a — Viewport-Driven Subscriptions + RwLock (commits `9882cf1 … 7afa293`)
**Successor:** Phase 7c — Arc-Snapshot Lock-Free Reads (deferred, not in this spec)

## Problem

After Phase 7a + the lock-cliff fix, the runtime read lock can be acquired
concurrently and DB writes don't block readers. **But** every WS tick still
runs N per-client filters (`filtered_mobility_delta_from_dto`), each
walking the global `MobilityDelta` to extract the subset that intersects
the client's subscription set. At 500 clients × 10k changed entities per
tick, that's **5M iterations per tick across all client filter tasks**,
serialised through the (now-shared but still single) `tokio::broadcast`
channel. The filter cost scales with `N_clients × N_changed`, not with
`N_clients × N_subscribed_per_client`.

The architectural fix: shard the broadcast by chunk. The tick loop emits a
per-chunk delta to a per-chunk `tokio::broadcast` channel. Each WS handler
holds receivers only for its subscribed chunks and pays no filter cost.
Per-client work scales with `N_subscribed_per_client`, not `N_world`.

This is Phase 7b of the WS scalability arc.

## Goals

- Per-client per-tick work scales with the client's own subscription size,
  not with the world's total changed-entity count.
- The runtime tick loop publishes once per Active|Hot chunk; no per-client
  fan-out happens in the tick path itself.
- The 3-client integration test (Phase 7a) keeps passing; assertions
  about disjoint subscriptions seeing disjoint entities now hold by
  construction (no shared filter step) rather than by filter correctness.
- The Phase 7a browser smoke (`scripts/smoke-7a.mjs`, adapted for 7b)
  shows the new `MobilityChunkDelta` / `MobilityChunkSnapshot` frames in
  DevTools instead of the old global `MobilityDelta`.

## Non-Goals (Phase 7c+)

- Lock-free reads via `arc-swap` / `Arc<RuntimeSnapshot>` (separate
  spec).
- Binary wire format (e.g. msgpack, capnp). JSON stays; bandwidth fight
  is a later phase.
- Delta compression, dead reckoning, predictive interpolation hints.
- Server-pushed chunk-activity events (client doesn't need them for 7b).

## Wire format change — "state of the art, remove the old crap"

The current `ServerMessageDto::MobilityDelta` (global filtered delta)
goes away entirely. Replaced by two new server-pushed message types:

```rust
ServerMessageDto::MobilityChunkDelta {
    chunk: ChunkCoordDto,
    changed_agents: Vec<AgentRecordDto>,
    changed_vehicles: Vec<VehicleRecordDto>,
    left_agents: Vec<EntityId>,    // entities that have moved out of this chunk
    left_vehicles: Vec<EntityId>,
}

ServerMessageDto::MobilityChunkSnapshot {
    chunk: ChunkCoordDto,
    agents: Vec<AgentRecordDto>,
    vehicles: Vec<VehicleRecordDto>,
}
```

`MobilityChunkSnapshot` is emitted exactly once when a client first
subscribes to a chunk (replaces the old "synthetic delta" mechanism).
After that, only `MobilityChunkDelta` messages for that chunk flow until
the client unsubscribes.

This is a protocol-breaking change. Frontend `applyServerMessage` and
`applyMobilityDelta` need new branches for the new types and the old
`MobilityDelta` branch is removed. No backwards-compat shim — the old
filtered-broadcast architecture is the "crap" being removed.

## Server architecture

### New AppState resource

```rust
pub struct AppState {
    runtime: Arc<RwLock<SimulationRuntime>>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDelta>>>,
    snapshot_store: Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
}
```

- `chunk_channels` is a `DashMap` (not `Arc<RwLock<HashMap>>`) so per-key
  inserts and lookups during subscribe/tick don't contend with each other.
- The old `deltas: broadcast::Sender<ServerMessageDto>` field is deleted.
- The old `subscribe_deltas()`, `spawn_delta_loop` are deleted in favour
  of the new tick-loop publishing directly into `chunk_channels`.

### Tick-loop fan-out

`MobilityWorld::tick_mobility()` signature changes:
```rust
pub fn tick_mobility(&mut self) -> HashMap<ChunkCoord, MobilityChunkDelta>
```

Implementation: the dirty-tracking infrastructure (`DirtyAgents`,
`DirtyVehicles`) already exists; the LOD systems
(`track_chunk_populations_system`) already group entities by chunk via
`AgentsByChunk` / `VehiclesByChunk`. Build per-chunk deltas by walking
the dirty sets and grouping by current chunk. Compute `left_*` by
comparing each dirty entity's current chunk to its previous chunk (stored
in a new `PreviousAgentChunks` / `PreviousVehicleChunks` resource updated
at end of tick).

The fan-out site (`spawn_delta_loop` or its successor):
```rust
let per_chunk_deltas = {
    let mut runtime = state.runtime.write().await;
    runtime.tick_mobility()
};
for (chunk, delta) in per_chunk_deltas {
    if let Some(sender) = state.chunk_channels.get(&chunk) {
        let _ = sender.send(delta); // best-effort, ignore receiver count
    }
}
```

The tick loop still holds the runtime write lock briefly to execute the
tick. After that, the per-chunk send is lock-free (DashMap reads + the
broadcast channel's own internal sync).

### Channel lifecycle: aggressive reap

Channel created on first subscribe to a chunk; destroyed on last
unsubscribe. Implemented inside `apply_subscription_diff` (or its WS
counterpart): when `ChunkSubscribers` transitions 0→1 for a chunk,
`chunk_channels.entry(coord).or_insert_with(|| broadcast::channel(CAP).0)`.
When the count goes 1→0, `chunk_channels.remove(&coord)`.

Bounded memory: `O(N_active_chunks)` channels, each ~1 KB. With 500
clients × 30 chunks subscribed each ≈ 50–200 unique chunks. Total
overhead < 200 KB.

`CAP` (broadcast channel buffer size): 8 messages. Each message is the
tick's per-chunk delta. At 10 Hz tick, that's 800 ms of buffering — far
more than a slow client should be allowed to lag before being dropped.
Lagged-receiver semantics: tokio's broadcast drops oldest, slow clients
miss intermediate ticks but still get the latest state.

### Per-handler stream multiplexing

Each WS handler owns:
- `connection: ConnectionState` (existing — tracks subscriptions, last-visible)
- `chunk_streams: StreamMap<ChunkCoord, BroadcastStream<MobilityChunkDelta>>`

`StreamMap` (from `tokio-stream`) is the right primitive for dynamic
stream multiplexing — you insert/remove streams by key, and `.next()`
yields whichever stream fires first. No need to spawn one task per chunk
subscription.

Handler main loop:
```rust
loop {
    tokio::select! {
        inbound = socket.recv() => { /* handle client message */ }
        Some((chunk, item)) = chunk_streams.next() => {
            match item {
                Ok(delta) => send_server_message(&mut socket,
                    ServerMessageDto::MobilityChunkDelta(delta)).await?,
                Err(BroadcastStreamRecvError::Lagged(_)) => {
                    // client is too slow — drop them or emit a re-snapshot
                    // for the chunk to recover
                    let snapshot = build_chunk_snapshot(state, chunk).await;
                    send_server_message(&mut socket,
                        ServerMessageDto::MobilityChunkSnapshot(snapshot)).await?;
                }
            }
        }
    }
}
```

### Subscribe flow

`handle_client_message` on `ChunkSubscribe { coords }`:
1. For each added coord (per Phase 7a diff logic):
   - Increment `ChunkSubscribers`
   - `chunk_channels.entry(coord).or_insert_with(channel)`
   - `receiver = chunk_channels[coord].subscribe()`
   - `chunk_streams.insert(coord, BroadcastStream::new(receiver))`
   - Build current snapshot of chunk: `build_chunk_snapshot(state, coord)` (collects agents+vehicles whose `chunk_of(position)` == coord under a read-lock)
   - Send `ServerMessageDto::MobilityChunkSnapshot { chunk, agents, vehicles }`
2. For each removed coord:
   - Decrement `ChunkSubscribers`
   - `chunk_streams.remove(coord)` (drops the receiver)
   - If `ChunkSubscribers[coord] == 0` → `chunk_channels.remove(coord)`

The synthetic-snapshot-on-subscribe is a write to the socket inside the
WS handler, sequenced before subsequent tick deltas. The client applies
snapshot first, then deltas. Order is preserved by single-WS-stream
guarantees.

### Disconnect cleanup

In `stream_world_deltas` cleanup block (Phase 7a):
- For each coord in `connection.subscription`:
  - Decrement `ChunkSubscribers`
  - If count drops to 0 → `chunk_channels.remove(coord)`
- `chunk_streams` is dropped along with the handler task (drops all
  receivers automatically).

## What gets deleted

- `tokio::broadcast::Sender<ServerMessageDto>` in `AppState` and all
  related plumbing (`subscribe_deltas`, `spawn_delta_loop`).
- `ServerMessageDto::MobilityDelta` variant (in `abutown-protocol`).
- `MobilityDeltaDto` (the top-level filtered-delta DTO) is deleted; its
  `changed_*` / `left_*` fields move into `MobilityChunkDelta` directly,
  no wrapper struct. `AgentRecordDto` / `VehicleRecordDto` (the entity
  payload types) are kept — they're the items inside both
  `MobilityChunkDelta.changed_*` and `MobilityChunkSnapshot.*`.
- `SimulationRuntime::filtered_mobility_delta_from_dto` (~50 LOC).
- `SimulationRuntime::synthetic_mobility_delta_for_subscription` (~30 LOC).
- `SimulationRuntime::next_mobility_delta` (~5 LOC).
- `SimulationRuntime::next_server_messages` (~10 LOC) — the tick loop
  builds messages directly now.
- Frontend `applyMobilityDelta`'s global-filter branch.

## Frontend architecture

- `src/backend/mobilityProtocol.ts`: add `MobilityChunkDeltaDto`,
  `MobilityChunkSnapshotDto`, both server-pushed; remove the old
  `MobilityDeltaDto` parser branch.
- `src/backend/mobilityState.ts`: `applyServerMessage` learns the two
  new variants. `applyMobilityChunkDelta` merges per-chunk: for each
  changed entity, replace in state; for each `left_*`, remove from
  state. `applyMobilityChunkSnapshot` replaces all entities for that
  chunk with the snapshot's contents.
- `src/backend/mobilityClient.ts`: no changes (the subscription poll and
  diff already operate on `ChunkCoord` sets — they don't care about
  per-chunk delta semantics).

## Frame ordering guarantees

The current frame order from server to a single client must be:
1. `hello` (once, on connect)
2. For each subscribe message:
   - `MobilityChunkSnapshot { chunk, ... }` per added chunk
3. Per tick:
   - `MobilityChunkDelta { chunk, ... }` per (subscribed-chunk × changed-this-tick) pair

The client applies snapshots before deltas because they're sent in order
on a single WS stream. The `tile_pulse` and other non-mobility frames
interleave freely.

If a `MobilityChunkDelta` arrives for a chunk the client doesn't
currently know about (race during unsubscribe), client drops it
silently.

## Error handling

- **Lagged broadcast receiver** (slow client misses ticks): when the WS
  handler's `BroadcastStream` reports `Err(Lagged(_))`, the handler
  rebuilds and sends a fresh `MobilityChunkSnapshot` for that chunk.
  This is a recovery rather than a connection close.
- **Channel not found at tick time**: `chunk_channels.get(&chunk)`
  returns `None` for a chunk that has no subscribers. Tick loop just
  skips it — no work, no error.
- **Channel created during tick** (subscribe arrives while tick is
  in flight): the receiver is created but the current tick's delta has
  already been sent. Client will receive next tick's delta plus the
  immediate `MobilityChunkSnapshot` from subscribe — overlap is benign.
- **WS-send failure**: existing path — close socket, run cleanup, drop
  all `chunk_streams` entries (which decrements subscribers).

## Testing

### Backend unit tests (new)

- `tick_mobility_produces_per_chunk_deltas` — seeds agents in 3 different
  chunks, ticks once, asserts the return map has exactly the chunks
  with changes.
- `tick_mobility_omits_unchanged_chunks` — seeds an agent with `walk_speed=0`
  in chunk A, dirty agent moves in chunk B, ticks, asserts returned map
  contains only B.
- `chunk_channel_lifecycle_aggressive_reap` — subscribe to chunk X,
  assert channel exists in AppState. Unsubscribe last receiver, assert
  channel removed.
- `lagged_receiver_triggers_snapshot_resync` — fill a broadcast channel
  past CAP, advance the receiver, assert the handler recovery path
  fires a snapshot.

### Backend integration tests (port from 7a, adapt to new protocol)

- `three_clients_with_disjoint_subscriptions_see_only_their_chunks` —
  same scenario, but now each client receives `MobilityChunkSnapshot`
  per subscribed chunk and only `MobilityChunkDelta` for those chunks.
  Disjoint by construction (no shared filter step).
- `subscribe_emits_snapshot_then_only_per_chunk_deltas` — subscribe to
  chunk X, assert next received message is `MobilityChunkSnapshot` for
  X, and no global `MobilityDelta` arrives.

### Frontend unit tests (vitest)

- `applyMobilityChunkSnapshot replaces all entities for that chunk` —
  state has agents in chunks A and B; snapshot for A arrives with
  different agents; B's agents are untouched, A's are replaced.
- `applyMobilityChunkDelta updates and removes` — replaces `changed_*`
  in state; removes ids listed in `left_*` even if their last-known
  chunk was elsewhere.
- `applyMobilityChunkDelta for unknown chunk is silent` — applies
  cleanly when client has no record of that chunk (race during
  unsubscribe).

### Browser smoke (CLAUDE.md mandate — frontend wire change)

Adapt `scripts/smoke-7a.mjs` → `scripts/smoke-7b.mjs`:
- Verify initial subscribe → server emits 1×`mobility_chunk_snapshot`
  per subscribed chunk (instead of the old `mobility_delta`).
- Verify pan → `chunk_subscribe` + new chunks' snapshots arrive;
  `chunk_unsubscribe` for chunks leaving.
- Verify per-tick: only `mobility_chunk_delta` frames for subscribed
  chunks, never any global `mobility_delta` frame.
- Verify idle: per-tick frames stop when no entities move in subscribed
  chunks.

## Concrete sequencing for the implementation plan

1. **Protocol**: add `MobilityChunkDelta`, `MobilityChunkSnapshot` DTOs.
   Keep the old `MobilityDelta` variant temporarily (server still sends
   it) so the build doesn't break mid-refactor.
2. **MobilityWorld**: extend tick to produce per-chunk delta map.
3. **AppState**: add `chunk_channels: Arc<DashMap<…>>`. Keep old
   `deltas` channel for now.
4. **WS handler subscribe path**: on subscribe, create channel +
   receiver + send initial snapshot. On unsubscribe, drop receiver +
   reap channel if last.
5. **WS handler stream loop**: rewrite `select!` to use `StreamMap`
   over per-chunk receivers + socket.
6. **Tick loop fan-out**: rewrite `spawn_delta_loop` to publish
   per-chunk deltas to `chunk_channels` instead of the global broadcast.
7. **Delete old paths**: remove `deltas` channel, `filtered_mobility_…`,
   `synthetic_mobility_…`, `MobilityDelta` variant. Frontend variant
   gone too.
8. **Frontend protocol + state**: add new variant handling, remove old.
9. **Tests**: port existing integration tests; add new unit tests.
10. **Browser smoke** (`scripts/smoke-7b.mjs`): verify chunk-frame flow.
11. **Quality gate**: cargo + vitest + clippy + tsc + browser smoke.
12. **progress.md** + push.

## Risks

- **MobilityChunkDelta "left" semantics**: an entity moving from chunk A
  to chunk B should appear as `left_*` in A's delta AND `changed_*` in
  B's delta. Subscribers to both will see it leave A and arrive in B.
  Subscribers to only A will see it disappear (correct). Subscribers
  to only B will see it appear (correct). Mid-tick race: if a client
  subscribes to B between A's delta and B's delta within the same
  tick, that's fine because the snapshot-on-subscribe captures the
  current state.
- **Stream backpressure with StreamMap**: if a chunk has high update
  frequency and the client's WS send is slow, the BroadcastStream may
  fill its capacity and start dropping. The Lagged-recovery path
  (re-snapshot) handles this but the snapshot itself takes a read lock
  on the runtime, briefly competing with other readers. Acceptable.
- **DashMap vs RwLock<HashMap>**: I chose DashMap for the `chunk_channels`
  to avoid contention between subscribe (write) and tick (read).
  DashMap is per-key locking, exactly what we want. Adds a dependency
  (`dashmap` crate) but it's a single small, mature dependency.
- **`PreviousAgentChunks` resource for `left_*` computation**: requires
  updating at end of tick. Memory cost: HashMap<AgentId, ChunkCoord> ~
  100k entries × ~30 bytes = ~3 MB. Acceptable.

## Open questions

None for Phase 7b. Phase 7c (Arc-snapshot reads) is the next architectural
step after 7b is validated under load.
