# Economy Persistence 6b Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Durably persist + restore the economy across restart (mirroring the mobility-snapshot path end-to-end) and expose a backend-only `GET /economy` JSON debug view. Closes the economy roadmap.

**Architecture:** `EconomySnapshotStore` trait + `InMemoryEconomySnapshotStore` (sim-core) + `PostgresEconomySnapshotStore` + migration (sim-server). Register `EconomySnapshotProvider` in `PersistencePlugin`. Thread an `economy_snapshot_store` through `hydrate_from_stores` and `AppState::new_with_stores` exactly like the mobility store. Write via the existing provider→`PersistPayload`→persist-loop path; restore in `hydrate_from_stores`. `/economy` reads the live snapshot via an on-demand `Mutation::CollectEconomySnapshot`.

**Tech Stack:** Rust, sim-core + sim-server, axum, sqlx (runtime queries — NO offline mode, NO macro), `async_trait`. Cargo via `scripts/cargo-serial.sh`, `CARGO_TARGET_DIR=/tmp/abutown-persist6b-target`.

**Compile-driven discipline (IMPORTANT):** Tasks 2 changes the signatures of `hydrate_from_stores` and `new_with_stores`, breaking ~15 call sites (mostly tests). After each signature change, run `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server --all-targets` and fix **every** reported call site — the compiler enumerates them deterministically. Do not guess the list; let the build drive it. The enumerated sites below are the known ones.

**Confirmed grounding:**
- `EconomyPlugin` + `PersistencePlugin` are already installed in the runtime (seed path `runtime/mod.rs:170`, hydrate path `:289`). Economy resources exist & tick.
- 6a exports (via `pub use economy::persist::*`): `EconomyPersistSnapshot`, `extract_from_world`, `apply_into_world`, `EconomySnapshotProvider` — all under `sim_core::economy::`. NOTE the mobility `apply_into_world` is imported unqualified in `runtime/mod.rs`; ALWAYS fully-qualify `sim_core::economy::apply_into_world` to avoid the clash.
- Mobility templates to mirror: `MobilitySnapshotStore`/`MobilitySnapshotStoreError`/`InMemoryMobilitySnapshotStore` in `sim-core/src/persistence.rs`; `PostgresMobilitySnapshotStore` in `sim-server/src/postgres_mobility.rs`; migration `sim-server/migrations/202605160002_mobility_snapshots.sql` + `202605280002_…base_world_metadata.sql`.
- `SnapshotCompatibility::new(base_world_id: impl Into<String>, base_world_schema_version: u32)`.
- `persistence.rs` already imports `async_trait`, `WorldId`, `crate::ids::ChunkCoord`; add `use crate::economy::EconomyPersistSnapshot;`.
- `PersistPayload` (`runtime_view.rs:43`), `Mutation` enum (`:16`), `RuntimeReadView` (`:54` — NOT modified).
- `AppState` fields/accessors `app/mod.rs:72-202`; `new`/`new_with_card_hands`/`new_with_stores` `:88-118`; `build_app_from_config` `:339-372`; `CollectPersistData` handler `:854-905`; `persist_snapshots_once` `:1138-1240`; router `:417-435`; `proto_response` `:502`.
- `hydrate_from_stores` `runtime/mod.rs:248-393`.
- Ripple call sites: `hydrate_from_stores` — `app/mod.rs:358` + `runtime/tests.rs` (~833, 857, 1040, 1081, 1128, 1198, 1269, 1312). `new_with_stores` — `app/mod.rs:91,108,366` + `app/tests.rs:132,196,376,591`. `PersistPayload` literal — `app/mod.rs:897` + `app/tests.rs:293`; destructure `app/mod.rs:1164`.

---

### Task 1: `EconomySnapshotStore` trait + in-memory + Postgres + migration

**Files:**
- Modify: `backend/crates/sim-core/src/persistence.rs` (error + trait + in-memory)
- Modify: `backend/crates/sim-core/src/economy/persist.rs` (add `Default` to `EconomyPersistSnapshot`)
- Create: `backend/crates/sim-server/src/postgres_economy.rs`
- Create: `backend/crates/sim-server/migrations/202605300001_economy_snapshots.sql`
- Modify: `backend/crates/sim-server/src/lib.rs` (or wherever modules are declared — add `mod postgres_economy;` / `pub use`)

