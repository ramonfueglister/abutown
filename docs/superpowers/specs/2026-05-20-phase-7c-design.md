# Phase 7c — Tick-Owned Runtime + Arc-Snapshot Lock-Free Reads

**Date:** 2026-05-20
**Status:** Spec
**Author:** Claude (with @ramonfueglister)

## §1 — Goal & Success Criterion

Remove `Arc<RwLock<SimulationRuntime>>` from `AppState` entirely. Replace
with:

- **Tick loop owns the runtime** — `SimulationRuntime` lives inside the
  tick task only, no locks.
- **`Arc<ArcSwap<RuntimeReadView>>`** — published by the tick loop after
  each tick. All read paths (HTTP, WS init frames, snapshot loop) load
  from this swap atomically. Wait-free.
- **`mpsc::UnboundedSender<Mutation>`** — single mutation channel. All
  write paths (commands, subscription diffs, mark-persisted) send a
  `Mutation` enum variant; the tick loop drains the channel as the first
  system in each tick.

### Acceptance

- Zero `RwLock<SimulationRuntime>` references in `backend/` (verified by
  grep).
- All workspace cargo tests pass (~181 + new tests for the queue + view).
- 158 vitest pass, tsc clean, clippy `-D warnings` clean.
- HTTP `/world`, `/mobility`, `/chunks/{x}/{y}` answer with bounded
  latency under load: a new test holds 100 concurrent commands inflight
  and verifies that 100 concurrent read requests all complete within
  5× tick interval (≤ 500 ms at 10 Hz).
- Existing `tick_100k_all_active` bench regresses ≤ 5 % vs current
  baseline (host shows 12-17 ms band — anything within is acceptable).
- Browser smoke `scripts/smoke-7b.mjs` 9/9 green.

### Out of scope

- Removing `bevy_ecs`. The user explicitly requires bevy ECS for the
  state-of-the-art ECS architecture.
- Multi-shard / parallel tick — separate Phase 8+ work.
- Wire-protocol changes — none.
- Persistence-format changes — none. The read view is derived state,
  not serialized.
- Command latency below one tick — accepted trade-off; deterministic
  tick boundary > sub-tick command apply.
- Replacing `bevy_async_ecs` library — small enough to inline.

### State-of-the-art validation

The pattern was sanity-checked against current bevy + Rust server idioms
(2026 sources):

