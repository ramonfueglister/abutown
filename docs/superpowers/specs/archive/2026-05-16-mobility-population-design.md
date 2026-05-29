# Mobility Population & Persistence Design

**Date:** 2026-05-16
**Status:** Approved for planning
**Predecessor:** `2026-05-15-chunk-recovery-design.md` (chunk recovery merged; mobility was explicitly deferred)
**Successor (out of scope):** Player-mutable mobility (commands, events, replay), pathfinding over chunk tiles, route editing.

---

## Goal

After a `sim-server` start, the world contains a small set of routes, stops, vehicles, and agents that visibly move (`tick_mobility` already drives them). After a restart, the same entities resume from where they were, with `tick` and all per-entity state preserved.

## Non-Goals

- Mobility commands from clients (no `MobilityCommandDto`).
- Mobility events in `world_events` or replay logic.
- Dynamic route editing.
- A* / pathfinding over actual chunk tiles. Routes are hardcoded coordinate lists.
- Cross-chunk consistency across lazy chunk loads.
- Mobility throughput beyond what `tick_mobility` already handles deterministically.

## Architecture

Mobility splits cleanly into three concerns:

- **Infrastructure (routes, stops):** hardcoded at first server start. Immutable thereafter.
- **Active entities (vehicles, agents):** spawned by an initial seeder, then mutated only by `tick_mobility`. No external mutation API.
- **Persistence:** full `MobilityWorld` snapshot per world, one row per `world_id` in a new `mobility_snapshots` Postgres table. Written from the existing snapshot loop. Read once at startup hydration.

Because `tick_mobility` is deterministic given identical state and tick count, a snapshot + the tick counter is sufficient for full recovery. **No event log is needed for mobility in this slice.**

## Data Model Changes

### Serde on existing mobility types

The five records (`AgentRecord`, `VehicleRecord`, `StopRecord`, `RouteRecord`, `MobilityWorld`) and their dependent enums (`AgentMobilityState`, `PlanStage`) gain `#[derive(Serialize, Deserialize)]`. Fields remain crate-private; Serde sees them via the derive without changing visibility.

The `HashMap<K, V>` fields serialize as JSON objects. The `VecDeque<AgentId>` on `StopRecord.waiting_agents` serializes as a JSON array.

### New table

```sql
CREATE TABLE mobility_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

UPSERT on every write. Single row per world. No history retention in this slice (the snapshot is the source of truth; restart loads the latest).

## Store Trait

In `backend/crates/sim-core/src/persistence.rs` (or a new submodule):

```rust
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct MobilitySnapshotStoreError {
    message: String,
}

impl MobilitySnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self { /* ... */ }
}

#[async_trait]
pub trait MobilitySnapshotStore: std::fmt::Debug + Send {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityWorld,
    ) -> Result<(), MobilitySnapshotStoreError>;

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, MobilityWorld)>, MobilitySnapshotStoreError>;
}
```

Two implementations:

- `InMemoryMobilitySnapshotStore` (sim-core): a `HashMap<String, (u64, MobilityWorld)>`.
- `PostgresMobilitySnapshotStore` (sim-server): UPSERT via `INSERT ... ON CONFLICT (world_id) DO UPDATE SET tick = $2, payload = $3, updated_at = now()`. JSON encoding via Serde, decode via `serde_json::from_value`.

## Initial Seeder

Lives in a new module `backend/crates/sim-core/src/mobility/seed.rs` (split the file — `mobility.rs` is already 769 lines):

```rust
pub fn initial_world() -> MobilityWorld {
    // Returns a populated world with:
    //   - 2 routes (route:horizontal, route:vertical)
    //   - 4 stops (start+end on each route)
    //   - 4 vehicles (2 per route)
    //   - 20 agents with randomized but deterministic plans
}
```

Routes traverse the seeded chunk coordinates `(4,4) ↔ (5,4)` and `(4,4) ↔ (4,5)`. Links are simple ID strings — the chunk tile traversal stays in the existing tick logic (which is link-id-driven, no geometric pathfinding today). Agent plans cycle through `WalkToStop → RideToStop → WalkToActivity → Activity` and may repeat or terminate.

Determinism: the seeder uses a hardcoded seed (e.g., `StdRng::seed_from_u64(0xab17_0517)`) so test runs and production starts are byte-identical.

## Runtime Integration

`SimulationRuntime` gains a third store field:

```rust
pub struct SimulationRuntime {
    // ... existing ...
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
    // ...
}
```

Three constructors update:

- `new()` — adds `Box::new(InMemoryMobilitySnapshotStore::default())`.
- `new_with_stores(event_store, snapshot_store)` — adds an in-memory default for compatibility.
- `new_with_all_stores(event_store, snapshot_store, mobility_snapshot_store)` — new full-control constructor.

`hydrate_from_stores` becomes:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
) -> Result<Self, HydrationError>
```

Hydration sequence after chunk recovery:

