# DB Terrain Seed Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the backend-authoritative Zurich layered terrain seed into the chunk snapshot store on first persistent startup without overwriting existing terrain chunks.

**Architecture:** Add a small runtime bootstrap helper that reads the bundled validated terrain seed, checks each seed chunk against `ChunkSnapshotStore`, writes only missing seed chunks as normal `ChunkSnapshotDto` records, and then hydrates runtime from the store as before. The same helper works for Postgres and in-memory stores through the existing trait, so no new terrain schema or economy/domain fields are introduced.

**Tech Stack:** Rust, async_trait snapshot store trait, existing chunk snapshot JSONB persistence, existing layered terrain seed loader.

---

## File Structure

- Modify `backend/crates/sim-server/src/runtime.rs` — add `TerrainSeedBootstrapReport`, seed chunk snapshot builder, bootstrap helper, and call it inside `hydrate_from_stores` before chunk hydration reads.
- Test in `backend/crates/sim-server/src/runtime.rs` — focused async unit tests using `InMemoryChunkSnapshotStore`.
- Modify `progress.md` — record that terrain seed bootstrap is now DB/persistence-backed after verification.

## Guardrails

- Do not add companies, homes, workplaces, ownership, jobs, production, money, ledger, forestry, or resource extraction.
- Do not add a new one-dimensional `TileKind`.
- Do not overwrite an existing chunk snapshot, even if it differs from the bundled seed.
- Do not make frontend builders runtime authority again.

## Tasks

### Task 1: Add Failing Bootstrap Tests

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Add tests for empty and partially populated stores**

Add these tests inside the existing `#[cfg(test)] mod tests` in `backend/crates/sim-server/src/runtime.rs`:

```rust
#[tokio::test]
async fn terrain_seed_bootstrap_writes_missing_chunks_to_empty_snapshot_store() {
    let seed = load_validated_layered_seed().expect("seed loads");
    let mut store = InMemoryChunkSnapshotStore::default();

    let report = bootstrap_missing_seed_chunk_snapshots(
        &mut store,
        &seed,
        &SimulationRuntime::default_world_id(),
    )
    .await
    .expect("bootstrap seed chunks");

    assert_eq!(report.written_chunks, 64);
    assert_eq!(report.existing_chunks, 0);
    assert_eq!(store.snapshot_count(), 64);

    let stored = ChunkSnapshotStore::read_snapshot(&store, ChunkCoord { x: 4, y: 4 })
        .await
        .unwrap()
        .expect("seeded chunk exists");
    assert_eq!(stored.world_id, SimulationRuntime::default_world_id());
    assert_eq!(stored.chunk_version, 0);
    assert_eq!(stored.tile_count, 1024);
    assert!(stored.tiles.iter().any(|tile| tile.base == TileBaseDto::Water));
    assert!(stored.tiles.iter().any(|tile| tile.surface == TileSurfaceDto::Street));
}

#[tokio::test]
async fn terrain_seed_bootstrap_preserves_existing_chunk_snapshots() {
    use sim_core::tile::{TileBase, TileRecord};

    let seed = load_validated_layered_seed().expect("seed loads");
    let mut store = InMemoryChunkSnapshotStore::default();
    let coord = ChunkCoord { x: 4, y: 4 };
    let mut custom_chunk = Chunk::new(coord, 32);
    custom_chunk
        .set_tile_record(
            0,
            TileRecord {
                base: TileBase::Park,
                version: 99,
                ..TileRecord::default()
            },
        )
        .unwrap();
    let mut custom_snapshot =
        build_chunk_snapshot("abutown-main", &custom_chunk, ChunkActivity::Warm);
    custom_snapshot.chunk_version = 99;
    ChunkSnapshotStore::write_snapshot(&mut store, custom_snapshot.clone())
        .await
        .unwrap();

    let report = bootstrap_missing_seed_chunk_snapshots(
        &mut store,
        &seed,
        &SimulationRuntime::default_world_id(),
    )
    .await
    .expect("bootstrap missing seed chunks");

    assert_eq!(report.written_chunks, 63);
    assert_eq!(report.existing_chunks, 1);
    assert_eq!(store.snapshot_count(), 64);

    let preserved = ChunkSnapshotStore::read_snapshot(&store, coord)
        .await
        .unwrap()
        .expect("custom chunk still exists");
    assert_eq!(preserved.chunk_version, 99);
    assert_eq!(preserved.chunk_state, abutown_protocol::ChunkStateDto::Warm);
    assert_eq!(preserved.tiles[0].base, TileBaseDto::Park);

    let filled = ChunkSnapshotStore::read_snapshot(&store, ChunkCoord { x: 0, y: 0 })
        .await
        .unwrap()
        .expect("missing seed chunk was written");
    assert_eq!(filled.chunk_version, 0);
    assert_eq!(filled.tile_count, 1024);
}
```

