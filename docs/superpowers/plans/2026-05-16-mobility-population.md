# Mobility Population & Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Spawn a small fixed set of mobility entities at server start so the world is visibly alive, persist the full `MobilityWorld` snapshot to Postgres, and restore it on restart so `tick_mobility` resumes exactly where it left off.

**Architecture:** Per-world full-state snapshot table (`mobility_snapshots`, single row per `world_id`), UPSERT-style. No mobility event log — `tick_mobility` is already deterministic, so snapshot + tick is sufficient. A deterministic in-process seeder (`mobility::seed::initial_world()`) populates fresh worlds. Persistence rides on the existing snapshot loop.

**Tech Stack:** Rust 2024, Tokio, `async_trait`, `sqlx` (Postgres), Serde (already a workspace dependency).

**Spec:** `docs/superpowers/specs/2026-05-16-mobility-population-design.md`

**Deviation from spec:** The spec listed a module split (`mobility.rs` → `mobility/mod.rs` + `mobility/seed.rs`). This plan keeps `mobility.rs` as a single file and places the seeder in an inline `pub mod seed { ... }` submodule at the bottom. Rationale: avoids cross-file import churn for ~70 lines of seed code. The spec's intent (clear seeder boundary, isolated logic) is preserved.

---

## File Structure

- Modify: `backend/crates/sim-core/src/mobility.rs`
  - Add `#[derive(Serialize, Deserialize)]` to the 7 record/enum types.
  - Add inline `pub mod seed` with `initial_world()`.
- Modify: `backend/crates/sim-core/src/persistence.rs`
  - Add `MobilitySnapshotStore` trait, `MobilitySnapshotStoreError`, and `InMemoryMobilitySnapshotStore`.
- Create: `backend/crates/sim-server/migrations/202605160002_mobility_snapshots.sql`
  - Table for per-world mobility snapshot.
- Create: `backend/crates/sim-server/src/postgres_mobility.rs`
  - `PostgresMobilitySnapshotStore` adapter.
- Modify: `backend/crates/sim-server/src/lib.rs`
  - `pub mod postgres_mobility;`
- Modify: `backend/crates/sim-server/src/runtime.rs`
  - New `mobility_snapshot_store` field on `SimulationRuntime`.
  - New `new_with_all_stores(event, snapshot, mobility) -> Self` constructor.
  - Existing `new()` / `new_with_event_store` / `new_with_stores` keep their signatures and inject an in-memory mobility store internally.
  - `hydrate_from_stores` signature gains a third `mobility_snapshot_store` parameter.
  - `HydrationError::Mobility(MobilitySnapshotStoreError)` variant.
  - `persist_mobility_snapshot` async method.
- Modify: `backend/crates/sim-server/src/app.rs`
  - Construct `PostgresMobilitySnapshotStore`, pass to `hydrate_from_stores`.
  - Snapshot loop calls `persist_mobility_snapshot`.
- Modify: `backend/crates/sim-server/tests/http.rs`
  - One opt-in Postgres integration test for mobility recovery.

---

## Task 1: Serde Derives On Mobility Types

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [ ] **Step 1: Add Serde imports at the top of mobility.rs**

Add right after the existing `use` block (around line 5–8):

```rust
use serde::{Deserialize, Serialize};
```

- [ ] **Step 2: Add the failing roundtrip test**

Append inside the existing `#[cfg(test)] mod tests` block in `mobility.rs`:

```rust
#[test]
fn mobility_world_serde_round_trip_preserves_state() {
    let original = sample_world();
    let json = serde_json::to_value(&original).expect("serialize");
    let restored: MobilityWorld = serde_json::from_value(json).expect("deserialize");
    assert_eq!(restored, original);
}
```