- [ ] **Step 1: Add `Default` to `EconomyPersistSnapshot`** in `economy/persist.rs` — change its derive to include `Default`:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomyPersistSnapshot {
```
(All fields are `Vec<…>` or `u64` → `Default` is valid.)

- [ ] **Step 2: Write the failing in-memory store test** — append to the `#[cfg(test)] mod tests` at the bottom of `sim-core/src/persistence.rs` (or add one if absent):

```rust
#[tokio::test]
async fn in_memory_economy_store_round_trips() {
    use crate::economy::EconomyPersistSnapshot;
    let mut store = InMemoryEconomySnapshotStore::default();
    let compat = SnapshotCompatibility::new("abutopia", 1);
    let mut snap = EconomyPersistSnapshot::default();
    snap.next_order_id = 99;

    store.write("w1", 7, &snap, &compat).await.unwrap();
    let got = store.read("w1", &compat).await.unwrap();
    assert_eq!(got, Some((7, snap.clone())));

    // Compatibility mismatch -> miss.
    let other = SnapshotCompatibility::new("abutopia", 2);
    assert_eq!(store.read("w1", &other).await.unwrap(), None);
}
```

(If `persistence.rs` has no `#[tokio::test]` infra, add `use super::*;`. `tokio` is already a dev-dep — mobility/runtime tests use `#[tokio::test]`.)

- [ ] **Step 3: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core in_memory_economy_store_round_trips`
Expected: FAIL to compile — `InMemoryEconomySnapshotStore` not found.

- [ ] **Step 4: Implement the error + trait + in-memory store** — add to `sim-core/src/persistence.rs` (after the mobility equivalents; add `use crate::economy::EconomyPersistSnapshot;` to the imports):

```rust
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct EconomySnapshotStoreError {
    message: String,
}

impl EconomySnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

#[async_trait]
pub trait EconomySnapshotStore: std::fmt::Debug + Send + Sync {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &EconomyPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), EconomySnapshotStoreError>;

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, EconomyPersistSnapshot)>, EconomySnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryEconomySnapshotStore {
    snapshots: HashMap<(String, SnapshotCompatibility), (u64, EconomyPersistSnapshot)>,
}

#[async_trait]
impl EconomySnapshotStore for InMemoryEconomySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &EconomyPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), EconomySnapshotStoreError> {
        self.snapshots.insert(
            (world_id.to_string(), compatibility.clone()),
            (tick, snapshot.clone()),
        );
        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, EconomyPersistSnapshot)>, EconomySnapshotStoreError> {
        Ok(self
            .snapshots
            .get(&(world_id.to_string(), compatibility.clone()))
            .cloned())
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core in_memory_economy_store_round_trips`
Expected: PASS.

- [ ] **Step 6: Create the migration** `backend/crates/sim-server/migrations/202605300001_economy_snapshots.sql`:

```sql
CREATE TABLE IF NOT EXISTS economy_snapshots (
    world_id TEXT PRIMARY KEY,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    base_world_id TEXT,
    base_world_schema_version INTEGER,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS economy_snapshots_base_world_idx
  ON economy_snapshots (world_id, base_world_id, base_world_schema_version)
```

(No trailing `;` after the final statement — `connect()` splits on `;` and skips empties, matching the mobility migration's handling.)

- [ ] **Step 7: Create `backend/crates/sim-server/src/postgres_economy.rs`** mirroring `postgres_mobility.rs`:

```rust
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use serde_json::Value;

use sim_core::economy::EconomyPersistSnapshot;
use sim_core::persistence::{
    EconomySnapshotStore, EconomySnapshotStoreError, SnapshotCompatibility,
};

const ECONOMY_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605300001_economy_snapshots.sql");

#[derive(Debug)]
pub struct PostgresEconomySnapshotStore {
    pool: PgPool,
}

impl PostgresEconomySnapshotStore {
    pub async fn connect(database_url: &str) -> Result<Self, EconomySnapshotStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| EconomySnapshotStoreError::unavailable(error.to_string()))?;

        for statement in ECONOMY_SNAPSHOTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| EconomySnapshotStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }
}

