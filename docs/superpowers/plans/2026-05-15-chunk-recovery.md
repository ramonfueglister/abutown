# Chunk Recovery & Command Idempotency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** After a `sim-server` restart, the world state is byte-for-byte identical to the state before the restart, and a duplicate `command_id` never produces a second mutation.

**Architecture:** Per-chunk CQRS + Event Sourcing. Chunk aggregates hydrate from a full-state snapshot, then replay events with `chunk_version > snapshot.chunk_version`. Commands deduplicate via a `UNIQUE (world_id, command_id)` constraint on `world_events`. Mobility is explicitly out of scope.

**Tech Stack:** Rust 2024, Tokio, `async_trait`, `sqlx` (Postgres), existing `WorldEventStore` / `ChunkSnapshotStore` traits, existing `Chunk` aggregate.

**Spec:** `docs/superpowers/specs/2026-05-15-chunk-recovery-design.md`

---

## File Structure

- Modify: `backend/crates/protocol/src/lib.rs`
  - Rename `ChunkSnapshotDto.dirty_tiles` → `ChunkSnapshotDto.tiles`. Update JSON test fixtures.
- Modify: `backend/crates/sim-core/src/chunk.rs`
  - Add `Chunk::from_snapshot()` and `Chunk::apply_event()`. Define `SnapshotDecodeError`, `EventApplyError`.
- Modify: `backend/crates/sim-core/src/persistence.rs`
  - Change `build_chunk_snapshot` to emit all non-default tiles (full-state sparse), not only `dirty_indices`.
- Modify: `backend/crates/sim-core/src/events.rs`
  - Extend `WorldEventStore` trait with `find_event_by_command`, `read_chunk_events_since`, `max_tick`, `max_version`. Implement on `InMemoryWorldEventStore` and `FailingWorldEventStore`.
- Modify: `backend/crates/sim-server/src/postgres_events.rs`
  - Implement the four new trait methods against Postgres. Make `append` rely on the new unique constraint.
- Create: `backend/crates/sim-server/migrations/202605160001_chunk_recovery.sql`
  - Add `chunk_x`, `chunk_y`, `chunk_version` columns + backfill + unique index on `(world_id, command_id)` + per-chunk replay index.
- Modify: `backend/crates/sim-server/src/chunk_registry.rs`
  - Add `insert_hydrated(chunk, last_persisted_version, activity)`. Track `last_persisted_version` and `last_snapshot_at` per loaded chunk. Make `collect_snapshots` only return chunks where `current_version > last_persisted_version` OR `now - last_snapshot_at >= 30s`.
- Modify: `backend/crates/sim-server/src/runtime.rs`
  - Add async `SimulationRuntime::hydrate_from_stores`. Add pre-flight dedup + clone-then-commit in command handling. Restore `tick`/`version` from event store.
- Modify: `backend/crates/sim-server/src/app.rs`
  - When `database_url` is configured, call `hydrate_from_stores` instead of `new_with_stores`.
- Modify: `backend/crates/sim-server/tests/http.rs`
  - Recovery + idempotency integration tests (opt-in via `ABUTOWN_TEST_DATABASE_URL`).

---

## Task 1: Snapshot Format Becomes Full-State Sparse

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`
- Modify: `backend/crates/sim-core/src/persistence.rs`
- Modify: `backend/crates/sim-core/src/chunk.rs` (test reference)
- Modify: `backend/crates/sim-server/src/postgres_snapshots.rs` (field rename only)
- Modify: `backend/crates/sim-server/src/chunk_registry.rs` (test reference)
- Modify: `backend/crates/sim-server/src/runtime.rs` (test reference)

- [x] **Step 1: Rename `dirty_tiles` → `tiles` in the DTO**

In `backend/crates/protocol/src/lib.rs`, locate the `ChunkSnapshotDto` struct (around line 61) and the field name `dirty_tiles`. Rename it to `tiles`. Update every place in the file that references `dirty_tiles` (including JSON serde rename if present, and test fixtures around lines 296–365).

Also rename the snapshot JSON key in the existing tests: search the file for the literal string `"dirty_tiles"` and replace with `"tiles"`.

- [x] **Step 2: Update protocol JSON tests**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol
```

Expected: all tests pass. If a JSON round-trip test references `dirty_tiles`, update its expected string to `tiles`.

- [x] **Step 3: Write a failing test for full-state snapshot**

In `backend/crates/sim-core/src/persistence.rs`, inside the existing `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn build_chunk_snapshot_emits_all_non_default_tiles_after_clear_dirty() {
    let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    chunk.set_tile_kind(0, TileKind::Road).unwrap();
    chunk.set_tile_kind(17, TileKind::Water).unwrap();
    chunk.clear_dirty();
    chunk.set_tile_kind(42, TileKind::BuildingFootprint).unwrap();

    let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

    let indices: Vec<u16> = snapshot.tiles.iter().map(|t| t.local_index).collect();
    assert_eq!(indices, vec![0, 17, 42], "snapshot must include all non-default tiles, not only currently-dirty ones");
    assert_eq!(snapshot.chunk_version, 3);
}
```

- [x] **Step 4: Run the test to verify it fails**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core build_chunk_snapshot_emits_all_non_default_tiles
```

Expected: FAIL — the current code only emits dirty indices, so index 0 and 17 are missing after `clear_dirty`.

- [x] **Step 5: Implement full-state sparse encoding**

In `backend/crates/sim-core/src/persistence.rs`, replace the body of `build_chunk_snapshot`:

```rust
pub fn build_chunk_snapshot(
    world_id: impl Into<String>,
    chunk: &Chunk,
    activity: ChunkActivity,
) -> ChunkSnapshotDto {
    let mut tiles: Vec<TileMutationDto> = Vec::new();
    for index in 0..chunk.tile_count() {
        let tile = chunk.tile_at(index).expect("index within tile_count");
        if tile.kind != TileKind::default() {
            tiles.push(TileMutationDto {
                local_index: index,
                kind: tile.kind.into(),
                version: tile.version,
            });
        }
    }

    ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId(world_id.into()),
        coord: chunk.coord().into(),
        chunk_state: activity.into(),
        chunk_version: chunk.version(),
        tile_count: chunk.tile_count(),
        tiles,
    }
}
```

Add `use crate::tile::TileKind;` at the top if not already imported.

- [x] **Step 6: Update any other references to `dirty_tiles`**

Run:

```bash
grep -rn "dirty_tiles" backend/
```

Expected output should be empty after this task. If there are references in `postgres_snapshots.rs`, `chunk_registry.rs`, `runtime.rs`, or any test, rename them to `tiles`.

- [x] **Step 7: Run all affected tests**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol
```