(`sample_world()` already exists in the test module — it's the helper that builds the fixture.)

Add `MobilityWorld` to the existing `#[derive(...)]` so it implements `PartialEq` — check first whether it already does. If `MobilityWorld` lacks `PartialEq`, add it: `#[derive(Debug, Default, PartialEq)]`.

- [ ] **Step 3: Run the test to confirm it fails**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility_world_serde_round_trip
```

Expected: FAIL — no `Serialize`/`Deserialize` derive on the types.

- [ ] **Step 4: Add Serde derives to all 7 types**

Locate each derive line and add `Serialize, Deserialize`:

- `AgentMobilityState` enum: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`
- `PlanStage` enum: `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`
- `AgentRecord`: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`
- `VehicleRecord`: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`
- `StopRecord`: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`
- `RouteRecord`: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`
- `MobilityWorld`: `#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]`

(If any type already has `Eq`, keep it.)

`AgentId`, `VehicleId`, `StopId`, `RouteId`, `LinkId` are defined in `sim_core::ids`. Verify they already have Serde derives by running the test — if compilation fails because an inner type is not serializable, add the derive there too.

- [ ] **Step 5: Run the test to confirm it passes**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility_world_serde_round_trip
```

Expected: PASS.

- [ ] **Step 6: Run the rest of the workspace to check nothing broke**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs backend/crates/sim-core/src/ids.rs
git commit -m "feat: serde-derive mobility records for persistence"
```

(If `ids.rs` was not touched, omit it from the add list.)

---

## Task 2: Deterministic Initial Seeder

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [ ] **Step 1: Add the failing test**

Append to the test module in `mobility.rs`:

```rust
#[test]
fn initial_world_seeds_expected_population() {
    let world = seed::initial_world();

    assert_eq!(world.tick(), 0);

    let snapshot = world.snapshot();
    assert_eq!(snapshot.routes_count_for_test(), 2, "expected 2 routes");
    assert_eq!(snapshot.stops.len(), 4, "expected 4 stops");
    assert_eq!(snapshot.vehicles.len(), 4, "expected 4 vehicles");
    assert_eq!(snapshot.agents.len(), 20, "expected 20 agents");

    for agent in &snapshot.agents {
        assert!(!agent.plan.is_empty(), "every agent must have at least one plan stage");
    }
    for vehicle in &snapshot.vehicles {
        assert!(vehicle.capacity > 0, "vehicle capacity must be positive");
    }
}

#[test]
fn initial_world_is_deterministic() {
    let a = seed::initial_world();
    let b = seed::initial_world();
    assert_eq!(a, b, "initial_world() must be deterministic across calls");
}
```

Notes:
- `MobilitySnapshot` (returned by `world.snapshot()`) does not currently expose `routes` because the existing `MobilitySnapshot` struct holds only agents/vehicles/stops. For the route count assertion, add a separate `world.route_count_for_test()` accessor under `#[cfg(test)]` on `MobilityWorld`, or read `world.routes.len()` if the test is in the same module (it is — `mod tests` is inside `mobility.rs`, so private fields are reachable).

Replace `routes_count_for_test()` with `world.routes.len()` directly:

```rust
assert_eq!(world.routes.len(), 2, "expected 2 routes");
let snapshot = world.snapshot();
assert_eq!(snapshot.stops.len(), 4, "expected 4 stops");
// etc.
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core initial_world_seeds initial_world_is_deterministic
```

Expected: FAIL — `seed::initial_world` does not exist.

- [ ] **Step 3: Implement the seeder**

At the bottom of `mobility.rs` (above the `#[cfg(test)] mod tests` block), add:

```rust
pub mod seed {
    use std::collections::{HashMap, VecDeque};

    use super::{
        AgentMobilityState, AgentRecord, MobilityWorld, PlanStage, RouteRecord, StopRecord,
        VehicleRecord,
    };
    use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

    /// Build a deterministic populated mobility world for fresh server starts.
    ///
    /// Two routes traverse the seeded chunk neighbourhood; 4 vehicles and
    /// 20 agents are spawned with cyclic plans. Calling this function twice
    /// returns equal worlds.
    pub fn initial_world() -> MobilityWorld {
        let horizontal_route = RouteId("route:horizontal".to_string());
        let vertical_route = RouteId("route:vertical".to_string());
        let horizontal_link = LinkId("link:horizontal:main".to_string());
        let vertical_link = LinkId("link:vertical:main".to_string());

        let horizontal_pickup = StopId("stop:horizontal:pickup".to_string());
        let horizontal_dropoff = StopId("stop:horizontal:dropoff".to_string());
        let vertical_pickup = StopId("stop:vertical:pickup".to_string());
        let vertical_dropoff = StopId("stop:vertical:dropoff".to_string());

        let walk_link = LinkId("link:walk:default".to_string());
        let work_activity = "activity:work".to_string();

        let mut routes = HashMap::new();
        routes.insert(
            horizontal_route.clone(),
            RouteRecord {
                id: horizontal_route.clone(),
                links: vec![horizontal_link.clone()],
            },
        );
        routes.insert(
            vertical_route.clone(),
            RouteRecord {
                id: vertical_route.clone(),
                links: vec![vertical_link.clone()],
            },
        );

        let mut stops = HashMap::new();
        for (stop_id, route_id, progress) in [
            (&horizontal_pickup, &horizontal_route, 0.0_f32),
            (&horizontal_dropoff, &horizontal_route, 1.0_f32),
            (&vertical_pickup, &vertical_route, 0.0_f32),
            (&vertical_dropoff, &vertical_route, 1.0_f32),
        ] {
            stops.insert(
                stop_id.clone(),
                StopRecord {
                    id: stop_id.clone(),
                    route_id: route_id.clone(),
                    link_index: 0,
                    progress,
                    waiting_agents: VecDeque::new(),
                },
            );
        }

        let mut vehicles = HashMap::new();
        for offset in 0..4u32 {
            let route_id = if offset % 2 == 0 {
                horizontal_route.clone()
            } else {
                vertical_route.clone()
            };
            let vehicle_id = VehicleId(format!("vehicle:seed:{offset}"));
            vehicles.insert(
                vehicle_id.clone(),
                VehicleRecord {
                    id: vehicle_id,
                    route_id,
                    link_index: 0,
                    progress: (offset as f32) * 0.25,
                    speed_per_tick: 0.1,
                    capacity: 4,
                    occupants: Vec::new(),
                    dwell_ticks_remaining: 0,
                },
            );
        }

        let mut agents = HashMap::new();
        for offset in 0..20u32 {
            let agent_id = AgentId(format!("agent:seed:{offset}"));
            let (pickup, dropoff, route_id) = if offset % 2 == 0 {
                (&horizontal_pickup, &horizontal_dropoff, &horizontal_route)
            } else {
                (&vertical_pickup, &vertical_dropoff, &vertical_route)
            };

            agents.insert(
                agent_id.clone(),
                AgentRecord {
                    id: agent_id,
                    state: AgentMobilityState::Walking {
                        link_id: walk_link.clone(),
                        progress: (offset as f32) * 0.05,
                    },
                    plan: vec![
                        PlanStage::WalkToStop {
                            link_id: walk_link.clone(),
                            stop_id: pickup.clone(),
                        },
                        PlanStage::RideToStop {
                            route_id: route_id.clone(),
                            stop_id: dropoff.clone(),
                        },
                        PlanStage::WalkToActivity {
                            link_id: walk_link.clone(),
                            activity_id: work_activity.clone(),
                        },
                        PlanStage::Activity {
                            activity_id: work_activity.clone(),
                        },
                    ],
                    plan_cursor: 0,
                    walk_speed_per_tick: 0.5,
                },
            );
        }

        MobilityWorld {
            tick: 0,
            agents,
            vehicles,
            stops,
            routes,
        }
    }
}
```

If any field on `MobilityWorld` is not visible from this module path (it should be — `seed` is a child module so it has access to private fields), make the struct field accessible by using `pub(super)` or via a builder method. Concretely: if compilation fails, add a `pub(crate) fn from_fixture(...)` constructor on `MobilityWorld` and call it from `seed`. The likely outcome is that direct field access works because `seed` is a child of the same module.

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core initial_world_seeds initial_world_is_deterministic
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: deterministic initial mobility world seeder"
```

---

## Task 3: MobilitySnapshotStore Trait + InMemory Impl

**Files:**
- Modify: `backend/crates/sim-core/src/persistence.rs`

- [ ] **Step 1: Write failing tests**

Append to the test module in `persistence.rs`:

```rust
#[tokio::test]
async fn mobility_snapshot_store_writes_and_reads() {
    use crate::mobility::seed;

    let mut store = InMemoryMobilitySnapshotStore::default();
    let world = seed::initial_world();

    MobilitySnapshotStore::write(&mut store, "abutown-main", 42, &world)
        .await
        .unwrap();

    let (tick, restored) = MobilitySnapshotStore::read(&store, "abutown-main")
        .await
        .unwrap()
        .expect("snapshot exists");

    assert_eq!(tick, 42);
    assert_eq!(restored, world);
}

#[tokio::test]
async fn mobility_snapshot_store_read_returns_none_for_unknown_world() {
    let store = InMemoryMobilitySnapshotStore::default();
    let result = MobilitySnapshotStore::read(&store, "missing-world").await.unwrap();
    assert!(result.is_none());
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility_snapshot_store
```

Expected: FAIL — types do not exist.

- [ ] **Step 3: Implement trait, error, and in-memory store**

Add to `backend/crates/sim-core/src/persistence.rs`:

```rust
use std::collections::HashMap;

use crate::mobility::MobilityWorld;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct MobilitySnapshotStoreError {
    message: String,
}

impl MobilitySnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
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

#[derive(Debug, Default)]
pub struct InMemoryMobilitySnapshotStore {
    snapshots: HashMap<String, (u64, MobilityWorld)>,
}

#[async_trait]
impl MobilitySnapshotStore for InMemoryMobilitySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityWorld,
    ) -> Result<(), MobilitySnapshotStoreError> {
        self.snapshots.insert(world_id.to_string(), (tick, snapshot.clone()));
        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, MobilityWorld)>, MobilitySnapshotStoreError> {
        Ok(self.snapshots.get(world_id).cloned())
    }
}
```

`MobilityWorld` needs `Clone`. Look at the current derive line and add `Clone` if missing: `#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]`.