#[async_trait::async_trait]
impl EconomySnapshotStore for PostgresEconomySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &EconomyPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), EconomySnapshotStoreError> {
        let tick_i64 = i64::try_from(tick)
            .map_err(|_| EconomySnapshotStoreError::unavailable("tick exceeds i64"))?;
        let schema_version = i32::try_from(compatibility.base_world_schema_version)
            .map_err(|_| EconomySnapshotStoreError::unavailable("schema version exceeds i32"))?;
        let payload: Value = serde_json::to_value(snapshot)
            .map_err(|error| EconomySnapshotStoreError::unavailable(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO economy_snapshots (
                world_id, tick, base_world_id, base_world_schema_version, payload
            )
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (world_id) DO UPDATE
              SET tick = EXCLUDED.tick,
                  base_world_id = EXCLUDED.base_world_id,
                  base_world_schema_version = EXCLUDED.base_world_schema_version,
                  payload = EXCLUDED.payload,
                  updated_at = now()
            "#,
        )
        .bind(world_id)
        .bind(tick_i64)
        .bind(&compatibility.base_world_id)
        .bind(schema_version)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|error| EconomySnapshotStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, EconomyPersistSnapshot)>, EconomySnapshotStoreError> {
        let schema_version = i32::try_from(compatibility.base_world_schema_version)
            .map_err(|_| EconomySnapshotStoreError::unavailable("schema version exceeds i32"))?;
        let row: Option<(i64, Value)> = sqlx::query_as(
            r#"
            SELECT tick, payload
            FROM economy_snapshots
            WHERE world_id = $1 AND base_world_id = $2 AND base_world_schema_version = $3
            "#,
        )
        .bind(world_id)
        .bind(&compatibility.base_world_id)
        .bind(schema_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| EconomySnapshotStoreError::unavailable(error.to_string()))?;

        match row {
            None => Ok(None),
            Some((tick, payload)) => {
                let snap: EconomyPersistSnapshot = serde_json::from_value(payload)
                    .map_err(|error| EconomySnapshotStoreError::unavailable(error.to_string()))?;
                let tick = u64::try_from(tick)
                    .map_err(|_| EconomySnapshotStoreError::unavailable("negative tick"))?;
                Ok(Some((tick, snap)))
            }
        }
    }
}
```

(Confirm `compatibility.base_world_id` / `.base_world_schema_version` field names match `postgres_mobility.rs` usage — they do. Match the exact `use` style of `postgres_mobility.rs` for `PgPool`/`PgPoolOptions`/`Value`/`async_trait`.)

- [ ] **Step 8: Declare the module** — add `mod postgres_economy;` (and `pub use postgres_economy::PostgresEconomySnapshotStore;` if the sibling postgres modules are re-exported) next to the `mod postgres_mobility;` declaration (check `sim-server/src/lib.rs` or `main.rs`).

- [ ] **Step 9: Build + commit**

Run: `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server` → expect success.
```bash
git add backend/crates/sim-core/src/persistence.rs \
        backend/crates/sim-core/src/economy/persist.rs \
        backend/crates/sim-server/src/postgres_economy.rs \
        backend/crates/sim-server/migrations/202605300001_economy_snapshots.sql \
        backend/crates/sim-server/src/lib.rs
git commit -m "feat(persist): EconomySnapshotStore trait + in-memory + postgres + migration"
```

---

### Task 2: thread the economy store through hydration + AppState; register the provider

**Files:** `runtime/mod.rs`, `app/mod.rs`, `persistence_plugin.rs`, `runtime_view.rs`, plus all ripple call sites in `runtime/tests.rs` and `app/tests.rs`.

- [ ] **Step 1: Register the provider** in `sim-server/src/persistence_plugin.rs` `install()` — after the mobility provider push:

```rust
        providers.0.push(Box::new(
            sim_core::economy::EconomySnapshotProvider {
                world_id: self.world_id.clone(),
            },
        ));