Expected: all green. If any test asserts on the old delta behavior (e.g., "snapshot is empty after clear_dirty"), update it to assert the new full-state behavior.

- [x] **Step 8: Commit**

```bash
git add backend/crates/protocol/src/lib.rs backend/crates/sim-core/src/persistence.rs backend/crates/sim-server/src/postgres_snapshots.rs backend/crates/sim-server/src/chunk_registry.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat: chunk snapshots carry full non-default tile state"
```

---

## Task 2: `Chunk::from_snapshot`

**Files:**
- Modify: `backend/crates/sim-core/src/chunk.rs`

- [x] **Step 1: Write the failing test**

Add inside `chunk.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn chunk_from_snapshot_round_trips_full_state() {
    use crate::persistence::build_chunk_snapshot;
    use crate::scheduler::ChunkActivity;

    let mut original = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    original.set_tile_kind(0, TileKind::Road).unwrap();
    original.set_tile_kind(17, TileKind::Water).unwrap();
    original.set_tile_kind(42, TileKind::BuildingFootprint).unwrap();

    let snapshot = build_chunk_snapshot("abutown-main", &original, ChunkActivity::Active);
    let restored = Chunk::from_snapshot(&snapshot).unwrap();

    assert_eq!(restored.coord(), original.coord());
    assert_eq!(restored.chunk_size(), original.chunk_size());
    assert_eq!(restored.version(), original.version());
    assert_eq!(restored.kind_at(0), Some(TileKind::Road));
    assert_eq!(restored.kind_at(17), Some(TileKind::Water));
    assert_eq!(restored.kind_at(42), Some(TileKind::BuildingFootprint));
    assert_eq!(restored.kind_at(1), Some(TileKind::default()));
    assert_eq!(restored.dirty_indices(), Vec::<u16>::new());
}

#[test]
fn chunk_from_snapshot_rejects_oversized_local_index() {
    use abutown_protocol::{ChunkCoordDto, ChunkSnapshotDto, PROTOCOL_VERSION, TileKindDto, TileMutationDto, WorldId};
    use crate::scheduler::ChunkActivity;

    let snapshot = ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        coord: ChunkCoordDto { x: 4, y: 4 },
        chunk_state: ChunkActivity::Active.into(),
        chunk_version: 1,
        tile_count: 1024,
        tiles: vec![TileMutationDto {
            local_index: 9999,
            kind: TileKindDto::Road,
            version: 1,
        }],
    };

    let err = Chunk::from_snapshot(&snapshot).unwrap_err();
    assert!(matches!(err, SnapshotDecodeError::IndexOutOfBounds { index: 9999, .. }));
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core chunk_from_snapshot
```

Expected: FAIL — `Chunk::from_snapshot` and `SnapshotDecodeError` do not exist yet.

- [x] **Step 3: Implement `from_snapshot` + error type**

In `backend/crates/sim-core/src/chunk.rs`, add:

```rust
use abutown_protocol::ChunkSnapshotDto;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotDecodeError {
    #[error("snapshot tile_count {tile_count} does not fit u16 indices")]
    InvalidTileCount { tile_count: u16 },
    #[error("tile index {index} is outside snapshot tile_count {tile_count}")]
    IndexOutOfBounds { index: u16, tile_count: u16 },
}
```

Add the method on `impl Chunk`:

```rust
pub fn from_snapshot(snapshot: &ChunkSnapshotDto) -> Result<Self, SnapshotDecodeError> {
    let tile_count = snapshot.tile_count;
    let chunk_size = (tile_count as f64).sqrt() as u16;
    if usize::from(chunk_size) * usize::from(chunk_size) != usize::from(tile_count) {
        return Err(SnapshotDecodeError::InvalidTileCount { tile_count });
    }

    let mut tiles = vec![TileRecord::default(); usize::from(tile_count)];
    for mutation in &snapshot.tiles {
        let slot = tiles
            .get_mut(usize::from(mutation.local_index))
            .ok_or(SnapshotDecodeError::IndexOutOfBounds {
                index: mutation.local_index,
                tile_count,
            })?;
        slot.kind = mutation.kind.into();
        slot.version = mutation.version;
        slot.flags.modified = true;
    }

    Ok(Self {
        coord: ChunkCoord {
            x: snapshot.coord.x,
            y: snapshot.coord.y,
        },
        chunk_size,
        version: snapshot.chunk_version,
        tiles,
        dirty: BTreeSet::new(),
    })
}
```

- [x] **Step 4: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core chunk
```

Expected: all chunk tests pass including the two new ones.

Commit:

```bash
git add backend/crates/sim-core/src/chunk.rs
git commit -m "feat: reconstruct chunk from full-state snapshot"
```

---

## Task 3: `Chunk::apply_event`

**Files:**
- Modify: `backend/crates/sim-core/src/chunk.rs`

- [x] **Step 1: Write the failing tests**

In `chunk.rs` test module:

```rust
#[test]
fn chunk_apply_event_advances_version_and_mutates_tile() {
    use abutown_protocol::{ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId};

    let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id: "event:1".to_string(),
        command_id: "command:1".to_string(),
        world_id: WorldId("abutown-main".to_string()),
        tick: 1,
        version: 1,
        coord: ChunkCoordDto { x: 4, y: 4 },
        local_index: 7,
        kind: TileKindDto::Road,
    });

    chunk.apply_event(&event, 1).unwrap();

    assert_eq!(chunk.version(), 1);
    assert_eq!(chunk.kind_at(7), Some(TileKind::Road));
}

