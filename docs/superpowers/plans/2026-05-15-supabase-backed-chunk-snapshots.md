# Supabase-Backed Chunk Snapshots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the root `.env` Supabase/Postgres configuration first-class for the Rust server, then persist authoritative chunk snapshots durably without touching the already-merged card-hand/auth work.

**Architecture:** Introduce a small `ServerConfig` boundary in `sim-server` that loads root `.env` values and maps `DATABASE_URL` to the existing Postgres runtime. Add an async chunk snapshot store contract beside the current in-memory snapshot store, then add a Postgres adapter and wire the snapshot loop through it. Keep startup hydration/replay as the next slice after durable writes are verified.

**Tech Stack:** Rust 2024, Axum, Tokio, `dotenvy`, `async-trait`, `sqlx` Postgres/Supabase, existing `ChunkSnapshotDto`, existing `PostgresWorldEventStore`, existing `CardHandStore`.

---

## Current Main-Branch Context

New agent commits on `main` changed the backend baseline:

- `19f9dbf feat: add authenticated card hand`
  - Adds `/cards`, `/card-hand`, Supabase JWT validation, and `user_card_hands`.
- `ccbab1a fix: gate card hand behind login`
  - Frontend login gate for card hand.
- `f594d4d chore: ignore local env files`
  - Root `.env` is ignored.

Root `.env` currently contains these key names, values intentionally not printed:

```text
DATABASE_URL
SUPABASE_ANON_KEY
SUPABASE_JWKS_X
SUPABASE_JWKS_Y
SUPABASE_SERVICE_ROLE_KEY
SUPABASE_URL
```

Important mismatch: current backend code reads an old backend-specific database variable, while the root `.env` provides `DATABASE_URL`. The next backend slice must make `DATABASE_URL` the required production database key before adding more persistence. Missing `DATABASE_URL` or missing `SUPABASE_URL` is a startup error.

## Scope

This plan includes:

- root `.env` loading for the server binary,
- strict config from required `DATABASE_URL` and `SUPABASE_URL`,
- documentation of which Supabase keys the backend actually uses,
- async chunk snapshot store trait,
- in-memory implementation preserving existing tests,
- Postgres `chunk_snapshots` migration,
- Postgres chunk snapshot adapter,
- snapshot loop wiring that only clears dirty flags after successful writes,
- opt-in Postgres integration tests.

This plan does not include:

- changing CardHandStore semantics,
- using `SUPABASE_SERVICE_ROLE_KEY` from Rust,
- browser direct writes to Supabase,
- server startup hydration from snapshots,
- event replay/compaction,
- command idempotency,
- auth/permissions for `POST /commands`.

## File Structure

- Modify `backend/Cargo.toml`
  - Add `dotenvy` workspace dependency.
- Modify `backend/crates/sim-server/Cargo.toml`
  - Add `dotenvy.workspace = true`.
- Create `backend/crates/sim-server/src/config.rs`
  - Own `ServerConfig::from_env()`.
  - Resolve required `database_url` from `DATABASE_URL`.
  - Resolve `supabase_url` from `SUPABASE_URL`.
- Modify `backend/crates/sim-server/src/main.rs`
  - Load root `.env` via `dotenvy::dotenv().ok()`.
  - Use `ServerConfig` and `build_app_from_config`.
- Modify `backend/crates/sim-server/src/app.rs`
  - Replace ad hoc env reads with `build_app_from_config`.
  - Keep card-hand/auth construction equivalent to current behavior.
- Modify `backend/crates/sim-core/src/persistence.rs`
  - Add async `ChunkSnapshotStore` trait and `ChunkSnapshotStoreError`.
  - Keep existing in-memory helpers.
- Modify `backend/crates/sim-server/src/chunk_registry.rs`
  - Split snapshot collection from dirty clearing.
- Modify `backend/crates/sim-server/src/runtime.rs`
  - Own `Box<dyn ChunkSnapshotStore + Send>`.
  - Make `persist_chunk_snapshots()` async and fallible.
- Create `backend/crates/sim-server/migrations/202605150003_chunk_snapshots.sql`
  - Store latest durable snapshot per world/chunk.
