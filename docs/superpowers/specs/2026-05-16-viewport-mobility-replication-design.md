# Viewport-Filtered Mobility Replication

> **Phase 4 of the million-agent roadmap.** Parent spec: `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md`.

## Purpose

After Phase 3 the backend simulates ~1015 entities (~960 walking agents + 51 cars + 4 trams) and broadcasts the full `MobilityDelta` to every WebSocket client on each 10 Hz tick. Above ~1k visible entities this saturates the per-connection bandwidth budget — each client pays for the entire world even though only a small window is on screen.

This phase introduces **Area-of-Interest (AoI) filtering** per the standard MMO pattern documented in the repo's `unity-netcode-ghost-snapshots.html` and `unreal-mass-entity.html`: a client only receives delta updates for entities inside its viewport.

The AoI gate is a per-connection **chunk subscription set**. Today the backend has no client→server message and no per-connection state — `TilePulse` and `MobilityDelta` are global broadcasts via a `tokio::sync::broadcast` channel. This phase introduces:

1. A new client→server WebSocket message `ChunkSubscribeDto { coords: Vec<ChunkCoordDto> }` (additive — extends the chunks the client wants).
2. A new client→server message `ChunkUnsubscribeDto { coords: Vec<ChunkCoordDto> }` (subtractive).
3. Per-connection state in the WS task: the current subscription set and `last_visible_ids` for `entered`/`left` diffing.
4. `MobilityDelta` is filtered per connection against the subscription before sending.
5. `TilePulse` is **not** filtered in this phase — keeping it as a global broadcast is fine because it's small (one `(coord, local_index, kind)` per pulse). Phase 4 scope is mobility AoI only; TilePulse AoI is a follow-up if it ever becomes a bandwidth concern.

After this phase the backend can sustainably populate ~10k entities — clients only pay for the ~200 entities visible in their viewport.

## Non-Goals

- Server-side spatial index (deferred to Phase 6 Chunk-LOD).
- Bandwidth coalescer / rate limiter (single delta per tick is fine).
- Separate mobility-only subscription decoupled from tile pulse subscription (YAGNI; revisit when zoom-modes diverge).
- Heuristics for "predictive prefetch" (e.g. fade-in cars approaching the viewport) — `buffer_tiles` is the existing chunk-subscription mechanism, no new logic.

## Architecture

### Per-connection AoI filter

The WS task lives in `backend/crates/sim-server/src/app.rs` (`stream_world_deltas`). Today it just forwards every broadcast `ServerMessageDto` to the socket. We refactor it into a `select!` loop that handles two sources:

- Inbound client messages from `socket.recv()` — `ChunkSubscribe`/`ChunkUnsubscribe` mutate the per-connection state.
- Outbound broadcast messages from `state.subscribe_deltas()` — pass through `TilePulse`/`WorldEvent`/`Hello` unmodified; `MobilityDelta` goes through the per-connection filter.

Per-connection state struct (introduced in this phase):

```rust
struct ConnectionState {
    subscription: HashSet<ChunkCoord>,
    last_visible_agents: HashSet<EntityId>,
    last_visible_vehicles: HashSet<EntityId>,
}
```

For each tick:

1. `SimulationRuntime` computes the global `MobilityDelta { changed_agents: Vec<AgentRecord>, changed_vehicles: Vec<VehicleRecord> }` **once**.
2. For each connected client, run the per-connection filter:
   ```
   ConnectionMobilityFilter {
     subscription: ChunkSubscription,         // shared with TilePulse
     last_visible_ids: HashSet<EntityId>,     // who this client believes exists
   }
   ```
   The filter produces a `MobilityDeltaDto` containing:
   - `changed_agents`: agents whose `chunk_of(world_coord) ∈ subscription` AND (id is new to this client OR was in delta.changed_agents)
   - `changed_vehicles`: same rule for vehicles
   - `left_agents`: ids in `last_visible_ids` that are no longer visible in this tick (left the subscription)
   - `left_vehicles`: same for vehicles
3. Update `last_visible_ids` to the new visible set.
4. Send the filtered delta over the WebSocket; skip the send entirely if all four arrays are empty.

`chunk_of(world_coord) = (floor(x / 32), floor(y / 32))` where `32` is the existing `CHUNK_SIZE`.

### `InVehicle` agents

Task 7 already filters `InVehicle` agents out of the source `changed_agents`. The AoI filter sees the post-filter list and does not see drivers. Vehicles carry their `occupants` field as-is — clients reconstruct passenger state from snapshot+`InVehicle` if they need it.

### Subscription change

When a client sends `chunk_subscribe` or `chunk_unsubscribe`:

- Compute the new `subscription`.
- For all currently-known entities, recompute visibility against the new subscription.
- Send a synthetic `MobilityDeltaDto`:
  - `changed_*`: entities now visible that weren't before (full `AgentMobilityDto` shape, like a small snapshot).
  - `left_*`: entities no longer visible.
- Update `last_visible_ids`.

This is one extra delta send per subscription change. With debounced camera updates (handled client-side), one subscription change per ~250ms is the worst case.

### Protocol changes

New `ClientMessageDto` enum (discriminated union, snake_case):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessageDto {
    ChunkSubscribe(ChunkSubscribeDto),
    ChunkUnsubscribe(ChunkUnsubscribeDto),
}

pub struct ChunkSubscribeDto {
    pub protocol_version: u16,
    pub coords: Vec<ChunkCoordDto>,
}