#[test]
fn chunk_apply_event_rejects_event_for_wrong_coord() {
    use abutown_protocol::{ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId};

    let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id: "event:1".to_string(),
        command_id: "command:1".to_string(),
        world_id: WorldId("abutown-main".to_string()),
        tick: 1,
        version: 1,
        coord: ChunkCoordDto { x: 9, y: 9 },
        local_index: 7,
        kind: TileKindDto::Road,
    });

    let err = chunk.apply_event(&event, 1).unwrap_err();
    assert!(matches!(err, EventApplyError::WrongChunkCoord { .. }));
}

#[test]
fn chunk_apply_event_idempotent_for_same_chunk_version() {
    use abutown_protocol::{ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId};

    let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id: "event:1".to_string(),
        command_id: "command:1".to_string(),
        world_id: WorldId("abutown-main".to_string()),
        tick: 1,
        version: 1,
        coord: ChunkCoordDto { x: 4, y: 4 },
        local_index: 7,
        kind: TileKindDto::Road,
    });

    chunk.apply_event(&event, 1).unwrap();
    chunk.apply_event(&event, 1).unwrap();

    assert_eq!(chunk.version(), 1, "re-applying the same chunk_version must not bump version");
    assert_eq!(chunk.kind_at(7), Some(TileKind::Road));
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core chunk_apply_event
```

Expected: FAIL — `apply_event` and `EventApplyError` do not exist.

- [x] **Step 3: Implement `apply_event`**

Add the error in `chunk.rs`:

```rust
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventApplyError {
    #[error("event coord ({event_x},{event_y}) does not match chunk coord ({chunk_x},{chunk_y})")]
    WrongChunkCoord { event_x: i32, event_y: i32, chunk_x: i32, chunk_y: i32 },
    #[error("event chunk_version {event_version} is older than current chunk version {chunk_version}")]
    StaleEvent { event_version: u64, chunk_version: u64 },
    #[error("event chunk_version {event_version} skips past current chunk version {chunk_version}")]
    GapEvent { event_version: u64, chunk_version: u64 },
    #[error("tile index {index} is outside chunk tile count {tile_count}")]
    IndexOutOfBounds { index: u16, tile_count: u16 },
}
```

Add the method on `impl Chunk`:

```rust
pub fn apply_event(
    &mut self,
    event: &abutown_protocol::WorldEventDto,
    event_chunk_version: u64,
) -> Result<(), EventApplyError> {
    use abutown_protocol::WorldEventDto;

    if event_chunk_version == self.version {
        return Ok(());
    }
    if event_chunk_version < self.version {
        return Err(EventApplyError::StaleEvent {
            event_version: event_chunk_version,
            chunk_version: self.version,
        });
    }
    if event_chunk_version != self.version + 1 {
        return Err(EventApplyError::GapEvent {
            event_version: event_chunk_version,
            chunk_version: self.version,
        });
    }

    match event {
        WorldEventDto::TileKindSet(payload) => {
            if payload.coord.x != self.coord.x || payload.coord.y != self.coord.y {
                return Err(EventApplyError::WrongChunkCoord {
                    event_x: payload.coord.x,
                    event_y: payload.coord.y,
                    chunk_x: self.coord.x,
                    chunk_y: self.coord.y,
                });
            }
            let tile_count = self.tile_count();
            let slot = self.tiles.get_mut(usize::from(payload.local_index)).ok_or(
                EventApplyError::IndexOutOfBounds {
                    index: payload.local_index,
                    tile_count,
                },
            )?;
            self.version = event_chunk_version;
            slot.kind = payload.kind.into();
            slot.version = self.version;
            slot.flags.modified = true;
            self.dirty.insert(payload.local_index);
        }
    }

    Ok(())
}
```

- [x] **Step 4: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core chunk
```

Expected: all chunk tests pass.

Commit:

```bash
git add backend/crates/sim-core/src/chunk.rs
git commit -m "feat: apply world events to chunk for replay"
```

---

## Task 4: Extend `WorldEventStore` Trait

**Files:**
- Modify: `backend/crates/sim-core/src/events.rs`

- [x] **Step 1: Write failing tests for new trait methods on in-memory store**

In `events.rs` test module, add:

```rust
#[tokio::test]
async fn event_store_finds_event_by_command_id() {
    let mut store = InMemoryWorldEventStore::default();
    WorldEventStore::append(&mut store, tile_event("event:1", 1)).await.unwrap();
    WorldEventStore::append(&mut store, tile_event("event:2", 2)).await.unwrap();

    let found = WorldEventStore::find_event_by_command(&store, "abutown-main", "command:event:1")
        .await
        .unwrap();
    assert_eq!(found, Some(tile_event("event:1", 1)));

    let missing = WorldEventStore::find_event_by_command(&store, "abutown-main", "command:nope")
        .await
        .unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn event_store_reads_chunk_events_since_version() {
    let mut store = InMemoryWorldEventStore::default();
    WorldEventStore::append(&mut store, tile_event("event:1", 1)).await.unwrap();
    WorldEventStore::append(&mut store, tile_event("event:2", 2)).await.unwrap();
    WorldEventStore::append(&mut store, tile_event("event:3", 3)).await.unwrap();

    let events = WorldEventStore::read_chunk_events_since(
        &store,
        "abutown-main",
        ChunkCoordDto { x: 4, y: 4 },
        1,
    )
    .await
    .unwrap();

    assert_eq!(events, vec![tile_event("event:2", 2), tile_event("event:3", 3)]);
}

#[tokio::test]
async fn event_store_reports_max_tick_and_version() {
    let mut store = InMemoryWorldEventStore::default();
    assert_eq!(WorldEventStore::max_tick(&store, "abutown-main").await.unwrap(), None);
    assert_eq!(WorldEventStore::max_version(&store, "abutown-main").await.unwrap(), None);

    WorldEventStore::append(&mut store, tile_event("event:1", 5)).await.unwrap();
    WorldEventStore::append(&mut store, tile_event("event:2", 9)).await.unwrap();

    assert_eq!(WorldEventStore::max_tick(&store, "abutown-main").await.unwrap(), Some(9));
    assert_eq!(WorldEventStore::max_version(&store, "abutown-main").await.unwrap(), Some(9));
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core event_store
```

Expected: FAIL — three new methods do not exist on the trait.