The existing `HashMap` import at the top of `persistence.rs` may already be present from chunk snapshots — verify and avoid duplicate imports.

- [ ] **Step 4: Run tests to confirm pass**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core mobility_snapshot_store persistence
```

Expected: all sim-core persistence tests pass.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/persistence.rs backend/crates/sim-core/src/mobility.rs
git commit -m "feat: in-memory mobility snapshot store"
```

(`mobility.rs` only if you had to add `Clone` to `MobilityWorld`.)

---

## Task 4: Postgres Migration For mobility_snapshots

**Files:**
- Create: `backend/crates/sim-server/migrations/202605160002_mobility_snapshots.sql`

- [ ] **Step 1: Write the migration**

Create the file with contents:

```sql
CREATE TABLE IF NOT EXISTS mobility_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Verify ordering**

```bash
ls backend/crates/sim-server/migrations/
```

Expected list (alphabetical = execution order):

```
202605150001_world_events.sql
202605150002_card_hand_core.sql
202605150003_chunk_snapshots.sql
202605160001_chunk_recovery.sql
202605160002_mobility_snapshots.sql
```

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/migrations/202605160002_mobility_snapshots.sql
git commit -m "feat: migrate mobility_snapshots table"
```

---

## Task 5: PostgresMobilitySnapshotStore Adapter