- `mpsc` channel as the mutation queue: identical to
  [`bevy-async-ecs`](https://docs.rs/bevy-async-ecs/latest/) internals
  and recommended in
  [bevy discussion #21820](https://github.com/bevyengine/bevy/discussions/21820)
  ("Create two resources for `inbound_rx` and `outbound_tx`").
- `ArcSwap` for lock-free read snapshots: standard for the "write
  rarely / read continuously" pattern. The
  [arc-swap docs](https://docs.rs/arc-swap/) explicitly call it out as
  "semantically equivalent to `Atomic<Arc<T>>` or `RwLock<Arc<T>>`
  without the lock".
- Single-writer tick-loop owning the World: matches authoritative game
  server best practice (Lightyear, Replicon use the same shape).

We deviate intentionally on three points: no Lightyear/Replicon (own
wire protocol from Phase 7a/7b); single-thread tick (determinism over
throughput); no bevy_async_ecs dep (inline the small piece we need).

## §2 — Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ tick task (owns SimulationRuntime, no lock)                     │
│                                                                  │
│   loop every 100 ms:                                             │
│     schedule.run(&mut world)                                     │
│       │                                                          │
│       ├─ MobilitySet::Input    drain_mutations_system            │
│       │     while mutation_rx.try_recv() { apply, reply oneshot }│
│       │                                                          │
│       ├─ MobilitySet::LOD      (existing systems)                │
│       ├─ MobilitySet::Advance  (existing systems)                │
│       ├─ MobilitySet::Output   (existing systems)                │
│       │                                                          │
│       ├─ MobilitySet::Publish  build_read_view_system            │
│       │     build RuntimeReadView from world,                    │
│       │     read_view_swap.store(Arc::new(view))                 │
│       │                                                          │
│       └─ MobilitySet::Bookkeeping  tick_increment_system         │
│                                                                  │
│   broadcast per_chunk_dtos to chunk_channels (built in Publish)  │
└─────────────────────────────────────────────────────────────────┘
            ▲                                ▲
            │ Mutation                        │ Arc<View>::load()
            │                                │
┌───────────┴────────┐         ┌──────────────┴──────────────┐
│ HTTP /commands     │         │ HTTP /world /mobility       │
│ WS chunk_subscribe │         │      /chunks/{x}/{y} /health│
│ WS chunk_unsub     │         │ WS world_summary init       │
│ WS disconnect      │         │ WS chunk_subscribe init     │
│ snapshot_persist   │         │ snapshot_persist read       │
│                    │         │                              │
│ tx.send(Mutation)  │         │ arc_swap.load() — wait-free  │
│ reply.await        │         │                              │
└────────────────────┘         └──────────────────────────────┘
```

### New types

```rust
/// All mutations to the runtime flow through one channel. The tick loop is
/// the sole consumer; applies happen at MobilitySet::Input each tick.
pub enum Mutation {
    ApplyCommand {
        command: ClientCommandDto,
        reply: oneshot::Sender<Result<AppliedCommand, CommandRejection>>,
    },
    SubscriptionDiff {
        added: Vec<ChunkCoord>,
        removed: Vec<ChunkCoord>,
        reply: oneshot::Sender<Vec<MobilityChunkSnapshotDto>>,
    },
    MarkChunkSnapshotsPersisted {
        coords: Vec<ChunkCoord>,
    },
}

/// Lock-free read view of the runtime, published once per tick by the
/// tick loop. Everything readers need is pre-materialized as DTOs so
/// readers never touch the live World.
pub struct RuntimeReadView {
    pub tick: u64,
    pub world_id: WorldId,
    pub mobility_tick: u64,
    pub health: HealthResponse,
    pub world_summary: WorldSummaryDto,
    pub chunk_snapshots: HashMap<ChunkCoord, ChunkSnapshotDto>,
    /// Full mobility snapshot. Built once per tick — used by the rarely
    /// hit `/mobility` HTTP endpoint. ~101k entities ≈ ~3 ms / tick to
    /// build; acceptable since it lives inside MobilitySet::Publish and
    /// readers consume it cheaply via `.clone()` on the Arc.
    pub mobility_full_dto: MobilitySnapshotDto,
    /// Per-chunk deltas computed by the tick — also published here so
    /// the broadcast step (immediately after schedule.run) can read them
    /// without re-acquiring any handle.
    pub per_chunk_deltas: Vec<MobilityChunkDeltaDto>,
}
```

### AppState (before → after)

```rust
// before
pub struct AppState {
    runtime: Arc<RwLock<SimulationRuntime>>,
    deltas: broadcast::Sender<ServerMessageDto>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>>,
    snapshot_store: Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
}

// after
pub struct AppState {
    mutations: mpsc::UnboundedSender<Mutation>,        // writers
    view: Arc<ArcSwap<RuntimeReadView>>,               // readers
    deltas: broadcast::Sender<ServerMessageDto>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>>,
    snapshot_store: Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
}
```

The `runtime` field is gone. The tick task spawns when `AppState::new`
is called, takes ownership of the freshly-built `SimulationRuntime`,
and never gives it up.

### New SystemSets

`MobilitySet` is extended with two new variants flanking the existing
sets:

```rust
#[derive(SystemSet, ...)]
pub enum MobilitySet {
    Input,        // NEW — drain_mutations_system
    LOD,
    Advance,
    Output,
    Publish,      // NEW — build_read_view_system
    Bookkeeping,
}
```

Ordering: `Input → LOD → Advance → Output → Publish → Bookkeeping`.
`Input` runs first so mutations from the previous tick interval are
absorbed before any simulation logic. `Publish` runs after `Output` so
the read view reflects post-tick positions.

### New resources in MobilityWorld::empty()

```rust
#[derive(Resource)]
pub struct MutationRx(pub mpsc::UnboundedReceiver<Mutation>);

#[derive(Resource)]
pub struct ReadViewSwap(pub Arc<ArcSwap<RuntimeReadView>>);

#[derive(Resource)]
pub struct PendingPerChunkDeltas(pub Vec<MobilityChunkDeltaDto>);
```

`MutationRx` and `ReadViewSwap` are inserted by `AppState::new` after
creating the channel and swap; `MobilityWorld::empty()` does NOT create
them (they're server-level, not core-level). The
`PendingPerChunkDeltas` is owned by sim-core and starts empty.

## §3 — Per-callsite Migration

| Site (file:line approx) | Today | Phase 7c |
|---|---|---|
| `tick_and_fan_out` Phase 1 (app.rs:505) | `runtime.write().tick_world_mobility()` | Replaced by `schedule.run(&mut world)` inside the tick task — no lock |
| `tick_and_fan_out` Phase 2 (app.rs:523) | `runtime.read()` → build DTOs from `MobilityWorld` | `build_read_view_system` constructs DTOs inside the schedule; tick task reads `view.load().per_chunk_deltas` to fan out — no lock |
| HTTP `/health` (app.rs:259) | `runtime.read().health()` | `view.load().health.clone()` |
| HTTP `/world` (app.rs:265) | `runtime.read().world_summary()` | `view.load().world_summary.clone()` |
| HTTP `/mobility` (app.rs:271) | `runtime.read().mobility_snapshot()` | `view.load().mobility_full_dto.clone()` |
| HTTP `/chunks/{x}/{y}` (app.rs:331) | `runtime.read().chunk_snapshot(coord)` | `view.load().chunk_snapshots.get(&coord).cloned()` |
| HTTP `/commands` (app.rs:341) | `runtime.write().apply_client_command(cmd)` | `tx.send(Mutation::ApplyCommand { command, reply })`; `reply.await` |
| WS `world_summary` init frame (app.rs:386) | `runtime.read().world_summary()` | `view.load().world_summary.clone()` |
| WS `chunk_subscribe` (app.rs:580) | `runtime.write().apply_subscription_diff(&added, ..)` + read snapshots | `tx.send(Mutation::SubscriptionDiff { added, removed, reply })`; `reply.await` returns the snapshots for the WS to forward |
| WS `chunk_unsubscribe` (app.rs:616) | `runtime.write().apply_subscription_diff(.., &removed)` | Same mutation variant with empty `added` |
| WS disconnect cleanup (app.rs:463) | `runtime.write().apply_subscription_diff(.., connection.subscription)` | Same mutation variant |
| `spawn_delta_loop` next_pulse (app.rs:153) | `runtime.write().next_pulse()` | Pulse counter becomes a Resource; `build_read_view_system` emits the next value; tick task pulls from `view.load().pulse` for the broadcast |
| `spawn_snapshot_loop` collect dirty chunks (app.rs:641, 783, 844) | `runtime.read()` | `view.load().chunk_snapshots` iterated |
| `spawn_snapshot_loop` mark persisted (app.rs:689) | `runtime.write().mark_chunk_snapshots_persisted(&coords)` | `tx.send(Mutation::MarkChunkSnapshotsPersisted { coords })` (fire-and-forget; no reply) |

### Command reply semantics

`Mutation::ApplyCommand` and `Mutation::SubscriptionDiff` both carry a
`oneshot::Sender` for the reply. `drain_mutations_system` is an
**exclusive system** (`fn(&mut World)`) so it can apply the mutation
AND build the reply (snapshots for added chunks) within the same
system, reading the just-mutated world. The reply lands before the
sender's `await` wakes. If the receiver was dropped (HTTP client
disconnected), the `oneshot::send` returns Err — drain logs and
proceeds.

Worst-case latency: command arrives just after the tick started → waits
one full tick interval (100 ms) until the next tick drains the queue
and replies. Acceptable for non-realtime gameplay.

### Pulse handling

`next_pulse` mutates the runtime's pulse counter and returns the next
value. After 7c, the pulse counter becomes a `Pulse(u64)` Resource;
`build_read_view_system` increments it and includes the new value in
the read view. The tick task reads `view.load().pulse` and sends the
corresponding `ServerMessageDto::Pulse` via the existing `deltas`
broadcast.

### Spawn order in AppState::new

```rust
pub fn new_with_stores(
    runtime: SimulationRuntime,
    snapshot_store: ...,
    mobility_snapshot_store: ...,
    card_hands: CardHandStore,
    auth: AuthVerifier,
) -> Self {
    let (mutation_tx, mutation_rx) = mpsc::unbounded_channel();
    let initial_view = build_initial_read_view(&runtime);
    let view = Arc::new(ArcSwap::from_pointee(initial_view));
    let (deltas, _) = broadcast::channel(DELTA_BROADCAST_CAPACITY);
    let chunk_channels = Arc::new(DashMap::new());

    let state = Self {
        mutations: mutation_tx,
        view: Arc::clone(&view),
        deltas: deltas.clone(),
        chunk_channels: Arc::clone(&chunk_channels),
        snapshot_store: ...,
        mobility_snapshot_store: ...,
        card_hands,
        auth,
    };

    // Spawn tick task. Owns runtime, receiver, view-arc, broadcast handles.
    tokio::spawn(tick_loop(
        runtime,
        mutation_rx,
        Arc::clone(&view),
        deltas,
        Arc::clone(&chunk_channels),
        SIMULATION_TICK_INTERVAL,
    ));

    // Snapshot loop separately; sends MarkChunkSnapshotsPersisted via tx.
    state.spawn_snapshot_loop(SNAPSHOT_INTERVAL);

    state
}
```

The tick task body:

```rust
async fn tick_loop(
    mut runtime: SimulationRuntime,
    mutation_rx: mpsc::UnboundedReceiver<Mutation>,
    view: Arc<ArcSwap<RuntimeReadView>>,
    deltas: broadcast::Sender<ServerMessageDto>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>>,
    interval: Duration,
) {
    // Install mutation_rx + view as Resources in the bevy World, so the
    // Input and Publish systems can access them.
    runtime.install_phase7c_resources(mutation_rx, Arc::clone(&view));

    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        runtime.run_schedule(); // schedule.run(&mut world)
        // Fan out per-chunk deltas (now in view).
        let v = view.load();
        for delta in &v.per_chunk_deltas {
            let chunk = ChunkCoord { x: delta.chunk.x, y: delta.chunk.y };
            if let Some(sender) = chunk_channels.get(&chunk).map(|e| e.clone()) {
                let _ = sender.send(delta.clone());
            }
        }
        // Pulse.
        let _ = deltas.send(ServerMessageDto::Pulse {
            world_id: v.world_id.clone(),
            tick: v.mobility_tick,
            sequence: v.pulse,
        });
    }
}
```

## §4 — Test Strategy

### Removed tests

- `concurrent_reads_proceed_during_snapshot_persist` (app.rs:752) — was
  testing the RwLock's read-parallelism property which no longer
  exists. Replace with the new test below.

### New tests (TDD-first)

1. `lock_free_reads_under_concurrent_commands` — replace the above.
   Spawn AppState. Issue 100 `/commands` requests concurrently. While
   they're in flight, issue 100 `/world` requests. All 200 must
   complete within 5 × tick_interval (≤ 500 ms at 10 Hz). The reads
   must complete WITHOUT waiting on any command — they just `arc_swap.load`.
2. `mutation_queue_serializes_concurrent_commands` — 1000 commands sent
   concurrently. After 105 ticks (worst case), all 1000 are applied in
   the order they arrived in the mpsc channel (which mpsc guarantees for
   a single sender clone; multi-sender is racey by design).
3. `read_view_consistent_after_tick` — after one tick.run, the
   `read_view.tick` equals `mobility_tick + 1` and `chunk_snapshots`
   covers all Active/Hot chunks. No mid-tick state visible.
4. `subscription_diff_reply_carries_snapshots_for_added_chunks` — send
   `Mutation::SubscriptionDiff { added: [c1, c2], removed: [], reply }`,
   await reply, assert it contains 2 `MobilityChunkSnapshotDto`s.
5. `command_reply_arrives_within_two_ticks` — issue
   `Mutation::ApplyCommand`, measure reply latency, assert ≤ 200 ms
   (2 × tick_interval).
6. `dropped_reply_channel_does_not_kill_tick_loop` — drop the receiver
   side of a `Mutation::ApplyCommand`'s oneshot, send it anyway. Tick
   loop must continue running (the failed `reply.send` is logged but
   not fatal).
7. `mark_chunk_snapshots_persisted_via_mutation` — snapshot persist loop
   sends the fire-and-forget mutation; next tick the runtime's persisted
   set reflects the coords.

### Integration tests

- `websocket.rs` 3-client disjoint subscribe — keep, constructor change
  only.
- `phase3-mobility-snapshot.json` snapshot round-trip — keep, the read
  view doesn't touch the serialized format.

### Bench

- `tick_100k_all_active` — must regress ≤ 5 % vs the current ~12-17 ms
  band. The expected cost addition is the per-tick `build_read_view`
  pass (~3 ms to materialize `mobility_full_dto` over 101k entities).
  If the regression exceeds 5 %, either build `mobility_full_dto`
  lazily (only when an HTTP `/mobility` request is observed) or move
  it behind a `cfg(feature = "debug_endpoints")`.

### Verification gates (mandatory)

- `cargo test` workspace
- `cargo clippy --all-targets -- -D warnings`
- `npx tsc --noEmit`
- `npx vitest run`
- `node scripts/smoke-7b.mjs`

## §5 — Implementation Order

Step-by-step in the plan (writing-plans skill takes it from here):

1. Add `arc-swap` dependency to `sim-server/Cargo.toml`.
2. Define `Mutation` enum and `RuntimeReadView` struct in
   `sim-server/src/runtime_view.rs` (new file).
3. Add new SystemSets (`MobilitySet::Input`, `MobilitySet::Publish`)
   and the two systems in `sim-core/src/mobility/systems.rs`. Behind a
   bool feature flag initially so existing tests don't break.
4. Add new resources (`MutationRx`, `ReadViewSwap`, `Pulse`,
   `PendingPerChunkDeltas`) in `sim-core/src/mobility/resources.rs`.
5. Tests 1-7 from §4 — write them now, mark `#[ignore]` until the
   migration completes.
6. Refactor `AppState` — swap fields, spawn tick task, drop the
   `RwLock`.
7. Migrate each callsite per §3, one at a time. After each migration,
   run cargo test for the touched file.
8. Remove the old `tick_and_fan_out` function (its work is now in
   `build_read_view_system` + tick task's broadcast loop).
9. Un-`#[ignore]` the new tests. Confirm all pass.
10. Run all verification gates.
11. Bench. Confirm ≤ 5 % regression.
12. Progress note.

## §6 — Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Pulse-broadcast order changes vs current behavior | Pulse counter is incremented in `build_read_view_system`; tick task broadcasts in the same loop iteration. Net effect: client sees pulse at the same cadence as before. |
| `oneshot::Sender` dropped (HTTP client cancelled) | `reply.send` returns Err; drain system logs and skips. Tick loop unaffected. Test 6 covers this. |
| `mpsc::unbounded_channel` memory pressure under command flood | At 10 Hz tick and realistic command rate (< 10/s expected), bound is trivially safe. If observed in production: switch to `bounded(1024)` with backpressure on HTTP. |
| `mobility_full_dto` build cost per tick (~3 ms for 101k agents) | Bench-gated. If regression > 5 %, lazy-build behind a per-tick atomic flag set by HTTP `/mobility` reads; clear on read. |
| Existing tests construct `AppState` via `new_with_stores` | Signature stays compatible; tick task is spawned internally as before. All tests should still compile after the constructor refactor. |
| `MobilityWorld::tick_mobility` returns `HashMap<ChunkCoord, MobilityChunkDelta>` — the per_chunk deltas | Rename to `tick_world_mobility_into(&mut pending: PendingPerChunkDeltas)` OR keep the return value and let `build_read_view_system` pull it out via a transient resource. Latter is cleaner. |

## §7 — File-Level Touch List

- `backend/crates/sim-server/Cargo.toml` — add `arc-swap = "1"`
- `backend/crates/sim-server/src/runtime_view.rs` — new file:
  `Mutation`, `RuntimeReadView`, `build_initial_read_view`
- `backend/crates/sim-server/src/app.rs` — refactor `AppState`, spawn
  tick task, migrate all 13 call sites from §3
- `backend/crates/sim-core/src/mobility/systems.rs` — new
  `MobilitySet::Input` + `MobilitySet::Publish`; new
  `drain_mutations_system` + `build_read_view_system`; update
  `install_systems`
- `backend/crates/sim-core/src/mobility/resources.rs` — new
  `MutationRx`, `ReadViewSwap`, `Pulse`, `PendingPerChunkDeltas` resources
- `backend/crates/sim-server/src/runtime.rs` — `SimulationRuntime`
  gains `install_phase7c_resources(mutation_rx, view_swap)`,
  `run_schedule()`, removes pulse counter field (now Resource)
- `backend/crates/sim-server/src/websocket.rs` (or wherever WS handlers
  live) — replace `runtime.write()` calls with `tx.send(Mutation::...)`
- `progress.md` — entry on completion

No frontend, no wire protocol, no persistence-schema files touched.

## §8 — Rollback

Each commit is one logical step (per §5). If any step fails CI, revert
that commit. The schema (`MobilitySnapshotDto`, `ChunkSnapshotDto`,
wire format) is untouched, so DB rollback is never required. If the
full migration goes sideways, revert all 7c commits — the prior
`RwLock` architecture from Phases 7a/7b remains intact.