- [x] **Step 3: Add a `DuplicateCommand` error variant**

In `events.rs`, extend the error:

```rust
impl WorldEventStoreError {
    pub fn duplicate_command(command_id: impl Into<String>) -> Self {
        Self {
            code: "duplicate_command_id",
            message: format!("command_id already present: {}", command_id.into()),
        }
    }
}
```

- [x] **Step 4: Extend the trait**

Replace the trait body:

```rust
#[async_trait]
pub trait WorldEventStore: std::fmt::Debug + Send {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError>;

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError>;

    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError>;

    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError>;

    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError>;
}
```

- [x] **Step 5: Implement on `InMemoryWorldEventStore`**

Replace the impl block:

```rust
#[async_trait]
impl WorldEventStore for InMemoryWorldEventStore {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        let metadata = WorldEventMetadata::from_event(&event);
        if self.events.iter().any(|existing| {
            let m = WorldEventMetadata::from_event(existing);
            m.world_id == metadata.world_id && m.command_id == metadata.command_id
        }) {
            return Err(WorldEventStoreError::duplicate_command(metadata.command_id));
        }
        self.events.push(event);
        Ok(())
    }

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        Ok(self.events.iter().find(|event| {
            let m = WorldEventMetadata::from_event(event);
            m.world_id == world_id && m.command_id == command_id
        }).cloned())
    }

    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        let mut matches: Vec<WorldEventDto> = self
            .events
            .iter()
            .filter(|event| {
                let m = WorldEventMetadata::from_event(event);
                if m.world_id != world_id {
                    return false;
                }
                match event {
                    WorldEventDto::TileKindSet(payload) => {
                        payload.coord.x == coord.x
                            && payload.coord.y == coord.y
                            && payload.version > after_chunk_version
                    }
                }
            })
            .cloned()
            .collect();
        matches.sort_by_key(|event| match event {
            WorldEventDto::TileKindSet(payload) => payload.version,
        });
        Ok(matches)
    }

    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Ok(self
            .events
            .iter()
            .filter(|event| WorldEventMetadata::from_event(event).world_id == world_id)
            .map(|event| WorldEventMetadata::from_event(event).tick)
            .max())
    }

    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Ok(self
            .events
            .iter()
            .filter(|event| WorldEventMetadata::from_event(event).world_id == world_id)
            .map(|event| WorldEventMetadata::from_event(event).version)
            .max())
    }
}
```

Note: the in-memory `append` now rejects duplicates to mirror the new Postgres unique-constraint behavior. Update the existing `event_store_appends_events_in_order` test only if needed (it uses distinct command_ids derived from event_id, so it should still pass).

- [x] **Step 6: Implement on `FailingWorldEventStore`**

```rust
#[async_trait]
impl WorldEventStore for FailingWorldEventStore {
    async fn append(&mut self, _event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn find_event_by_command(
        &self,
        _world_id: &str,
        _command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn read_chunk_events_since(
        &self,
        _world_id: &str,
        _coord: abutown_protocol::ChunkCoordDto,
        _after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn max_tick(&self, _world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn max_version(&self, _world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
}
```

- [x] **Step 7: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core events
```

Expected: all event tests pass.

Commit:

```bash
git add backend/crates/sim-core/src/events.rs
git commit -m "feat: world event store supports dedup and chunk replay queries"
```

---

## Task 5: Postgres Migration For Recovery

**Files:**
- Create: `backend/crates/sim-server/migrations/202605160001_chunk_recovery.sql`

- [x] **Step 1: Write the migration**

Create the file with the following contents:

```sql
ALTER TABLE world_events
  ADD COLUMN IF NOT EXISTS chunk_x INTEGER,
  ADD COLUMN IF NOT EXISTS chunk_y INTEGER,
  ADD COLUMN IF NOT EXISTS chunk_version BIGINT;

UPDATE world_events
   SET chunk_x = (payload->'coord'->>'x')::int,
       chunk_y = (payload->'coord'->>'y')::int,
       chunk_version = version
 WHERE chunk_x IS NULL;

ALTER TABLE world_events
  ALTER COLUMN chunk_x SET NOT NULL,
  ALTER COLUMN chunk_y SET NOT NULL,
  ALTER COLUMN chunk_version SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS world_events_world_command_uniq
  ON world_events (world_id, command_id);

CREATE INDEX IF NOT EXISTS world_events_chunk_version_idx
  ON world_events (world_id, chunk_x, chunk_y, chunk_version);
```

- [x] **Step 2: Verify migration ordering**

Run:

```bash
ls backend/crates/sim-server/migrations/
```

Expected output includes `202605150001_world_events.sql`, `202605150002_card_hand_core.sql`, `202605150003_chunk_snapshots.sql`, `202605160001_chunk_recovery.sql` — alphabetical order matches intended execution order.

- [x] **Step 3: Commit**

```bash
git add backend/crates/sim-server/migrations/202605160001_chunk_recovery.sql
git commit -m "feat: migrate world_events for chunk-aware recovery"
```

---

## Task 6: Postgres Adapter For New Trait Methods

**Files:**
- Modify: `backend/crates/sim-server/src/postgres_events.rs`

- [x] **Step 1: Read the existing `append` implementation**

Run:

```bash
grep -n "fn append\|INSERT INTO world_events\|sqlx::query" backend/crates/sim-server/src/postgres_events.rs
```

Expected: locates the existing INSERT statement for events. Note its column order.

- [x] **Step 2: Update the INSERT to include the new columns and ON CONFLICT clause**

Change the existing INSERT in `append` to include `chunk_x`, `chunk_y`, `chunk_version` and to use `ON CONFLICT (world_id, command_id) DO NOTHING`:

```rust
let rows_affected = sqlx::query(
    "INSERT INTO world_events
        (event_id, world_id, command_id, event_type, tick, version,
         chunk_x, chunk_y, chunk_version, payload)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
     ON CONFLICT (world_id, command_id) DO NOTHING",
)
.bind(&metadata.event_id)
.bind(&metadata.world_id)
.bind(&metadata.command_id)
.bind(metadata.event_type)
.bind(metadata.tick as i64)
.bind(metadata.version as i64)
.bind(chunk_x)
.bind(chunk_y)
.bind(chunk_version as i64)
.bind(payload_json)
.execute(&self.pool)
.await
.map_err(|err| WorldEventStoreError::unavailable(err.to_string()))?
.rows_affected();