- Create `backend/crates/sim-server/src/postgres_snapshots.rs`
  - Add `PostgresChunkSnapshotStore`.
- Modify `backend/crates/sim-server/src/lib.rs`
  - Export `config` and `postgres_snapshots`.
- Modify `backend/crates/sim-server/tests/http.rs`
  - Add config and snapshot-loop regression tests where practical.
- Modify `backend/README.md`
  - Document `.env`, Supabase/Postgres config, and snapshot persistence status.

---

### Task 1: Add Server Config For Root `.env`

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/crates/sim-server/Cargo.toml`
- Create: `backend/crates/sim-server/src/config.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`
- Modify: `backend/crates/sim-server/src/main.rs`
- Modify: `backend/crates/sim-server/src/app.rs`

- [x] **Step 1: Add dependencies**

Add to `backend/Cargo.toml` under `[workspace.dependencies]`:

```toml
dotenvy = "0.15"
```

Add to `backend/crates/sim-server/Cargo.toml` under `[dependencies]`:

```toml
dotenvy.workspace = true
```

- [x] **Step 2: Add config tests**

Create `backend/crates/sim-server/src/config.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_required_supabase_database_values() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
        ])
        .unwrap();

        assert_eq!(config.database_url, "postgres://primary");
        assert_eq!(config.supabase_url, "https://project.supabase.co");
    }

    #[test]
    fn config_rejects_missing_database_url() {
        let error = ServerConfig::from_pairs([("SUPABASE_URL", "https://project.supabase.co")])
            .unwrap_err();

        assert_eq!(error, ServerConfigError::MissingDatabaseUrl);
    }

    #[test]
    fn config_rejects_missing_supabase_url() {
        let error = ServerConfig::from_pairs([("DATABASE_URL", "postgres://primary")]).unwrap_err();

        assert_eq!(error, ServerConfigError::MissingSupabaseUrl);
    }
}
```

- [x] **Step 3: Implement config**

Add above the tests:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub database_url: String,
    pub supabase_url: String,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, ServerConfigError> {
        Self::from_pairs(std::env::vars())
    }

    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, ServerConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let mut database_url = None;
        let mut supabase_url = None;

        for (key, value) in pairs {
            match key.as_ref() {
                "DATABASE_URL" => database_url = Some(value.into()),
                "SUPABASE_URL" => supabase_url = Some(value.into()),
                _ => {}
            }
        }

        Ok(Self {
            database_url: database_url.ok_or(ServerConfigError::MissingDatabaseUrl)?,
            supabase_url: supabase_url.ok_or(ServerConfigError::MissingSupabaseUrl)?,
        })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServerConfigError {
    #[error("DATABASE_URL is required")]
    MissingDatabaseUrl,
    #[error("SUPABASE_URL is required")]
    MissingSupabaseUrl,
}
```

- [x] **Step 4: Wire config**

In `backend/crates/sim-server/src/lib.rs` add:

```rust
pub mod config;
```

In `backend/crates/sim-server/src/main.rs`, load root `.env` before config:

```rust
let _ = dotenvy::dotenv();
let config = sim_server::config::ServerConfig::from_env().context("load server config")?;
```

Change app construction to:

```rust
axum::serve(listener, build_app_from_config(&config).await?)
```

In `backend/crates/sim-server/src/app.rs`, add:

```rust
use crate::config::ServerConfig;
```

Replace `build_app_from_env()` internals with:

```rust
pub async fn build_app_from_env() -> anyhow::Result<Router> {
    let _ = dotenvy::dotenv();
    let config = ServerConfig::from_env()?;
    build_app_from_config(&config).await
}

pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let event_store = PostgresWorldEventStore::connect(&config.database_url).await?;
    let card_hands = CardHandStore::postgres(&config.database_url).await?;
    let auth = AuthVerifier::supabase(&config.supabase_url).await;

    Ok(build_app_with_runtime_and_card_hands(
        SimulationRuntime::new_with_event_store(Box::new(event_store)),
        card_hands,
        auth,
    ))
}
```