```

- [ ] **Step 2: `hydrate_from_stores`** in `runtime/mod.rs` — add the 4th store param + return element + the restore branch:
  - Signature: add `economy_snapshot_store: Box<dyn EconomySnapshotStore + Send + Sync>,` after `mobility_snapshot_store`. Add `Box<dyn EconomySnapshotStore + Send + Sync>` as the 4th element of the returned tuple type.
  - Add `use sim_core::persistence::EconomySnapshotStore;` (and `EconomySnapshotStoreError` for the error variant) to the imports.
  - After `apply_into_world(&mut world, mobility_snap);` insert:

```rust
        if let Some((_tick, econ_snap)) = economy_snapshot_store
            .read(&world_id.0, &snapshot_compatibility)
            .await
            .map_err(HydrationError::Economy)?
        {
            sim_core::economy::apply_into_world(&mut world, &econ_snap);
        }
```
  - At the function's return tuple, add `economy_snapshot_store` as the 4th element (it was only `.read()` — `&self` — so it is still owned and returnable).

- [ ] **Step 3: Add `HydrationError::Economy`** — find the `HydrationError` enum in `runtime/mod.rs`, add a variant mirroring `Mobility`:

```rust
    #[error("economy snapshot store: {0}")]
    Economy(sim_core::persistence::EconomySnapshotStoreError),
```
(Match the exact attribute/`#[from]` style of the `Mobility` variant; if `Mobility` uses `#[from]`, mirror it.)

- [ ] **Step 4: Add `SimulationRuntime::economy_snapshot`** — in the `impl SimulationRuntime` block (near `mobility_persist_snapshot`):

```rust
    /// Live economy snapshot for the debug endpoint.
    pub fn economy_snapshot(&self) -> sim_core::economy::EconomyPersistSnapshot {
        sim_core::economy::extract_from_world(&self.world)
    }
```
(`self.world` is accessible inside the impl. If the field is named differently, match it.)

- [ ] **Step 5: `AppState`** in `app/mod.rs`:
  - Add field: `economy_snapshot_store: Arc<Mutex<Box<dyn EconomySnapshotStore + Send + Sync>>>,` (after `mobility_snapshot_store`).
  - Add `use sim_core::persistence::{EconomySnapshotStore, InMemoryEconomySnapshotStore};` (extend the existing persistence import).
  - Add accessor mirroring `mobility_snapshot_store()`:

```rust
    fn economy_snapshot_store(&self) -> Arc<Mutex<Box<dyn EconomySnapshotStore + Send + Sync>>> {
        Arc::clone(&self.economy_snapshot_store)
    }
```
  - `new_with_stores`: add param `economy_snapshot_store: Box<dyn EconomySnapshotStore + Send + Sync>,` (after `mobility_snapshot_store`); in the `Self { … }` literal add `economy_snapshot_store: Arc::new(Mutex::new(economy_snapshot_store)),`.
  - `new` (`:91`) and `new_with_card_hands` (`:108`) delegations: add `Box::new(InMemoryEconomySnapshotStore::default()),` after the mobility in-memory store arg.

- [ ] **Step 6: `build_app_from_config`** (`:339-372`):
  - After the `mobility_snapshot_store` is built: `let economy_snapshot_store = PostgresEconomySnapshotStore::connect(&config.database_url).await?;`
  - Pass `Box::new(economy_snapshot_store)` as the 4th store arg to `hydrate_from_stores`, and destructure the returned 4-tuple: `let (runtime, snapshot_store, mobility_snapshot_store, economy_snapshot_store) = SimulationRuntime::hydrate_from_stores(…).await?;`
  - Pass `economy_snapshot_store` to `AppState::new_with_stores(…)` (after `mobility_snapshot_store`).
  - Add `use crate::postgres_economy::PostgresEconomySnapshotStore;` (or fully-qualify).

- [ ] **Step 7: Build and fix ALL ripple call sites.**