if rows_affected == 0 {
    return Err(WorldEventStoreError::duplicate_command(&metadata.command_id));
}
```

Extract `chunk_x`, `chunk_y`, `chunk_version` from the event:

```rust
let (chunk_x, chunk_y, chunk_version) = match &event {
    WorldEventDto::TileKindSet(payload) => (payload.coord.x, payload.coord.y, payload.version),
};
```

- [x] **Step 3: Implement `find_event_by_command`**

Add:

```rust
async fn find_event_by_command(
    &self,
    world_id: &str,
    command_id: &str,
) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
    let row: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM world_events
          WHERE world_id = $1 AND command_id = $2
          LIMIT 1",
    )
    .bind(world_id)
    .bind(command_id)
    .fetch_optional(&self.pool)
    .await
    .map_err(|err| WorldEventStoreError::unavailable(err.to_string()))?;

    match row {
        None => Ok(None),
        Some((payload,)) => {
            let event: WorldEventDto = serde_json::from_value(payload)
                .map_err(|err| WorldEventStoreError::unavailable(err.to_string()))?;
            Ok(Some(event))
        }
    }
}
```

- [x] **Step 4: Implement `read_chunk_events_since`**

```rust
async fn read_chunk_events_since(
    &self,
    world_id: &str,
    coord: abutown_protocol::ChunkCoordDto,
    after_chunk_version: u64,
) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM world_events
          WHERE world_id = $1 AND chunk_x = $2 AND chunk_y = $3 AND chunk_version > $4
          ORDER BY chunk_version ASC",
    )
    .bind(world_id)
    .bind(coord.x)
    .bind(coord.y)
    .bind(after_chunk_version as i64)
    .fetch_all(&self.pool)
    .await
    .map_err(|err| WorldEventStoreError::unavailable(err.to_string()))?;

    rows.into_iter()
        .map(|(payload,)| {
            serde_json::from_value::<WorldEventDto>(payload)
                .map_err(|err| WorldEventStoreError::unavailable(err.to_string()))
        })
        .collect()
}
```

- [x] **Step 5: Implement `max_tick` and `max_version`**

```rust
async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT MAX(tick) FROM world_events WHERE world_id = $1",
    )
    .bind(world_id)
    .fetch_optional(&self.pool)
    .await
    .map_err(|err| WorldEventStoreError::unavailable(err.to_string()))?;
    Ok(row.and_then(|(opt,)| opt).map(|v| v as u64))
}

async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT MAX(version) FROM world_events WHERE world_id = $1",
    )
    .bind(world_id)
    .fetch_optional(&self.pool)
    .await
    .map_err(|err| WorldEventStoreError::unavailable(err.to_string()))?;
    Ok(row.and_then(|(opt,)| opt).map(|v| v as u64))
}
```

- [x] **Step 6: Verify and commit**

Run:

```bash
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: compiles.

Commit:

```bash
git add backend/crates/sim-server/src/postgres_events.rs
git commit -m "feat: postgres event store supports recovery queries"
```

---

## Task 7: `ChunkRegistry::insert_hydrated` and Smarter Snapshot Trigger

**Files:**
- Modify: `backend/crates/sim-server/src/chunk_registry.rs`

- [x] **Step 1: Read the existing registry shape**

Run:

```bash
grep -n "struct LoadedChunk\|insert_chunk\|fn collect_snapshots\|fn mark_snapshots_persisted" backend/crates/sim-server/src/chunk_registry.rs
```

Expected: locates the inner per-chunk record. Note its field names.

- [x] **Step 2: Add `last_persisted_version` and `last_snapshot_at` to the loaded chunk struct**

In the struct that holds a loaded chunk (likely `LoadedChunk` or similar), add fields:

```rust
last_persisted_version: u64,
last_snapshot_at: std::time::Instant,
```

Default them to `0` and `Instant::now()` on insert.

- [x] **Step 3: Write a failing test for the trigger logic**

Add to the registry test module:

```rust
#[test]
fn collect_snapshots_skips_chunks_with_no_new_events_within_snapshot_ceiling() {
    let mut registry = ChunkRegistry::new(32);
    let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    chunk.set_tile_kind(0, TileKind::Road).unwrap();
    registry.insert_chunk(chunk, ChunkActivity::Active);

    let world_id = WorldId("abutown-main".to_string());

    let first = registry.collect_snapshots(&world_id);
    assert_eq!(first.len(), 1, "first call must include the dirty chunk");
    let coords: Vec<ChunkCoord> = first.iter().map(|s| ChunkCoord { x: s.coord.x, y: s.coord.y }).collect();
    registry.mark_snapshots_persisted(&coords);

    let second = registry.collect_snapshots(&world_id);
    assert!(second.is_empty(), "second call without new events and within 30s must produce no snapshots");
}

#[test]
fn collect_snapshots_emits_again_after_new_event() {
    let mut registry = ChunkRegistry::new(32);
    let chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    registry.insert_chunk(chunk, ChunkActivity::Active);

    let world_id = WorldId("abutown-main".to_string());
    let coords: Vec<ChunkCoord> = registry
        .collect_snapshots(&world_id)
        .iter()
        .map(|s| ChunkCoord { x: s.coord.x, y: s.coord.y })
        .collect();
    registry.mark_snapshots_persisted(&coords);

    // Simulate an event arriving by mutating directly through the registry helper used in production.
    registry.set_tile_kind(ChunkCoord { x: 4, y: 4 }, 5, TileKind::Water).unwrap();

    let next = registry.collect_snapshots(&world_id);
    assert_eq!(next.len(), 1, "new event must produce a new snapshot candidate");
}
```

Note: if the registry exposes a different mutation entry point than `set_tile_kind`, adjust the second test to use whatever the current API offers (search the file for the mutation helper).