**Files:**
- Create: `backend/crates/sim-server/src/postgres_mobility.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`

- [ ] **Step 1: Add the module export**

In `backend/crates/sim-server/src/lib.rs`, after the existing `pub mod postgres_snapshots;` line, add:

```rust
pub mod postgres_mobility;
```

- [ ] **Step 2: Write the adapter with a unit-test stub**

Create `backend/crates/sim-server/src/postgres_mobility.rs`:

```rust
use async_trait::async_trait;
use serde_json::Value;
use sim_core::mobility::MobilityWorld;
use sim_core::persistence::{MobilitySnapshotStore, MobilitySnapshotStoreError};
use sqlx::{PgPool, postgres::PgPoolOptions};

const MOBILITY_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160002_mobility_snapshots.sql");

#[derive(Debug)]
pub struct PostgresMobilitySnapshotStore {
    pool: PgPool,
}

impl PostgresMobilitySnapshotStore {
    pub async fn connect(database_url: &str) -> Result<Self, MobilitySnapshotStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        for statement in MOBILITY_SNAPSHOTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }
}

#[async_trait]
impl MobilitySnapshotStore for PostgresMobilitySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityWorld,
    ) -> Result<(), MobilitySnapshotStoreError> {
        let tick_i64 = i64::try_from(tick)
            .map_err(|_| MobilitySnapshotStoreError::unavailable("tick exceeds i64"))?;
        let payload: Value = serde_json::to_value(snapshot)
            .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO mobility_snapshots (world_id, tick, payload)
            VALUES ($1, $2, $3)
            ON CONFLICT (world_id) DO UPDATE
              SET tick = EXCLUDED.tick,
                  payload = EXCLUDED.payload,
                  updated_at = now()
            "#,
        )
        .bind(world_id)
        .bind(tick_i64)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, MobilityWorld)>, MobilitySnapshotStoreError> {
        let row: Option<(i64, Value)> = sqlx::query_as(
            "SELECT tick, payload FROM mobility_snapshots WHERE world_id = $1",
        )
        .bind(world_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        match row {
            None => Ok(None),
            Some((tick, payload)) => {
                let world: MobilityWorld = serde_json::from_value(payload).map_err(|error| {
                    MobilitySnapshotStoreError::unavailable(error.to_string())
                })?;
                let tick = u64::try_from(tick)
                    .map_err(|_| MobilitySnapshotStoreError::unavailable("negative tick in row"))?;
                Ok(Some((tick, world)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_mobility_store_round_trip_when_database_url_is_set() {
        use sim_core::mobility::seed;

        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let mut store = PostgresMobilitySnapshotStore::connect(&database_url).await.unwrap();
        let world = seed::initial_world();
        let world_id = format!("test:mobility:{}", uuid::Uuid::now_v7());

        store.write(&world_id, 7, &world).await.unwrap();
        let (tick, restored) = store.read(&world_id).await.unwrap().expect("snapshot exists");

        assert_eq!(tick, 7);
        assert_eq!(restored, world);
    }
}
```

- [ ] **Step 3: Verify build and test**

```bash
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server postgres_mobility
```