Run: `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server --all-targets`
Fix every error (the compiler lists them). Known sites:
  - `runtime/tests.rs` `hydrate_from_stores` callers (~833, 857, 1040, 1081, 1128, 1198, 1269, 1312): add `Box::new(sim_core::persistence::InMemoryEconomySnapshotStore::default()),` as the 4th store arg; change `let (runtime, _, _) =` to `let (runtime, _, _, _) =` (add a 4th `_`). Add the import to the test module if needed.
  - `app/tests.rs` `new_with_stores` callers (132, 196, 376, 591): add `Box::new(InMemoryEconomySnapshotStore::default()),` after the mobility store arg (import it in the test module).

Repeat build until green. (No behavior change yet — economy store is wired but unused by the persist loop until Task 3; existing tests pass because the in-memory economy store is just stored.)

- [ ] **Step 8: Run the sim-server test suite** (regression after threading)

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS — all existing tests green with the new param threaded.

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-server/src/
git commit -m "feat(persist): thread economy snapshot store through hydration + AppState"
```

---

### Task 3: write the economy snapshot in the persist loop

**Files:** `runtime_view.rs`, `app/mod.rs`, test in `app/tests.rs`.

- [ ] **Step 1: Extend `PersistPayload`** (`runtime_view.rs`) — add fields + import:

```rust
use sim_core::economy::EconomyPersistSnapshot;
```
```rust
pub struct PersistPayload {
    pub chunk_snapshots: Vec<ChunkSnapshotDto>,
    pub world_id: WorldId,
    pub mobility_tick: u64,
    pub mobility_world: MobilityPersistSnapshot,
    pub economy_tick: u64,
    pub economy_world: EconomyPersistSnapshot,
}
```

- [ ] **Step 2: Collect handler** (`app/mod.rs` `CollectPersistData`, ~854-905):
  - Add `let mut economy_world: Option<sim_core::economy::EconomyPersistSnapshot> = None;` next to `mobility_world`.
  - Add an `"economy"` match arm (before the `other =>` arm):

```rust
                    "economy" => match serde_json::from_slice::<
                        sim_core::economy::EconomyPersistSnapshot,
                    >(&item.payload)
                    {
                        Ok(snap) => economy_world = Some(snap),
                        Err(error) => tracing::warn!(
                            %error,
                            kind = item.key.kind,
                            identifier = %item.key.identifier,
                            "provider emitted economy payload that failed to deserialize",
                        ),
                    },
```
  - In the `PersistPayload { … }` literal add:

```rust
                economy_tick: runtime.mobility_tick(),
                economy_world: economy_world.unwrap_or_default(),
```

- [ ] **Step 3: `persist_snapshots_once`** (`app/mod.rs`):
  - Extend the destructure (`:1164`) to bind `economy_tick` and `economy_world`.
  - After the mobility write block (before Phase 3) add:

```rust
    // Phase 2c: economy DB write — store-mutex only, no runtime lock held.
    {
        let econ_store = state.economy_snapshot_store();
        let mut econ_store = econ_store.lock().await;
        if let Err(error) = econ_store
            .write(&world_id.0, economy_tick, &economy_world, &compatibility)
            .await
        {
            tracing::warn!(%error, "failed to persist economy snapshot");
        }
    }