- [x] **Step 4: Run tests to verify they fail**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server collect_snapshots_skips collect_snapshots_emits_again
```

Expected: FAIL — `collect_snapshots` today always returns every chunk regardless of state.

- [x] **Step 5: Update `collect_snapshots`**

Change it to filter:

```rust
pub(crate) fn collect_snapshots(&self, world_id: &WorldId) -> Vec<ChunkSnapshotDto> {
    let ceiling = std::time::Duration::from_secs(30);
    let now = std::time::Instant::now();
    self.chunks
        .values()
        .filter(|loaded| {
            loaded.chunk.version() > loaded.last_persisted_version
                || now.duration_since(loaded.last_snapshot_at) >= ceiling
        })
        .filter_map(|loaded| {
            Some(build_chunk_snapshot(
                world_id.0.clone(),
                &loaded.chunk,
                loaded.activity,
            ))
        })
        .collect()
}
```

Field names (`self.chunks`, `loaded.chunk`, `loaded.activity`) must match what is in the file — adjust if different.

- [x] **Step 6: Update `mark_snapshots_persisted`**

```rust
pub(crate) fn mark_snapshots_persisted(&mut self, coords: &[ChunkCoord]) {
    let now = std::time::Instant::now();
    for coord in coords {
        if let Some(loaded) = self.chunks.get_mut(coord) {
            loaded.last_persisted_version = loaded.chunk.version();
            loaded.last_snapshot_at = now;
            loaded.chunk.clear_dirty();
        }
    }
}
```

- [x] **Step 7: Add `insert_hydrated`**

```rust
pub(crate) fn insert_hydrated(
    &mut self,
    chunk: Chunk,
    last_persisted_version: u64,
    activity: ChunkActivity,
) {
    let coord = chunk.coord();
    self.chunks.insert(
        coord,
        LoadedChunk {
            chunk,
            activity,
            last_persisted_version,
            last_snapshot_at: std::time::Instant::now(),
        },
    );
}
```

(Adjust `LoadedChunk` field list to match whatever the struct actually has.)

- [x] **Step 8: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server chunk_registry
```

Expected: all chunk_registry tests pass including the two new ones.

Commit:

```bash
git add backend/crates/sim-server/src/chunk_registry.rs
git commit -m "feat: snapshot only chunks with new events or beyond ceiling"
```

---

## Task 8: `SimulationRuntime::hydrate_from_stores`

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Write a failing unit test for hydration**

Add to `runtime.rs` test module:

```rust
#[tokio::test]
async fn hydrate_from_stores_restores_chunk_from_snapshot_and_replays_tail_events() {
    use abutown_protocol::{ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId};
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, build_chunk_snapshot};
    use sim_core::scheduler::ChunkActivity;

    // Seed: a chunk with tile 0 = Road at version 1, snapshotted.
    let mut authoring_chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
    authoring_chunk.set_tile_kind(0, TileKind::Road).unwrap();
    let snapshot = build_chunk_snapshot("abutown-main", &authoring_chunk, ChunkActivity::Active);

    let mut snapshot_store = InMemoryChunkSnapshotStore::default();
    snapshot_store.write_snapshot(snapshot);

    // Tail event after the snapshot: tile 7 = Water at chunk_version 2.
    let tail_event = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id: "event:tail".to_string(),
        command_id: "command:tail".to_string(),
        world_id: WorldId("abutown-main".to_string()),
        tick: 2,
        version: 2,
        coord: ChunkCoordDto { x: 4, y: 4 },
        local_index: 7,
        kind: TileKindDto::Water,
    });
    let mut event_store = InMemoryWorldEventStore::default();
    event_store.append(tail_event.clone());

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(event_store),
        Box::new(snapshot_store),
    )
    .await
    .unwrap();

    let restored = runtime.chunk_snapshot(ChunkCoord { x: 4, y: 4 }).unwrap();
    assert_eq!(restored.chunk_version, 2);
    let kinds: std::collections::HashMap<u16, TileKindDto> =
        restored.tiles.iter().map(|t| (t.local_index, t.kind)).collect();
    assert_eq!(kinds.get(&0), Some(&TileKindDto::Road));
    assert_eq!(kinds.get(&7), Some(&TileKindDto::Water));
}

#[tokio::test]
async fn hydrate_from_stores_falls_back_to_seed_when_no_snapshot() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::persistence::InMemoryChunkSnapshotStore;

    let runtime = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
    )
    .await
    .unwrap();

    let snap = runtime.chunk_snapshot(ChunkCoord { x: 4, y: 4 }).unwrap();
    assert_eq!(snap.chunk_version, 1, "seeded chunk has one tile mutation by default");
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server hydrate_from_stores
```

Expected: FAIL — `hydrate_from_stores` does not exist.

- [x] **Step 3: Implement `hydrate_from_stores`**

In `backend/crates/sim-server/src/runtime.rs`, add a new async constructor. Place it near `new_with_stores`:

```rust
pub async fn hydrate_from_stores(
    event_store: Box<dyn WorldEventStore + Send>,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
) -> Result<Self, HydrationError> {
    use abutown_protocol::ChunkCoordDto;

    let world_id = Self::default_world_id();
    let mut registry = ChunkRegistry::new(CHUNK_SIZE);

    for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate() {
        let snap = snapshot_store
            .read_snapshot(coord)
            .await
            .map_err(HydrationError::Snapshot)?;

        let (mut chunk, mut chunk_version) = match snap {
            Some(snapshot) => {
                let version = snapshot.chunk_version;
                let chunk = Chunk::from_snapshot(&snapshot).map_err(HydrationError::Decode)?;
                (chunk, version)
            }
            None => {
                let mut chunk = Chunk::new(coord, CHUNK_SIZE);
                let seed_index = (offset as u16) * 17;
                let seed_kind = match offset {
                    0 => TileKind::Road,
                    1 => TileKind::Water,
                    _ => TileKind::BuildingFootprint,
                };
                chunk
                    .set_tile_kind(seed_index, seed_kind)
                    .map_err(HydrationError::Chunk)?;
                let v = chunk.version();
                (chunk, v)
            }
        };

        let events = event_store
            .read_chunk_events_since(
                &world_id.0,
                ChunkCoordDto { x: coord.x, y: coord.y },
                chunk_version,
            )
            .await
            .map_err(HydrationError::Events)?;

        for event in &events {
            let next_version = chunk_version + 1;
            chunk.apply_event(event, next_version).map_err(HydrationError::Apply)?;
            chunk_version = next_version;
        }

        let activity = if offset == 0 {
            ChunkActivity::Active
        } else {
            ChunkActivity::Warm
        };
        registry.insert_hydrated(chunk, chunk_version, activity);
    }

    let global_tick = event_store
        .max_tick(&world_id.0)
        .await
        .map_err(HydrationError::Events)?
        .unwrap_or(0);
    let global_version = event_store
        .max_version(&world_id.0)
        .await
        .map_err(HydrationError::Events)?
        .unwrap_or(0);
    let event_count = global_version as usize;

    Ok(Self {
        world_id,
        registry,
        mobility: MobilityWorld::default(),
        snapshot_store,
        event_store,
        event_count,
        tick: global_tick,
        version: global_version,
    })
}
```