1. `mobility_snapshot_store.read(world_id)` →
   - `Some((tick, world))`: `self.mobility = world`. The `MobilityWorld.tick` field is restored from the persisted struct (the row's `tick` column is a denormalization for indexing/observability, not the source of truth).
   - `None`: `self.mobility = mobility::seed::initial_world()`.
2. Continue with existing global `tick`/`version` restoration (these remain world-event-driven, not mobility-driven).

`HydrationError` gains a `Mobility(MobilitySnapshotStoreError)` variant.

`SimulationRuntime` gains:

```rust
pub async fn persist_mobility_snapshot(&mut self) -> Result<(), MobilitySnapshotStoreError> {
    self.mobility_snapshot_store
        .write(&self.world_id.0, self.mobility.tick(), &self.mobility)
        .await
}
```

Called from the same snapshot loop that today calls `persist_chunk_snapshots`. The loop in `app.rs` adds one extra `runtime.persist_mobility_snapshot().await?` after the chunk write. No throttling beyond the loop's existing interval — `MobilityWorld` is small enough (single row, < 1 MB even with hundreds of agents) that this is acceptable.

## Wiring

`build_app_from_config` constructs all three stores against the same `database_url`:

```rust
let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&config.database_url).await?;
```

The in-memory path (`build_app()`) constructs `InMemoryMobilitySnapshotStore::default()`. Existing test code that uses `SimulationRuntime::new()` continues to work because `new()` injects an in-memory mobility store automatically.

## Failure Policy

Same as chunk recovery: any error during hydration (read, decode, malformed payload) is fatal — server refuses to start. Silent fallback to fresh seed would discard the in-flight mobility state without an operator signal.

Snapshot-loop write failures are logged but do not crash the server (the runtime continues, the snapshot is retried next loop tick — matches the existing chunk-snapshot behavior).

## Testing Strategy

**Unit (sim-core):**

- `MobilityWorld` round-trips through `serde_json::to_value` → `from_value` and is `PartialEq`-equal to the original. Covers `HashMap`/`VecDeque` ordering quirks.
- `mobility::seed::initial_world()` produces exactly: 2 routes, 4 stops, 4 vehicles, 20 agents. Each entity passes basic invariants (agent plans are non-empty, vehicle capacity > 0, stops reference valid routes).
- `InMemoryMobilitySnapshotStore::write`/`read` round-trip.
- Determinism: two calls to `initial_world()` produce equal worlds.

**Unit (sim-server):**

- `SimulationRuntime::new_with_all_stores` + `persist_mobility_snapshot` writes through the in-memory mobility store; subsequent `hydrate_from_stores` rebuilds an identical mobility world.
- Hydration with empty mobility store falls back to `initial_world()` and reports `tick == 0`.

**Integration (opt-in, `ABUTOWN_TEST_DATABASE_URL`):**

- `postgres_mobility_state_survives_runtime_restart`: mutate via `tick_mobility` for N ticks → persist → drop runtime → rebuild → state identical (compare via DTO equality to handle private fields).

**Regression:** all existing mobility unit tests and all chunk-recovery tests still green.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Serde derive on `MobilityWorld` breaks JSON wire compatibility for `MobilitySnapshotDto` | DTO is a separate type; Serde on `MobilityWorld` is purely for the new `payload` JSONB column. No protocol change. |
| 20 agents × tick rate × 5s snapshot interval = large JSON writes | Single row UPSERT, < 50 KB JSON for this seed size. Acceptable. If agent count grows 100×, revisit throttling. |
| Initial seeder coupled to seeded chunk coords | Seeder lives in sim-core and references coord literals. If `SEEDED_CHUNKS` ever changes in sim-server, the seeder may produce routes that traverse non-loaded chunks. Acceptable for this slice; documented in a comment on the seeder. |
| Determinism after restart conflicts with new agent spawning later | This slice does no spawning beyond initial. The future "player spawns vehicle" plan must introduce its own determinism story. Documented as out of scope. |
| Tick column denormalization can drift from `payload.tick` | The hydration path reads `payload.tick`, not the column. Column is for observability / indexes only. Documented. |

## File Structure

- New: `backend/crates/sim-core/src/mobility/seed.rs`
- Modified: `backend/crates/sim-core/src/mobility.rs` — split into module (mobility/mod.rs re-exports), add Serde derives.
- Modified: `backend/crates/sim-core/src/persistence.rs` — add `MobilitySnapshotStore` trait + error + in-memory impl.
- New: `backend/crates/sim-server/src/postgres_mobility.rs` — Postgres adapter.
- New: `backend/crates/sim-server/migrations/202605160002_mobility_snapshots.sql`.
- Modified: `backend/crates/sim-server/src/runtime.rs` — new store field, new constructor, hydration update, `persist_mobility_snapshot`.
- Modified: `backend/crates/sim-server/src/app.rs` — construct Postgres mobility store; loop persists mobility.
- Modified: `backend/crates/sim-server/src/lib.rs` — export `postgres_mobility`.
- Modified: `backend/crates/sim-server/tests/http.rs` — recovery integration test.

## Open Questions Resolved During Brainstorming

- **Population mechanism:** procedural-from-seeder (not chunk-tile-driven, not player-driven).
- **Persistence model:** snapshot-only, no event log for mobility.
- **Snapshot frequency:** every snapshot-loop tick, no extra throttling.
- **Store granularity:** single row per world (no per-entity rows, no history table).
- **Recovery failure mode:** fail-fast on read/decode error.
- **Determinism source:** hardcoded seed in `initial_world()`, deterministic `tick_mobility`.
