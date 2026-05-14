# Backend Persistence Snapshot Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the first durable-world boundary: the Rust runtime periodically writes authoritative chunk snapshots into an in-memory snapshot store and clears dirty chunk state after each successful snapshot pass.

**Architecture:** Keep hot simulation state in `SimulationRuntime` and `ChunkRegistry`; add a snapshot persistence pass that copies loaded chunk snapshots into `sim_core::persistence::InMemoryChunkSnapshotStore`. The server starts a separate snapshot loop next to the existing broadcast tick loop, so persistence remains outside websocket client handling and does not advance simulation time.

**Tech Stack:** Rust, Tokio, Axum app state, `sim-core::persistence::InMemoryChunkSnapshotStore`, existing `ChunkSnapshotDto` protocol.

---

## Scope

This plan implements an in-memory persistence boundary only:

- snapshot store query helpers,
- registry snapshot writing for all loaded chunks,
- dirty flag clearing after a successful write,
- runtime-owned snapshot store,
- server-side periodic snapshot loop,
- tests and docs.

It does not implement Supabase/Postgres, schema migrations, external database writes, recovery from process restart, chunk unload, dynamic chunk loading, or player-driven mutation APIs.

## File Structure

- Modify `backend/crates/sim-core/src/persistence.rs`
  - Adds `snapshot_count()` and `snapshot_coords()` helpers for verification and later adapters.
- Modify `backend/crates/sim-server/src/chunk_registry.rs`
  - Adds `write_snapshots()` to persist all loaded chunks into a store.
  - Clears dirty flags after each chunk snapshot is written.
- Modify `backend/crates/sim-server/src/runtime.rs`
  - Owns `InMemoryChunkSnapshotStore`.
  - Exposes `persist_chunk_snapshots()` and `stored_chunk_snapshot()` for app wiring and tests.
- Modify `backend/crates/sim-server/src/app.rs`
  - Starts a separate snapshot loop with a coarse interval.
  - Adds a small testable `persist_snapshots_once()` helper.
- Modify `backend/README.md`
  - Documents the in-memory snapshot loop boundary.

---

### Task 1: Add Snapshot Store Query Helpers

**Files:**
- Modify: `backend/crates/sim-core/src/persistence.rs`

- [ ] **Step 1: Write failing tests for snapshot store helpers**

Add this test after `snapshot_contains_only_dirty_tiles_then_clears_dirty_state` in `backend/crates/sim-core/src/persistence.rs`:

```rust
#[test]
fn snapshot_store_reports_count_and_sorted_coords() {
    let mut store = InMemoryChunkSnapshotStore::default();

    let mut east = Chunk::new(ChunkCoord { x: 5, y: 4 }, 32);
    east.set_tile_kind(0, TileKind::Water).expect("tile exists");
    let mut visible = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    visible
        .set_tile_kind(0, TileKind::Road)
        .expect("tile exists");

    store.write_snapshot(build_chunk_snapshot(
        "abutown-main",
        &east,
        ChunkActivity::Warm,
    ));
    store.write_snapshot(build_chunk_snapshot(
        "abutown-main",
        &visible,
        ChunkActivity::Active,
    ));

    assert_eq!(store.snapshot_count(), 2);
    assert_eq!(
        store.snapshot_coords(),
        vec![ChunkCoord { x: 4, y: 4 }, ChunkCoord { x: 5, y: 4 }]
    );
}
```

- [ ] **Step 2: Run the store tests to verify they fail**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core persistence::tests::snapshot_store_reports_count_and_sorted_coords
```

Expected: FAIL with missing methods `snapshot_count` and `snapshot_coords`.

- [ ] **Step 3: Implement store helpers**

Add these methods to `impl InMemoryChunkSnapshotStore` after `read_snapshot`:

```rust
pub fn snapshot_count(&self) -> usize {
    self.snapshots.len()
}

pub fn snapshot_coords(&self) -> Vec<ChunkCoord> {
    let mut coords: Vec<ChunkCoord> = self.snapshots.keys().copied().collect();
    coords.sort_by_key(|coord| (coord.y, coord.x));
    coords
}
```

- [ ] **Step 4: Run the focused test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core persistence::tests::snapshot_store_reports_count_and_sorted_coords
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/persistence.rs
git commit -m "feat: expose snapshot store queries"
```

---

### Task 2: Persist Registry Snapshots

**Files:**
- Modify: `backend/crates/sim-server/src/chunk_registry.rs`

- [ ] **Step 1: Write failing registry persistence tests**

Add `InMemoryChunkSnapshotStore` to the top imports in `backend/crates/sim-server/src/chunk_registry.rs`:

```rust
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    persistence::{InMemoryChunkSnapshotStore, build_chunk_snapshot},
    scheduler::ChunkActivity,
};
```

Add this test after `registry_reports_loaded_tile_counts`:

```rust
#[test]
fn registry_writes_snapshots_and_clears_dirty_tiles() {
    let mut registry = ChunkRegistry::new(32);
    registry.insert_chunk(
        chunk_with_seed(ChunkCoord { x: 5, y: 4 }, 7, TileKind::Water),
        ChunkActivity::Warm,
    );
    registry.insert_chunk(
        chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 3, TileKind::Road),
        ChunkActivity::Active,
    );

    let world_id = WorldId("abutown-main".to_string());
    let mut store = InMemoryChunkSnapshotStore::default();

    assert_eq!(registry.write_snapshots(&world_id, &mut store), 2);
    assert_eq!(store.snapshot_count(), 2);
    assert_eq!(
        store.snapshot_coords(),
        vec![ChunkCoord { x: 4, y: 4 }, ChunkCoord { x: 5, y: 4 }]
    );
    assert_eq!(
        store
            .read_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible snapshot exists")
            .dirty_tiles
            .len(),
        1
    );

    assert_eq!(registry.write_snapshots(&world_id, &mut store), 2);
    assert!(
        store
            .read_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible snapshot still exists")
            .dirty_tiles
            .is_empty()
    );
}
```

- [ ] **Step 2: Run the registry persistence test to verify it fails**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry::tests::registry_writes_snapshots_and_clears_dirty_tiles
```

Expected: FAIL with missing method `ChunkRegistry::write_snapshots`.

- [ ] **Step 3: Implement registry snapshot writing**

Add this method to `impl ChunkRegistry` after `tile_count`:

```rust
pub(crate) fn write_snapshots(
    &mut self,
    world_id: &WorldId,
    store: &mut InMemoryChunkSnapshotStore,
) -> usize {
    let coords = self.loaded_coords();
    let mut written = 0;

    for coord in coords {
        let Some(loaded) = self.chunks.get_mut(&coord) else {
            continue;
        };

        let snapshot = build_chunk_snapshot(&world_id.0, &loaded.chunk, loaded.activity);
        store.write_snapshot(snapshot);
        loaded.chunk.clear_dirty();
        written += 1;
    }

    written
}
```

- [ ] **Step 4: Run registry tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry
```

Expected: PASS, with the existing registry tests plus the new persistence test passing.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/chunk_registry.rs
git commit -m "feat: persist chunk registry snapshots"
```

---

### Task 3: Add Runtime Snapshot Store

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Write failing runtime persistence tests**

Add `InMemoryChunkSnapshotStore` to the imports in `backend/crates/sim-server/src/runtime.rs`:

```rust
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    persistence::InMemoryChunkSnapshotStore,
    scheduler::ChunkActivity,
    tile::TileKind,
};
```

Add this test after `runtime_rotates_pulses_through_loaded_chunks`:

```rust
#[test]
fn runtime_persists_loaded_chunk_snapshots_and_clears_dirty_state() {
    let mut runtime = SimulationRuntime::new();

    assert_eq!(runtime.persist_chunk_snapshots(), 3);

    let visible = runtime
        .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
        .expect("visible snapshot stored");
    assert_eq!(visible.coord, ChunkCoordDto { x: 4, y: 4 });
    assert_eq!(visible.dirty_tiles.len(), 1);

    let east = runtime
        .stored_chunk_snapshot(ChunkCoord { x: 5, y: 4 })
        .expect("east snapshot stored");
    assert_eq!(east.coord, ChunkCoordDto { x: 5, y: 4 });
    assert_eq!(east.dirty_tiles.len(), 1);

    assert_eq!(runtime.persist_chunk_snapshots(), 3);
    assert!(
        runtime
            .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible snapshot remains stored")
            .dirty_tiles
            .is_empty()
    );
}
```

- [ ] **Step 2: Run runtime persistence test to verify it fails**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime::tests::runtime_persists_loaded_chunk_snapshots_and_clears_dirty_state
```

Expected: FAIL with missing `snapshot_store`, `persist_chunk_snapshots`, and `stored_chunk_snapshot`.

- [ ] **Step 3: Add snapshot store to runtime**

Modify the `SimulationRuntime` struct to:

```rust
pub struct SimulationRuntime {
    world_id: WorldId,
    registry: ChunkRegistry,
    snapshot_store: InMemoryChunkSnapshotStore,
    tick: u64,
    version: u64,
}
```

Modify the `Self` returned by `SimulationRuntime::new()` to include:

```rust
snapshot_store: InMemoryChunkSnapshotStore::default(),
```

Add these methods after `chunk_snapshot`:

```rust
pub fn persist_chunk_snapshots(&mut self) -> usize {
    self.registry
        .write_snapshots(&self.world_id, &mut self.snapshot_store)
}

pub fn stored_chunk_snapshot(&self, coord: ChunkCoord) -> Option<&ChunkSnapshotDto> {
    self.snapshot_store.read_snapshot(coord)
}
```

- [ ] **Step 4: Run runtime tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime::
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs
git commit -m "feat: add runtime snapshot store"
```

---

### Task 4: Start Server Snapshot Loop

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

- [ ] **Step 1: Write failing app snapshot helper test**

Add this test module to the bottom of `backend/crates/sim-server/src/app.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::ids::ChunkCoord;

    #[tokio::test]
    async fn persist_snapshots_once_writes_runtime_snapshots() {
        let state = AppState::new(SimulationRuntime::new());

        assert_eq!(persist_snapshots_once(&state).await, 3);

        let runtime = state.runtime();
        let runtime = runtime.lock().await;
        let snapshot = runtime
            .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible snapshot stored");
        assert_eq!(snapshot.coord.x, 4);
        assert_eq!(snapshot.coord.y, 4);
    }
}
```

- [ ] **Step 2: Run app test to verify it fails**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server app::tests::persist_snapshots_once_writes_runtime_snapshots
```

Expected: FAIL with missing function `persist_snapshots_once`.

- [ ] **Step 3: Implement snapshot loop wiring**

Add this constant next to `SIMULATION_TICK_INTERVAL`:

```rust
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);
```

Add this method to `impl AppState` after `spawn_delta_loop`:

```rust
fn spawn_snapshot_loop(&self, snapshot_interval: Duration) {
    let state = self.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(snapshot_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            let _ = persist_snapshots_once(&state).await;
        }
    });
}
```

In `build_app_with_runtime`, after `state.spawn_delta_loop(SIMULATION_TICK_INTERVAL);`, add:

```rust
state.spawn_snapshot_loop(SNAPSHOT_INTERVAL);
```

Add this helper after `stream_world_deltas`:

```rust
async fn persist_snapshots_once(state: &AppState) -> usize {
    let runtime = state.runtime();
    let mut runtime = runtime.lock().await;
    runtime.persist_chunk_snapshots()
}
```

- [ ] **Step 4: Run app tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server app::tests::persist_snapshots_once_writes_runtime_snapshots
```

Expected: PASS.

- [ ] **Step 5: Run websocket tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: PASS. The snapshot loop must not interfere with websocket broadcast cadence.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/app.rs
git commit -m "feat: run backend snapshot loop"
```

---

### Task 5: Document And Verify Snapshot Loop

**Files:**
- Modify: `backend/README.md`

- [ ] **Step 1: Update backend README**

Add this paragraph after the current `/ws` ticking paragraph in `backend/README.md`:

```markdown
The server also runs an in-memory snapshot loop every five seconds. It writes snapshots for all loaded chunks into the current process snapshot store and clears chunk dirty flags after each successful pass. This is the first persistence boundary; Supabase/Postgres adapters remain a later slice.
```

- [ ] **Step 2: Run complete backend verification**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: all commands pass.

- [ ] **Step 3: Run relevant frontend verification**

Run:

```bash
npm test -- tests/backend/backendState.test.ts
npm run build
```

Expected:
- backend bridge tests pass,
- production build passes.

- [ ] **Step 4: Run browser smoke**

Start the backend server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

Start the Vite client:

```bash
npm run dev -- --port 5177
```

Open `http://127.0.0.1:5177/` and confirm:

- `RUST LIVE` appears,
- `world abutown-main` appears,
- `chunk 4:4 active` appears,
- tick/version increments after waiting long enough for a `4:4` pulse,
- no page errors are reported.

Stop both servers after the smoke test.

- [ ] **Step 5: Commit**

```bash
git add backend/README.md
git commit -m "docs: document backend snapshot loop"
```

- [ ] **Step 6: Push plan branch**

Run:

```bash
git push -u origin codex/backend-persistence-snapshot-plan
```

Expected: branch pushes successfully.

---

## Self-Review

- Spec coverage: The plan covers the first in-memory persistence boundary, dirty snapshot writing, dirty clearing, runtime-owned snapshot storage, server loop wiring, docs, and verification. It intentionally excludes Supabase/Postgres and recovery.
- Placeholder scan: No placeholder tokens or vague test instructions remain. Every code-changing step includes exact code and commands.
- Type consistency: `InMemoryChunkSnapshotStore`, `ChunkRegistry::write_snapshots`, `SimulationRuntime::persist_chunk_snapshots`, and `persist_snapshots_once` are introduced before use by later tasks.