Expected: builds; the opt-in test silently returns when env var is unset.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-server/src/postgres_mobility.rs backend/crates/sim-server/src/lib.rs
git commit -m "feat: postgres mobility snapshot adapter"
```

---

## Task 6: SimulationRuntime Wiring

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Extend imports**

Add at the top of `runtime.rs` (alongside the existing `use sim_core::...` block):

```rust
use sim_core::persistence::{
    ChunkSnapshotStore, InMemoryChunkSnapshotStore, MobilitySnapshotStore,
    InMemoryMobilitySnapshotStore, MobilitySnapshotStoreError,
};
```

Make sure `MobilitySnapshotStore`, `InMemoryMobilitySnapshotStore`, `MobilitySnapshotStoreError` come from the same `persistence` module (they do — Task 3 added them there). Drop duplicate `use` lines if needed.

- [ ] **Step 2: Add the field**

Locate the `pub struct SimulationRuntime { ... }` definition. Add a new field after `snapshot_store`:

```rust
mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
```

In the `Debug` impl, add `.field("mobility_snapshot_store", ...)` if other stores are listed there, or skip it (the existing impl uses `finish_non_exhaustive`).

- [ ] **Step 3: Add the new constructor**

Inside `impl SimulationRuntime`, add:

```rust
pub fn new_with_all_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
) -> Self {
    let mut runtime = Self::new_with_stores(event_store, snapshot_store);
    runtime.mobility_snapshot_store = mobility_snapshot_store;
    runtime
}
```

- [ ] **Step 4: Update existing constructors to inject the default**

Edit `new()`:

```rust
pub fn new() -> Self {
    Self::new_with_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
    )
}
```

(No change needed — it delegates.)

Edit `new_with_stores()` to also initialize the new field:

```rust
pub fn new_with_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
) -> Self {
    Self {
        world_id: Self::default_world_id(),
        registry: ChunkRegistry::new(CHUNK_SIZE),
        mobility: MobilityWorld::default(),
        snapshot_store,
        event_store,
        mobility_snapshot_store: Box::new(InMemoryMobilitySnapshotStore::default()),
        event_count: 0,
        tick: 0,
        version: 0,
    }
}
```

(Edit only the struct-init block — keep the seeded-chunks loop intact. Read the existing `new_with_stores` body and add the one new field assignment in the right place.)

- [ ] **Step 5: Add the persist method**

Inside `impl SimulationRuntime`, alongside `persist_chunk_snapshots`:

```rust
pub async fn persist_mobility_snapshot(&mut self) -> Result<(), MobilitySnapshotStoreError> {
    self.mobility_snapshot_store
        .write(&self.world_id.0, self.mobility.tick(), &self.mobility)
        .await
}
```

- [ ] **Step 6: Add a unit test for the round trip**

In the runtime test module, add:

```rust
#[tokio::test]
async fn runtime_persists_mobility_snapshot_and_reloads_through_store() {
    use sim_core::mobility::seed;
    use sim_core::persistence::InMemoryMobilitySnapshotStore;

    let store: Box<dyn MobilitySnapshotStore + Send> =
        Box::new(InMemoryMobilitySnapshotStore::default());
    let mut runtime = SimulationRuntime::new_with_all_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        store,
    );
    runtime.set_mobility_for_test(seed::initial_world());
    runtime.persist_mobility_snapshot().await.unwrap();

    let (tick, world) = runtime
        .mobility_snapshot_store
        .read(&runtime.world_id.0)
        .await
        .unwrap()
        .expect("snapshot exists");

    assert_eq!(tick, 0);
    assert_eq!(world, seed::initial_world());
}
```

The test calls a helper `set_mobility_for_test` that does not yet exist. Add it under `#[cfg(test)]` on `impl SimulationRuntime`:

```rust
#[cfg(test)]
pub fn set_mobility_for_test(&mut self, mobility: MobilityWorld) {
    self.mobility = mobility;
}
```

The test also reads `runtime.mobility_snapshot_store` and `runtime.world_id` directly — they're private fields. Both reads are within `mod tests`, which is inside `runtime.rs`, so private access works.

- [ ] **Step 7: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server runtime_persists_mobility_snapshot
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: green.

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs
git commit -m "feat: simulation runtime stores and persists mobility snapshot"
```

---

## Task 7: Hydrate Mobility From Store At Startup

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/src/app.rs` (compile-only change at first, see below)

- [ ] **Step 1: Add a failing test for hydration**

In the runtime test module:

```rust
#[tokio::test]
async fn hydrate_seeds_fresh_mobility_when_store_is_empty() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::mobility::seed;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, InMemoryMobilitySnapshotStore};

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
    )
    .await
    .unwrap();

    assert_eq!(runtime.mobility_snapshot(), seed::initial_world().snapshot_dto_for_test(&runtime.world_id));
}

#[tokio::test]
async fn hydrate_restores_mobility_from_store_when_present() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::mobility::seed;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, InMemoryMobilitySnapshotStore, MobilitySnapshotStore};

    let mut mobility_store = InMemoryMobilitySnapshotStore::default();
    let mut authored = seed::initial_world();
    // Advance one tick so the persisted state differs from a fresh seed.
    let _ = authored.tick_mobility();
    let persisted_tick = authored.tick();
    MobilitySnapshotStore::write(&mut mobility_store, "abutown-main", persisted_tick, &authored)
        .await
        .unwrap();

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(mobility_store),
    )
    .await
    .unwrap();

    assert_eq!(runtime.mobility_tick(), persisted_tick);
}
```

These reference test-only helpers `mobility_snapshot()` returns a `MobilitySnapshotDto` (already exists on the runtime). The second assertion needs a new public test helper `mobility_tick()`:

```rust
#[cfg(test)]
pub fn mobility_tick(&self) -> u64 {
    self.mobility.tick()
}
```

The first assertion compares two `MobilitySnapshotDto` values. The expression `seed::initial_world().snapshot_dto_for_test(&runtime.world_id)` requires a helper on `MobilityWorld`. Define it `#[cfg(test)]` in `mobility.rs`:

```rust
#[cfg(test)]
impl MobilityWorld {
    pub fn snapshot_dto_for_test(&self, world_id: &crate::ids::WorldId) -> abutown_protocol::MobilitySnapshotDto {
        crate::mobility::build_mobility_snapshot_dto(world_id, self.tick(), self.snapshot())
    }
}
```

(`build_mobility_snapshot_dto` already exists in `mobility.rs`.) `WorldId` is in `abutown_protocol` per existing usage — verify the import path; if `WorldId` is at `abutown_protocol::WorldId`, adjust accordingly.

Simpler alternative: drop the DTO comparison and just assert `runtime.mobility_tick() == 0` for the seed case. That keeps the test small without test helpers on `MobilityWorld`. Use this simpler version:

```rust
#[tokio::test]
async fn hydrate_seeds_fresh_mobility_when_store_is_empty() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, InMemoryMobilitySnapshotStore};

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
    )
    .await
    .unwrap();

    assert_eq!(runtime.mobility_tick(), 0);
    assert_eq!(runtime.mobility_agent_count_for_test(), 20);
    assert_eq!(runtime.mobility_vehicle_count_for_test(), 4);
}
```

Add the helpers:

```rust
#[cfg(test)]
impl SimulationRuntime {
    pub fn mobility_tick(&self) -> u64 { self.mobility.tick() }
    pub fn mobility_agent_count_for_test(&self) -> usize {
        self.mobility.snapshot().agents.len()
    }
    pub fn mobility_vehicle_count_for_test(&self) -> usize {
        self.mobility.snapshot().vehicles.len()
    }
}
```

- [ ] **Step 2: Run tests to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server hydrate_seeds_fresh_mobility hydrate_restores_mobility_from_store
```

Expected: FAIL — `hydrate_from_stores` only takes 2 arguments today.

- [ ] **Step 3: Extend `hydrate_from_stores` signature and body**

Locate the existing async fn:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
) -> Result<Self, HydrationError>
```

Change to:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
) -> Result<Self, HydrationError>
```

At the start of the body (after `let world_id = ...`), read the mobility store:

```rust
let mobility = match mobility_snapshot_store
    .read(&world_id.0)
    .await
    .map_err(HydrationError::Mobility)?
{
    Some((_tick, world)) => world,
    None => sim_core::mobility::seed::initial_world(),
};
```

In the `Ok(Self { ... })` final construction, replace the existing `mobility: MobilityWorld::default(),` with `mobility,` and add `mobility_snapshot_store,` at the right place.

Add the `HydrationError::Mobility` variant:

```rust
#[derive(Debug, thiserror::Error)]
pub enum HydrationError {
    #[error("snapshot store error: {0}")]
    Snapshot(sim_core::persistence::ChunkSnapshotStoreError),
    #[error("event store error: {0}")]
    Events(sim_core::events::WorldEventStoreError),
    #[error("snapshot decode error: {0}")]
    Decode(sim_core::chunk::SnapshotDecodeError),
    #[error("event apply error: {0}")]
    Apply(sim_core::chunk::EventApplyError),
    #[error("chunk error during seed: {0}")]
    Chunk(sim_core::chunk::ChunkError),
    #[error("mobility store error: {0}")]
    Mobility(MobilitySnapshotStoreError),
}
```

- [ ] **Step 4: Update the production caller in app.rs**

In `backend/crates/sim-server/src/app.rs`, the function `build_app_from_config` currently calls `hydrate_from_stores(event_store, snapshot_store)`. Update:

```rust
let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&config.database_url).await?;
let runtime = SimulationRuntime::hydrate_from_stores(
    Box::new(event_store),
    Box::new(snapshot_store),
    Box::new(mobility_snapshot_store),
)
.await?;
```

Import: add `use crate::postgres_mobility::PostgresMobilitySnapshotStore;` at the top of `app.rs`.

`MobilitySnapshotStoreError` propagates as `anyhow::Error` via `?` because it derives `thiserror::Error` (i.e., implements `std::error::Error`).

- [ ] **Step 5: Run tests**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server hydrate_seeds_fresh_mobility hydrate_restores_mobility_from_store
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: both new tests pass; full workspace green.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs backend/crates/sim-core/src/mobility.rs
git commit -m "feat: hydrate mobility world on runtime startup"
```