pub struct ChunkUnsubscribeDto {
    pub protocol_version: u16,
    pub coords: Vec<ChunkCoordDto>,
}
```

`MobilityDeltaDto` gains two fields:

```rust
pub struct MobilityDeltaDto {
  pub protocol_version: u32,
  pub world_id: WorldId,
  pub tick: u64,
  pub changed_agents: Vec<AgentMobilityDto>,
  pub changed_vehicles: Vec<VehicleMobilityDto>,
  pub left_agents: Vec<EntityId>,    // new
  pub left_vehicles: Vec<EntityId>,  // new
}
```

The two new fields are `Vec<EntityId>` (just strings). Serialised in snake_case to match existing convention. Frontend `applyMobilityDelta` drops the IDs from its `agents`/`vehicles` maps before applying the updates.

`MobilitySnapshotDto` shape unchanged.

### Initial connection

Before the first `chunk_subscribe` arrives, the connection's subscription is empty. The filter produces empty arrays each tick → nothing is sent for mobility. The frontend will send `chunk_subscribe` right after `Hello` for the chunks currently in viewport (new behavior introduced by this phase), so the first mobility data lands inside one tick (~100ms) of WS open.

`Hello` is sent on connect as today (unchanged). `MobilitySnapshotDto` is **not** auto-sent on connect anymore — it's emitted as a synthetic delta when the first `chunk_subscribe` arrives, containing all entities in the just-subscribed chunks.

## Frontend changes

`src/backend/mobilityState.ts` — `applyMobilityDelta`:

```ts
for (const id of delta.left_agents ?? []) state.agents.delete(id);
for (const id of delta.left_vehicles ?? []) state.vehicles.delete(id);
// existing apply for changed_agents / changed_vehicles
```

Default `[]` for backward compatibility with snapshot tests that omit the field; the runtime DTO guard accepts missing/empty arrays.

Add a new `src/backend/chunkSubscriptionClient.ts` module that:

1. Computes the visible chunk set from the current camera viewport (intersect with world bounds).
2. Sends a `chunk_subscribe` over the WebSocket immediately after `Hello` is received.
3. Diffs visible chunks on camera change (with a 250 ms debounce); sends `chunk_subscribe` for newly visible chunks and `chunk_unsubscribe` for chunks that left.

For Phase 4 default, send a generous initial subscription (e.g. the full 8×8-chunk world) so the user sees everything as today — the bandwidth win comes when zoom/clipping is later refined per camera. This keeps the visible UX unchanged in this phase; the architectural change is the gate, not the policy.

## Testing

**Unit (Rust, in `sim-server` or `sim-core`):**
- `connection_filter_drops_entities_outside_subscription` — agents in chunk (1,1) excluded when subscription = {(4,4)}.
- `connection_filter_emits_left_for_entity_that_moved_out` — agent was in (4,4), now in (3,4); subscription = {(4,4)}; expect agent in `left_agents`.
- `connection_filter_emits_join_for_entity_that_moved_in` — agent was in (3,4), now in (4,4); subscription = {(4,4)}; expect agent in `changed_agents`.
- `connection_filter_skips_send_when_all_empty` — no visible entities and nothing left; tick produces no WS frame.
- `subscription_change_emits_join_and_left_for_diff` — subscription changes from {(4,4)} to {(5,4)}; agents in (4,4) get `left_agents`, agents in (5,4) get `changed_agents` (with full DTO).

**Integration (WS test in `sim-server/tests/websocket.rs`):**
- Two clients subscribe to different chunk sets; assert each receives only its subset of entities.
- Single client subscribes to A, then to A∪B (extend); assert join-delta for B's new entities.
- Single client subscribes to A∪B, then to A (shrink); assert left-delta for B's removed entities.

**Frontend (Vitest):**
- `mobilityState.test.ts`: `applyMobilityDelta` with `left_agents: ['a:1']` drops the agent from state.
- `mobilityProtocol.test.ts`: parses `MobilityDeltaDto` with and without the `left_*` fields (backward-compat).

**E2E:** no new spec needed — render-smoke continues to assert tick reception. The bandwidth change is invisible to the spec.

## Backward compatibility

- A client running pre-Phase-4 code (no `chunk_subscribe`) sees nothing until they upgrade. Acceptable: same repo, frontend and backend ship together.
- `left_agents`/`left_vehicles` default to empty in serde — old snapshot fixtures missing the fields still parse.

## Risks

- **Subscription-update storm:** if the user pans fast, every camera-move could re-send a synthetic delta. Mitigation: client debounces `chunk_subscribe` to 250 ms (already the behavior for tile pulses — verify in planning).
- **`last_visible_ids` memory:** at worst case a client has subscribed to the whole world; the `HashSet<EntityId>` grows to ~1000 strings per connection. Cheap.
- **Per-tick filter cost:** O(n) over `changed_agents` + `changed_vehicles` per connection, where n = delta size. For 100 connections × 1000 changed entities = 100k filter calls per tick. Acceptable at current scale; if connections grow, Phase 6 spatial index amortises.

## Success criteria

- A client with subscription `{(4,4), (5,4)}` receives only agents/vehicles whose `world_coord` is in those chunks.
- Camera pan from `{(4,4)}` to `{(5,4)}` produces exactly one delta containing `left_*` for old chunk's entities and `changed_*` for new chunk's entities (snapshot-like).
- Bandwidth per client drops from ~1015 entities/tick to ~150-200 entities/tick at default zoom (~10-15 visible chunks × ~16 entities/chunk).
- All existing E2E and unit tests pass.
- New unit tests for `ConnectionMobilityFilter` cover the join/leave/still-visible matrix.