Add the error enum above `impl SimulationRuntime`:

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
}
```

Make `ChunkError`, `SnapshotDecodeError`, `EventApplyError` `pub` in `sim_core::chunk` if not already.

- [x] **Step 4: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server hydrate_from_stores
```

Expected: both tests pass.

Commit:

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-core/src/chunk.rs
git commit -m "feat: hydrate simulation runtime from stores"
```

---

## Task 9: Command Idempotency In `handle_command`

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Locate the command-handling entry point**

Run:

```bash
grep -n "pub async fn handle_command\|pub fn apply_set_tile_kind\|append.*event\|AppliedCommand" backend/crates/sim-server/src/runtime.rs | head -10
```

Expected: identifies the function that today applies a command and writes an event. Note the function name (referred to below as `handle_command` — adjust if the actual name differs).

- [x] **Step 2: Write a failing test for duplicate command handling**

Add to the runtime test module:

```rust
#[tokio::test]
async fn duplicate_command_id_is_idempotent_and_writes_only_one_event() {
    use abutown_protocol::{ClientCommandDto, SetTileKindCommandDto, ChunkCoordDto, TileKindDto, WorldId, PROTOCOL_VERSION};

    let mut runtime = SimulationRuntime::new();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:dup".to_string(),
        coord: ChunkCoordDto { x: 4, y: 4 },
        local_index: 12,
        kind: TileKindDto::Water,
    });

    let first = runtime.handle_command(command.clone()).await.unwrap();
    let second = runtime.handle_command(command.clone()).await.unwrap();

    assert_eq!(first, second, "duplicate command must return identical response");
    assert_eq!(runtime.event_count(), 1, "only one event must be appended");
}
```

Adjust `handle_command` signature in the test to match what the actual method is named.

- [x] **Step 3: Run test to verify it fails**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server duplicate_command_id_is_idempotent
```

Expected: FAIL — either two events get appended (if the in-memory store accepts duplicates) or the function panics on the unique-constraint error.

- [x] **Step 4: Implement pre-flight dedup + clone-then-commit**

In `handle_command` (or the equivalent), restructure to:

```rust
pub async fn handle_command(
    &mut self,
    command: ClientCommandDto,
) -> Result<CommandAcceptedDto, CommandRejection> {
    let (world_id, command_id) = match &command {
        ClientCommandDto::SetTileKind(c) => (c.world_id.0.clone(), c.command_id.clone()),
    };

    // Pre-flight dedup.
    match self.event_store.find_event_by_command(&world_id, &command_id).await {
        Ok(Some(existing)) => {
            return Ok(self.accepted_dto_from_event(&existing));
        }
        Ok(None) => {}
        Err(err) => {
            return Err(CommandRejection {
                world_id: Some(WorldId(world_id)),
                command_id: Some(command_id),
                code: "event_store_unavailable",
                message: err.to_string(),
            });
        }
    }

    // Build the prospective event + mutated chunk on a clone.
    let applied = self.build_applied_command(command.clone())?;

    // Append; on UNIQUE conflict the store returns DuplicateCommand — re-fetch the winner.
    match self.event_store.append(applied.event.clone()).await {
        Ok(()) => {
            self.commit_applied(applied.clone());
            Ok(applied.response)
        }
        Err(err) if err.code() == "duplicate_command_id" => {
            let existing = self
                .event_store
                .find_event_by_command(&world_id, &command_id)
                .await
                .map_err(|err| CommandRejection {
                    world_id: Some(WorldId(world_id.clone())),
                    command_id: Some(command_id.clone()),
                    code: "event_store_unavailable",
                    message: err.to_string(),
                })?
                .ok_or_else(|| CommandRejection {
                    world_id: Some(WorldId(world_id.clone())),
                    command_id: Some(command_id.clone()),
                    code: "event_store_inconsistent",
                    message: "duplicate command_id reported but lookup returned none".to_string(),
                })?;
            Ok(self.accepted_dto_from_event(&existing))
        }
        Err(err) => Err(CommandRejection {
            world_id: Some(WorldId(world_id)),
            command_id: Some(command_id),
            code: err.code(),
            message: err.to_string(),
        }),
    }
}
```

`build_applied_command` returns an `AppliedCommand` (existing type) but does NOT mutate `self.registry` — it computes the new chunk state on a clone of the affected chunk. `commit_applied` swaps that clone into the registry, increments `event_count`, `tick`, `version`.

`accepted_dto_from_event(event)` reconstructs a `CommandAcceptedDto` from an existing event row (matches the original `AppliedCommand::response` shape: `protocol_version`, `world_id`, `command_id`, and event metadata).

If `build_applied_command`, `commit_applied`, and `accepted_dto_from_event` do not yet exist, factor them out of the current `handle_command` body. The current code combines compute + mutate + append in one pass; this task splits the three.