(`mobility.rs` only if you added test helpers there.)

---

## Task 8: Snapshot Loop Persists Mobility

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

- [ ] **Step 1: Inspect the existing snapshot loop**

```bash
grep -n "spawn_snapshot_loop\|persist_chunk_snapshots\|persist_snapshots_once" backend/crates/sim-server/src/app.rs
```

Identify the function that drives chunk snapshots inside the loop. It currently looks like (paraphrasing):

```rust
async fn persist_snapshots_once(state: Arc<...>) -> Result<usize, ...> {
    let mut guard = state.runtime.lock().await;
    guard.persist_chunk_snapshots().await
}
```

- [ ] **Step 2: Add mobility persistence after the chunk write**

Change the body so both kinds of snapshot are written in sequence:

```rust
async fn persist_snapshots_once(state: Arc<...>) -> Result<usize, ...> {
    let mut guard = state.runtime.lock().await;
    let written = guard.persist_chunk_snapshots().await?;
    guard.persist_mobility_snapshot().await
        .map_err(|err| /* convert to whatever this function returns */)?;
    Ok(written)
}
```

The error mapping: `persist_chunk_snapshots` returns `Result<usize, ChunkSnapshotStoreError>`. `persist_mobility_snapshot` returns `Result<(), MobilitySnapshotStoreError>`. The two error types are distinct. The simplest fix is to log the mobility error and continue (matching the existing tolerance for chunk-snapshot failures — chunk failures already don't crash the loop):

```rust
async fn persist_snapshots_once(state: Arc<...>) -> Result<usize, ChunkSnapshotStoreError> {
    let mut guard = state.runtime.lock().await;
    let written = guard.persist_chunk_snapshots().await?;
    if let Err(error) = guard.persist_mobility_snapshot().await {
        tracing::warn!(target = "mobility_snapshot", error = %error, "mobility snapshot write failed");
    }
    Ok(written)
}
```

If `tracing` is not in scope, use `eprintln!` instead. Search the existing file for `tracing::` or `eprintln!` usage and match it.

- [ ] **Step 3: Verify**

```bash
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: builds and all tests pass.

- [ ] **Step 4: Commit**

```bash
git add backend/crates/sim-server/src/app.rs
git commit -m "feat: snapshot loop persists mobility world"
```

---

## Task 9: Opt-In Postgres Recovery Integration Test

**Files:**
- Modify: `backend/crates/sim-server/tests/http.rs`

- [ ] **Step 1: Add the test**

Append to `backend/crates/sim-server/tests/http.rs`:

```rust
#[tokio::test]
async fn postgres_mobility_state_survives_runtime_restart() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::mobility::seed;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, MobilitySnapshotStore};
    use sim_server::postgres_mobility::PostgresMobilitySnapshotStore;
    use sim_server::runtime::SimulationRuntime;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!("skipping postgres_mobility_state_survives_runtime_restart; ABUTOWN_TEST_DATABASE_URL not set");
        return;
    };

    let world_id = format!("test:mobility:{}", uuid::Uuid::now_v7());

    // ---- First runtime: seed mobility, advance some ticks, persist, drop.
    let persisted_tick;
    let persisted_world;
    {
        let mobility_store = PostgresMobilitySnapshotStore::connect(&database_url).await.unwrap();
        let mut runtime = SimulationRuntime::new_with_all_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(mobility_store),
        );

        // Override world_id for test isolation.
        runtime.override_world_id_for_test(&world_id);
        runtime.set_mobility_for_test(seed::initial_world());

        // Advance the mobility simulation a few ticks.
        for _ in 0..5 {
            let _ = runtime.next_mobility_delta_for_test();
        }
        persisted_tick = runtime.mobility_tick();
        persisted_world = runtime.mobility_world_clone_for_test();
        runtime.persist_mobility_snapshot().await.unwrap();
    }

    // ---- Second runtime: connect store directly and verify the snapshot exists.
    let mut store = PostgresMobilitySnapshotStore::connect(&database_url).await.unwrap();
    let (tick, restored) = MobilitySnapshotStore::read(&store, &world_id)
        .await
        .unwrap()
        .expect("snapshot must be present after restart");

    assert_eq!(tick, persisted_tick);
    assert_eq!(restored, persisted_world);

    // Cleanup: best-effort delete so test rows don't accumulate.
    let _ = sqlx::query("DELETE FROM mobility_snapshots WHERE world_id = $1")
        .bind(&world_id)
        .execute(store.pool_for_test())
        .await;
}
```

This requires three new public test helpers on `SimulationRuntime`:

- `override_world_id_for_test(&mut self, world_id: &str)` — sets `self.world_id = WorldId(world_id.to_string())`.
- `next_mobility_delta_for_test(&mut self) -> MobilityDeltaDto` — wraps `self.next_mobility_delta()` so integration tests in the `tests/` crate (which can't access private fields) can drive the simulation.
- `mobility_world_clone_for_test(&self) -> MobilityWorld` — clones the world for comparison.

And one helper on `PostgresMobilitySnapshotStore`:

- `pool_for_test(&self) -> &sqlx::PgPool` — returns a reference to the pool so the test cleanup query can run.

Add all four under `#[cfg(any(test, feature = "test-support"))]` blocks if a feature exists, OR plain `pub` blocks gated on `#[cfg(test)]` where appropriate. Since integration tests in `tests/` are NOT compiled with `cfg(test)` enabled inside library crates, use plain `pub fn` with a name suffix `_for_test` to signal intent (consistent with the chunk-recovery plan's pattern, which exposed `apply_client_command` as `pub`).