- [x] **Step 2: Run tests to verify RED**

Run:

```bash
PATH="/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-server terrain_seed_bootstrap_
```

Expected: FAIL because `bootstrap_missing_seed_chunk_snapshots` and `TerrainSeedBootstrapReport` do not exist.

### Task 2: Implement Bootstrap Helper

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Add report struct and seed snapshot helper**

Add near the seed helpers in `runtime.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerrainSeedBootstrapReport {
    written_chunks: usize,
    existing_chunks: usize,
}

fn seed_chunk_snapshot(
    seed: &LayeredTerrainSeed,
    world_id: &WorldId,
    coord: ChunkCoord,
) -> Result<ChunkSnapshotDto, HydrationError> {
    let tiles = sim_core::terrain_seed::chunk_tiles_from_seed(seed, coord).ok_or_else(|| {
        HydrationError::Seed(format!(
            "seed missing chunk tiles for {}:{}",
            coord.x, coord.y
        ))
    })?;
    Ok(build_chunk_snapshot_from_parts(
        &world_id.0,
        coord,
        &tiles,
        0,
        ChunkActivity::Active,
    ))
}
```

- [x] **Step 2: Add idempotent write-missing helper**

Add near `seed_chunk_snapshot`:

```rust
async fn bootstrap_missing_seed_chunk_snapshots(
    snapshot_store: &mut dyn ChunkSnapshotStore,
    seed: &LayeredTerrainSeed,
    world_id: &WorldId,
) -> Result<TerrainSeedBootstrapReport, HydrationError> {
    let chunk_size = u32::from(seed.chunk_size);
    let mut report = TerrainSeedBootstrapReport {
        written_chunks: 0,
        existing_chunks: 0,
    };

    for chunk_y in 0..(seed.height / chunk_size) {
        for chunk_x in 0..(seed.width / chunk_size) {
            let coord = ChunkCoord {
                x: chunk_x as i32,
                y: chunk_y as i32,
            };
            if snapshot_store
                .read_snapshot(coord)
                .await
                .map_err(HydrationError::Snapshot)?
                .is_some()
            {
                report.existing_chunks += 1;
                continue;
            }

            let snapshot = seed_chunk_snapshot(seed, world_id, coord)?;
            snapshot_store
                .write_snapshot(snapshot)
                .await
                .map_err(HydrationError::Snapshot)?;
            report.written_chunks += 1;
        }
    }

    Ok(report)
}
```

- [x] **Step 3: Call bootstrap before hydration reads**

In `SimulationRuntime::hydrate_from_stores`, shadow `snapshot_store` as mutable and call the helper after loading the seed:

```rust
let mut snapshot_store = snapshot_store;
let seed = load_validated_layered_seed()?;
let _bootstrap_report =
    bootstrap_missing_seed_chunk_snapshots(&mut *snapshot_store, &seed, &world_id).await?;
let seed_chunk_size = u32::from(seed.chunk_size);
```

Remove the older duplicate `let seed = load_validated_layered_seed()?;` line from the chunk hydration block.

- [x] **Step 4: Run tests to verify GREEN**

Run:

```bash
PATH="/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-server terrain_seed_bootstrap_
```

Expected: PASS.

### Task 3: Broader Verification And Record Progress

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Run targeted hydration tests**

Run:

```bash
PATH="/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-server hydrate_from_stores
```

Expected: PASS.

- [x] **Step 2: Run full verification**

Run:

```bash
PATH="/Users/ramonfuglister/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml --workspace
npm test
npm run build
npx playwright test tests/e2e/render-smoke.spec.ts --project=chromium
```

Expected: all pass.

- [x] **Step 3: Update `progress.md`**

Add one concise entry near the top:

```markdown
2026-05-27T20:XX:XX.000Z - DB terrain seed bootstrap: persistent runtime hydration now writes missing Zurich layered terrain seed chunks into `ChunkSnapshotStore` before reading chunks back, preserving any existing chunk snapshots and filling only missing 32x32 chunks. This keeps the DB terrain state idempotent and terrain-only; no economy/domain fields were introduced. Verification: targeted bootstrap/hydration tests passed, backend workspace tests passed, Vitest passed, build passed, and Playwright render smoke passed.
```

- [x] **Step 4: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs progress.md docs/superpowers/plans/2026-05-27-db-terrain-seed-bootstrap.md
git commit -m "feat: bootstrap terrain seed snapshots"
```

Expected: commit succeeds.

## Self-Review

- Spec coverage: covers first-start DB persistence, idempotency, non-overwrite, and terrain-only scope.
- Placeholder scan: no TBD/TODO placeholders.
- Type consistency: uses existing `ChunkSnapshotStore`, `LayeredTerrainSeed`, `WorldId`, `ChunkCoord`, `ChunkSnapshotDto`, and `build_chunk_snapshot_from_parts`.