- [x] **Step 5: Verify and commit**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server runtime
```

Expected: all runtime tests pass including the new dedup test.

Commit:

```bash
git add backend/crates/sim-server/src/runtime.rs
git commit -m "feat: dedupe duplicate command_id at command intake"
```

---

## Task 10: Wire Hydration In `build_app_from_config`

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

- [x] **Step 1: Locate the current branch in `build_app_from_config`**

Run:

```bash
grep -n "build_app_from_config\|new_with_event_store\|new_with_stores\|PostgresWorldEventStore::connect\|PostgresChunkSnapshotStore::connect" backend/crates/sim-server/src/app.rs
```

Expected: identifies the branch that constructs the runtime when `database_url` is present.

- [x] **Step 2: Replace `new_with_stores` with `hydrate_from_stores` on the postgres branch**

Change the relevant block to:

```rust
let event_store = PostgresWorldEventStore::connect(database_url).await?;
let snapshot_store = PostgresChunkSnapshotStore::connect(database_url).await?;
let card_hands = CardHandStore::postgres(database_url).await?;
let auth = match &config.supabase_url {
    url => AuthVerifier::supabase(url).await,
};
let runtime =
    SimulationRuntime::hydrate_from_stores(Box::new(event_store), Box::new(snapshot_store))
        .await?;
return Ok(build_app_with_runtime_and_card_hands(runtime, card_hands, auth));
```

Note: `HydrationError` must satisfy `Into<anyhow::Error>` (derive via thiserror with `#[error]` and let `anyhow` auto-wrap). The `?` operator on `hydrate_from_stores(...).await?` then works.

The in-memory branch (no `database_url`) still uses `build_app()` which internally calls `SimulationRuntime::new()`.

- [x] **Step 3: Verify build**

Run:

```bash
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: compiles.

- [x] **Step 4: Commit**

```bash
git add backend/crates/sim-server/src/app.rs
git commit -m "feat: hydrate runtime from stores on startup"
```

---

## Task 11: Integration Tests (Opt-In, Postgres)

**Files:**
- Modify: `backend/crates/sim-server/tests/http.rs`

- [x] **Step 1: Write a recovery integration test**

Add a `#[tokio::test]` gated on `ABUTOWN_TEST_DATABASE_URL`:

```rust
#[tokio::test]
async fn world_state_survives_runtime_restart() {
    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
        return;
    };

    // First runtime: mutate a tile via POST /commands.
    let app1 = build_test_app_with_postgres(&database_url).await;
    let response = app1
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"type":"set_tile_kind","protocol_version":1,"world_id":"abutown-main","command_id":"command:recover-test:1","coord":{"x":4,"y":4},"local_index":21,"kind":"water"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Force a snapshot write before tearing down.
    persist_snapshots_once(app1.clone()).await.unwrap();
    drop(app1);

    // Second runtime: same database, fresh in-memory state.
    let app2 = build_test_app_with_postgres(&database_url).await;
    let response = app2
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let tiles = snapshot["tiles"].as_array().unwrap();
    assert!(
        tiles.iter().any(|t| t["local_index"] == 21 && t["kind"] == "water"),
        "post-restart snapshot must contain the tile that was set before restart: {tiles:?}"
    );
}
```

`build_test_app_with_postgres` and `persist_snapshots_once` already exist in the test file (or are exposed from `sim_server::app`). Reuse what's there.

- [x] **Step 2: Write a duplicate-command integration test**

```rust
#[tokio::test]
async fn duplicate_command_returns_same_response_and_writes_one_event() {
    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
        return;
    };

    let app = build_test_app_with_postgres(&database_url).await;
    let unique = format!("command:dup-test:{}", uuid::Uuid::new_v4());
    let body_template = format!(
        r#"{{"type":"set_tile_kind","protocol_version":1,"world_id":"abutown-main","command_id":"{unique}","coord":{{"x":4,"y":4}},"local_index":29,"kind":"road"}}"#
    );

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(body_template.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    let first_status = first.status();
    let first_body = hyper::body::to_bytes(first.into_body()).await.unwrap();

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(body_template))
                .unwrap(),
        )
        .await
        .unwrap();
    let second_status = second.status();
    let second_body = hyper::body::to_bytes(second.into_body()).await.unwrap();

    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(first_body, second_body, "duplicate command must return identical body");

    // Optional: verify exactly one row landed in world_events via direct sqlx query.
}
```

- [x] **Step 3: Verify all existing tests still pass with in-memory stores**

Run:

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green. Integration tests gated on `ABUTOWN_TEST_DATABASE_URL` silently skip if the var is unset.

- [x] **Step 4: If a Postgres dev DB is available, run with the var set**

If you have a database URL, run:

```bash
ABUTOWN_TEST_DATABASE_URL="$DATABASE_URL" cargo test --locked --manifest-path backend/Cargo.toml -p sim-server world_state_survives duplicate_command_returns_same_response
```

Expected: both integration tests pass.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-server/tests/http.rs
git commit -m "test: cover postgres recovery and command idempotency end-to-end"
```

---

## Task 12: Final Quality Gate

**Files:** none modified — verification only.

- [x] **Step 1: Run formatter, full test suite, clippy**

Run in sequence:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: all three succeed.

- [x] **Step 2: Update progress.md**

Append one line to `progress.md`:

```
2026-05-15T<HH:MM:SS>.000Z - Chunk recovery: switched snapshots to full-state sparse encoding, added chunk-version-aware event replay, command_id idempotency via unique constraint, and runtime hydration on startup.
```

Use the current UTC timestamp. Commit:

```bash
git add progress.md
git commit -m "docs: record chunk recovery progress"
```

---

## Self-Review

- **Spec coverage:**
  - Schema Changes → Task 5.
  - Recovery Flow → Tasks 2, 3, 4, 6, 8.
  - Command Path with Idempotency → Tasks 4, 6, 9.
  - Snapshot Trigger → Task 7.
  - Public API Changes → Tasks 4, 8, 10.
  - Testing Strategy → Tasks 1–9 unit tests, Task 11 integration.
  - Snapshot format change (delta → full state) → Task 1.
- **Placeholder scan:** no TBD/TODO; every code step has a code block; commit messages are inline.
- **Type consistency:** `Chunk::from_snapshot(&snap)`, `Chunk::apply_event(&event, chunk_version)`, `hydrate_from_stores(event_store, snapshot_store)`, `WorldEventStore::find_event_by_command(world_id, command_id)`, `WorldEventStore::read_chunk_events_since(world_id, coord, after_chunk_version)` — used consistently across tasks.
- **Scope check:** Mobility, Player aggregates, expected_chunk_version, lazy hydration, snapshot compaction all explicitly out — matches spec.