- [x] **Step 5: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server config
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server card_hand
```

Expected: both pass.

Commit:

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/sim-server/Cargo.toml backend/crates/sim-server/src/config.rs backend/crates/sim-server/src/lib.rs backend/crates/sim-server/src/main.rs backend/crates/sim-server/src/app.rs
git commit -m "feat: load backend supabase config from env"
```

---

### Task 2: Add Async Chunk Snapshot Store Contract

**Files:**
- Modify: `backend/crates/sim-core/src/persistence.rs`

- [x] **Step 1: Add failing tests**

Add tests covering async in-memory write/read and typed failure shape:

```rust
#[tokio::test]
async fn chunk_snapshot_store_writes_and_reads_snapshot() {
    let mut store = InMemoryChunkSnapshotStore::default();
    let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    chunk.set_tile_kind(0, TileKind::Road).expect("tile exists");
    let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

    ChunkSnapshotStore::write_snapshot(&mut store, snapshot.clone()).await.unwrap();

    let stored = ChunkSnapshotStore::read_snapshot(&store, ChunkCoord { x: 4, y: 4 })
        .await
        .unwrap()
        .expect("snapshot exists");
    assert_eq!(stored, snapshot);
}
```

- [x] **Step 2: Implement trait**

Add:

```rust
use async_trait::async_trait;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ChunkSnapshotStoreError {
    message: String,
}

impl ChunkSnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

#[async_trait]
pub trait ChunkSnapshotStore: std::fmt::Debug + Send {
    async fn write_snapshot(&mut self, snapshot: ChunkSnapshotDto) -> Result<(), ChunkSnapshotStoreError>;
    async fn read_snapshot(&self, coord: ChunkCoord) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError>;
}
```

Implement it for `InMemoryChunkSnapshotStore`, cloning snapshots on reads.

- [x] **Step 3: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core persistence
```

Commit:

```bash
git add backend/crates/sim-core/src/persistence.rs
git commit -m "feat: add chunk snapshot store contract"
```

---

### Task 3: Split Snapshot Collection From Dirty Clearing

**Files:**
- Modify: `backend/crates/sim-server/src/chunk_registry.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/src/app.rs`

- [x] **Step 1: Add tests**

Add a registry test proving snapshot collection does not clear dirty flags until explicitly marked persisted.

Expected behavior:

- `collect_snapshots()` returns three snapshots for seeded chunks.
- A second collection before clearing still contains dirty tiles.
- `mark_snapshots_persisted(&coords)` clears dirty tiles only for successful coords.

- [x] **Step 2: Implement registry split**

Add methods shaped like:

```rust
pub(crate) fn collect_snapshots(&self, world_id: &WorldId) -> Vec<ChunkSnapshotDto>;
pub(crate) fn mark_snapshots_persisted(&mut self, coords: &[ChunkCoord]);
```

Do not hold a mutable chunk borrow across an async `.await`.

- [x] **Step 3: Update runtime snapshot persistence**

Change runtime snapshot persistence to:

```rust
pub async fn persist_chunk_snapshots(&mut self) -> Result<usize, ChunkSnapshotStoreError> {
    let snapshots = self.registry.collect_snapshots(&self.world_id);
    let persisted_coords: Vec<ChunkCoord> = snapshots
        .iter()
        .map(|snapshot| ChunkCoord { x: snapshot.coord.x, y: snapshot.coord.y })
        .collect();

    for snapshot in snapshots {
        self.snapshot_store.write_snapshot(snapshot).await?;
    }

    self.registry.mark_snapshots_persisted(&persisted_coords);
    Ok(persisted_coords.len())
}
```

- [x] **Step 4: Update app helper**

Change `persist_snapshots_once()` to return `Result<usize, ChunkSnapshotStoreError>` and log snapshot-loop failures instead of panicking or clearing dirty state.

- [x] **Step 5: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server chunk_registry
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server persist_snapshots_once
```

Commit:

```bash
git add backend/crates/sim-server/src/chunk_registry.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs
git commit -m "feat: make snapshot persistence fallible"
```

---

### Task 4: Add Postgres Chunk Snapshot Store

**Files:**
- Create: `backend/crates/sim-server/migrations/202605150003_chunk_snapshots.sql`
- Create: `backend/crates/sim-server/src/postgres_snapshots.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`