Place the helpers on `impl SimulationRuntime` in `runtime.rs`:

```rust
pub fn override_world_id_for_test(&mut self, world_id: &str) {
    self.world_id = WorldId(world_id.to_string());
}

pub fn next_mobility_delta_for_test(&mut self) -> abutown_protocol::MobilityDeltaDto {
    self.next_mobility_delta()
}

pub fn mobility_world_clone_for_test(&self) -> MobilityWorld {
    self.mobility.clone()
}
```

And on `PostgresMobilitySnapshotStore` in `postgres_mobility.rs`:

```rust
pub fn pool_for_test(&self) -> &sqlx::PgPool {
    &self.pool
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: all pass; the postgres test silently skips when env var is unset.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/tests/http.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/postgres_mobility.rs
git commit -m "test: cover postgres mobility recovery end-to-end"
```

---

## Task 10: Final Quality Gate + progress.md

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Run formatter, full test suite, clippy**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: all three succeed.

If `cargo fmt --check` finds drift, run `cargo fmt --manifest-path backend/Cargo.toml --all` and stage the changed files.

- [ ] **Step 2: Append to progress.md**

Add one line at the end of `progress.md`:

```
2026-05-16T<HH:MM:SS>.000Z - Mobility population: deterministic initial seeder for routes/stops/vehicles/agents, Postgres mobility_snapshots table, full-state mobility persistence on the existing snapshot loop, and mobility hydration at server startup.
```

Use the current UTC timestamp.

- [ ] **Step 3: Commit**

```bash
git add progress.md
git commit -m "docs: record mobility population progress"
```

---

## Self-Review

- **Spec coverage:**
  - Goal (alive after restart) → Tasks 2, 6, 7, 8, 9.
  - Serde derives → Task 1.
  - Initial seeder → Task 2.
  - `MobilitySnapshotStore` trait + InMemory → Task 3.
  - Migration → Task 4.
  - Postgres adapter → Task 5.
  - Runtime field + constructor + persist method → Task 6.
  - `hydrate_from_stores` extension + `HydrationError::Mobility` → Task 7.
  - App wiring + snapshot loop → Tasks 7 + 8.
  - Integration test → Task 9.
  - Final gate + progress note → Task 10.
- **Placeholder scan:** All steps have concrete code or commands. No TBD/TODO/"similar to" references. The one judgment call ("if tracing is not in scope, use eprintln") gives the engineer a concrete fallback.
- **Type consistency:** `MobilitySnapshotStore::write(world_id, tick, snapshot: &MobilityWorld)` used consistently; `read` returns `Option<(u64, MobilityWorld)>` consistently. `hydrate_from_stores` 3-arg signature used in Tasks 7, 9. `new_with_all_stores` used in Tasks 6, 9. `HydrationError::Mobility` referenced only in Task 7. Test helpers (`mobility_tick`, `mobility_agent_count_for_test`, etc.) defined in Task 7 and used in Task 9.
- **Module layout deviation from spec** is documented at the top of the plan.
- **Spec out-of-scope items** (player commands, mobility events, dynamic routing) are not addressed by any task, matching the spec.