```

- [ ] **Step 4: Fix the `app/tests.rs:293` `PersistPayload` literal** — add `economy_tick: 0, economy_world: sim_core::economy::EconomyPersistSnapshot::default(),`.

- [ ] **Step 5: Write the persist-write integration test** — append to `app/tests.rs`:

```rust
#[tokio::test]
async fn persist_writes_economy_snapshot_to_store() {
    use sim_core::economy::{AccountBook, EconomicActorId, Money};
    use sim_core::persistence::{
        EconomySnapshotStore, InMemoryChunkSnapshotStore, InMemoryEconomySnapshotStore,
        InMemoryMobilitySnapshotStore, SnapshotCompatibility,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:econ-persist:1").await;
    // Seed an account so the economy snapshot is non-trivial.
    runtime
        .world
        .resource_mut::<AccountBook>()
        .accounts
        .insert(EconomicActorId(1), sim_core::economy::MoneyAccount { available: Money(500), locked: Money(0) });

    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let econ_store: Arc<Mutex<Box<dyn EconomySnapshotStore + Send + Sync>>> =
        Arc::new(Mutex::new(Box::new(InMemoryEconomySnapshotStore::default())));
    // Build AppState but inject our shared economy store so we can inspect it.
    // (If AppState exposes no injection hook, construct with new_with_stores and
    //  read back via a second store handle — simplest: pass the same Box.)
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    persist_snapshots_once(&state).await.unwrap();

    let store = state.economy_snapshot_store();
    let store = store.lock().await;
    let compat = SnapshotCompatibility::new(
        base_world.world_id().to_string(),
        base_world.snapshot_compatibility().base_world_schema_version,
    );
    let got = store.read(base_world.world_id(), &compat).await.unwrap();
    assert!(got.is_some(), "economy snapshot persisted");
    let (_tick, snap) = got.unwrap();
    assert!(
        snap.accounts.iter().any(|(a, _)| *a == EconomicActorId(1)),
        "seeded account present in persisted economy snapshot"
    );
}
```

(`runtime.world` may be `pub(crate)`; this test is in-crate so it is accessible — if not, use a public seeding helper. `state.economy_snapshot_store()` is a private accessor reachable from in-crate tests. Adjust if `mutate_runtime_tile`/`wait_for_tick_past`/`TICK_WAIT` helpers differ — they are used by neighboring tests in this file.)

- [ ] **Step 6: Run**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server persist_writes_economy_snapshot_to_store`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/runtime_view.rs backend/crates/sim-server/src/app/
git commit -m "feat(persist): write economy snapshot in the persist loop"
```

---

### Task 4: restore the economy snapshot on hydration (test)

The restore branch was added in Task 2 Step 2. This task proves it.

**Files:** test in `runtime/tests.rs`.

- [ ] **Step 1: Write the restore test** — append to `runtime/tests.rs`:

```rust
#[tokio::test]
async fn hydrate_restores_economy_snapshot() {
    use sim_core::economy::{EconomicActorId, EconomyPersistSnapshot, Money, MoneyAccount};
    use sim_core::persistence::{
        EconomySnapshotStore, InMemoryChunkSnapshotStore, InMemoryEconomySnapshotStore,
        InMemoryMobilitySnapshotStore,
    };

    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world for test");
    let compat = base_world.snapshot_compatibility();

    // Pre-load an economy store with a snapshot carrying one account.
    let mut snap = EconomyPersistSnapshot::default();
    snap.accounts.push((EconomicActorId(1), MoneyAccount { available: Money(777), locked: Money(0) }));
    let mut econ_store = InMemoryEconomySnapshotStore::default();
    econ_store
        .write(base_world.world_id(), 1, &snap, &compat)
        .await
        .unwrap();

    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(sim_core::persistence::InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(econ_store),
        &base_world,
    )
    .await
    .unwrap();

    let restored = runtime.economy_snapshot();
    assert_eq!(
        restored.accounts.iter().find(|(a, _)| *a == EconomicActorId(1)).map(|(_, acc)| acc.available),
        Some(Money(777)),
        "economy account restored from snapshot store"
    );
}

#[tokio::test]
async fn hydrate_with_empty_economy_store_yields_default_economy() {
    use sim_core::persistence::{
        InMemoryChunkSnapshotStore, InMemoryEconomySnapshotStore, InMemoryMobilitySnapshotStore,
    };
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world for test");
    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(sim_core::persistence::InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();
    assert!(runtime.economy_snapshot().accounts.is_empty());
}
```

(Use the real `InMemoryWorldEventStore` type/path used by neighboring `hydrate_from_stores` tests — match `event_store` construction in this file. If `WorldId(base_world.world_id())` is needed, follow the existing test idiom.)

- [ ] **Step 2: Run**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server hydrate_restores_economy_snapshot hydrate_with_empty_economy_store`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/src/runtime/tests.rs
git commit -m "test(persist): economy snapshot restored on hydration"
```

---

### Task 5: `GET /economy` debug view

**Files:** `runtime_view.rs`, `app/mod.rs`, test in `app/tests.rs`.

- [ ] **Step 1: Add the mutation** in `runtime_view.rs` `Mutation` enum:

```rust
    /// On-demand snapshot of the live economy for the debug endpoint.
    CollectEconomySnapshot {
        reply: oneshot::Sender<sim_core::economy::EconomyPersistSnapshot>,
    },
```

- [ ] **Step 2: Handle it** in the tick task's mutation match (`app/mod.rs`, alongside `CollectPersistData`):

```rust
        Mutation::CollectEconomySnapshot { reply } => {
            let _ = reply.send(runtime.economy_snapshot());
        }
```

- [ ] **Step 3: Add the route** in `build_router_from_state`:

```rust
        .route("/economy", get(economy))
```

- [ ] **Step 4: Add the handler** in `app/mod.rs`:

```rust
async fn economy(State(state): State<AppState>) -> Response {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if state
        .mutations
        .send(crate::runtime_view::Mutation::CollectEconomySnapshot { reply: tx })
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match rx.await {
        Ok(snap) => match serde_json::to_vec(&snap) {
            Ok(bytes) => (
                [(http::header::CONTENT_TYPE, "application/json")],
                bytes,
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
```

(`state.mutations` is accessible in-module. `Response`, `StatusCode`, `State`, `get`, `http::header` are already imported/used by neighboring handlers — reuse those imports.)

- [ ] **Step 5: Write the endpoint test** — append to `app/tests.rs`:

```rust
#[tokio::test]
async fn economy_endpoint_returns_json_snapshot() {
    use sim_core::economy::{AccountBook, EconomicActorId, EconomyPersistSnapshot, Money, MoneyAccount};

    let mut runtime = SimulationRuntime::new();
    runtime
        .world
        .resource_mut::<AccountBook>()
        .accounts
        .insert(EconomicActorId(5), MoneyAccount { available: Money(1234), locked: Money(0) });
    let state = AppState::new(runtime);
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    // Call the handler via the mutation round-trip.
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .mutations_for_test()
        .send(crate::runtime_view::Mutation::CollectEconomySnapshot { reply: tx })
        .unwrap();
    let snap = rx.await.unwrap();
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    assert!(decoded.accounts.iter().any(|(a, acc)| *a == EconomicActorId(5) && acc.available == Money(1234)));
}
```

(If `AppState` has no test accessor for `mutations`, either (a) add a `#[cfg(test)] pub(crate) fn mutations_for_test(&self) -> …`, or (b) drive the real HTTP handler via `axum::body`/`tower::ServiceExt::oneshot` like other endpoint tests in this file — prefer mirroring an existing endpoint test's invocation style if one exists. Pick the simplest that compiles; do NOT weaken the assertion.)

- [ ] **Step 6: Run**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server economy_endpoint_returns_json_snapshot`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/
git commit -m "feat(persist): GET /economy JSON debug view"
```

---

### Final gate (orchestrator runs; implementer reports readiness)

```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace --all-targets
```

All green. Implementer does NOT push or open a PR. Report per-task RED→GREEN + commit SHAs, the `-p sim-server` test summary, clippy/fmt status, and confirm: persist-write test, restore test, and `/economy` test all pass.

## Self-review notes

- **Spec coverage:** store trait + in-memory + postgres + migration ✓, provider registration ✓, threading through hydrate + AppState ✓, persist write ✓, restore ✓, debug view ✓.
- **Ripple:** every `hydrate_from_stores` (8 test + 1 prod) and `new_with_stores` (4 test + 3 prod-path) and `PersistPayload` (1 prod + 1 test) site enumerated; compile-driven safety net.
- **Name clash:** `sim_core::economy::apply_into_world` always fully-qualified (mobility's is unqualified in scope).
- **CI safety:** Postgres path compile-only (no DB in CI); all behavior tests use in-memory stores. Runtime sqlx, no offline data needed.
- **Backend-only:** `/economy` is JSON, no protocol/protobuf/frontend change → no browser smoke.
- **Default:** `EconomyPersistSnapshot` gains `Default` (all-Vec/u64 fields) for the empty-payload fallback + test literals.