- [ ] **Step 1: Add migration**

Create:

```sql
CREATE TABLE IF NOT EXISTS chunk_snapshots (
    world_id TEXT NOT NULL,
    chunk_x INTEGER NOT NULL,
    chunk_y INTEGER NOT NULL,
    chunk_state TEXT NOT NULL,
    chunk_version BIGINT NOT NULL CHECK (chunk_version >= 0),
    tile_count INTEGER NOT NULL CHECK (tile_count >= 0),
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (world_id, chunk_x, chunk_y)
);

CREATE INDEX IF NOT EXISTS chunk_snapshots_world_updated_idx
    ON chunk_snapshots (world_id, updated_at DESC);
```

- [ ] **Step 2: Add adapter tests**

Unit-test serialization:

- `SqlChunkSnapshotRecord::from_snapshot()` extracts `world_id`, `chunk_x`, `chunk_y`, `chunk_state`, `chunk_version`, `tile_count`, and full JSON payload.

Add opt-in integration test using `ABUTOWN_TEST_DATABASE_URL`.

- [ ] **Step 3: Implement adapter**

Create `PostgresChunkSnapshotStore` with:

```rust
pub async fn connect(database_url: &str) -> Result<Self, ChunkSnapshotStoreError>;
```

Trait behavior:

- `write_snapshot`: upsert latest row by `(world_id, chunk_x, chunk_y)`.
- `read_snapshot`: select payload by coord for `abutown-main` initially, matching current runtime single-world scope.

- [ ] **Step 4: Export module**

Add in `backend/crates/sim-server/src/lib.rs`:

```rust
pub mod postgres_snapshots;
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server postgres_snapshots
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Commit:

```bash
git add backend/crates/sim-server/migrations/202605150003_chunk_snapshots.sql backend/crates/sim-server/src/postgres_snapshots.rs backend/crates/sim-server/src/lib.rs
git commit -m "feat: add postgres chunk snapshot store"
```

---

### Task 5: Wire Snapshot Store From Supabase Config

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/README.md`

- [ ] **Step 1: Add runtime constructor**

Add:

```rust
pub fn new_with_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
) -> Self
```

Keep `new_with_event_store()` for tests by delegating to `new_with_stores(event_store, Box::new(InMemoryChunkSnapshotStore::default()))`.

- [ ] **Step 2: Wire config**

In `build_app_from_config`, create these from required config:

```rust
let event_store = PostgresWorldEventStore::connect(&config.database_url).await?;
let snapshot_store = PostgresChunkSnapshotStore::connect(&config.database_url).await?;
let card_hands = CardHandStore::postgres(&config.database_url).await?;
```

Then:

```rust
SimulationRuntime::new_with_stores(Box::new(event_store), Box::new(snapshot_store))
```

- [ ] **Step 3: Document key usage**

Update `backend/README.md`:

- `DATABASE_URL`: required root `.env` key for SQLx Postgres/Supabase.
- `SUPABASE_URL`: used for JWT/JWKS auth.
- `SUPABASE_ANON_KEY`: frontend login/client key, not used by Rust persistence.
- `SUPABASE_SERVICE_ROLE_KEY`: intentionally not used by Rust in this slice.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Commit:

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs backend/README.md
git commit -m "feat: persist snapshots through configured database"
```

---

## Next Slice After This Plan

After durable snapshot writes are green, create a separate recovery plan:

- read latest chunk snapshots during startup,
- hydrate seeded loaded chunks from durable snapshot payloads,
- choose replay behavior for events newer than the latest snapshot,
- define command idempotency before accepting real user mutation retries.

That recovery slice should not be bundled here because it changes startup truth semantics.

## Self-Review

- Spec coverage: This plan advances the architecture requirement for Supabase/Postgres durable chunk snapshots and keeps event history already implemented.
- Duplicate check: It does not redo `world_events` or `card_hand`; it only normalizes config and adds `chunk_snapshots`.
- Config check: It accounts for the actual root `.env` key `DATABASE_URL` and deliberately fails startup when required production config is missing.
- Secret safety: No `.env` values are stored, printed, committed, or copied into docs.
