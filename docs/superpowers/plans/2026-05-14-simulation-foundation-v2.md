# Simulation Foundation v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first Rust backend foundation slice for Abutown's single persistent aquarium world: protocol types, chunk-local tile arrays, ECS materialization, dirty snapshots, and a small read-only server surface.

**Architecture:** Create a Rust workspace under `backend/` with `protocol`, `sim-core`, and `sim-server`. Keep tiles in dense chunk-local arrays with dirty bitsets; use `bevy_ecs` only for materialized dynamic entities. Persistence is an in-memory adapter in this slice so the hot loop and durable boundary are shaped before Supabase/Postgres is introduced.

**Tech Stack:** Rust 2024 edition, `bevy_ecs`, Tokio, Axum, Serde, UUID, Time, Thiserror, Anyhow, Tower HTTP.

---

## Scope Boundary

This implements only the architecture foundation from `docs/superpowers/specs/2026-05-14-abutown-simulation-architecture-v2-design.md`.

It does not implement economy, ledger, citizens, production, auth, Supabase migrations, player commands, or frontend integration.

## File Structure

- Create `backend/Cargo.toml`: Rust workspace and shared dependency versions.
- Create `backend/README.md`: backend run/test commands and architecture notes.
- Create `backend/crates/protocol/Cargo.toml`: protocol crate package.
- Create `backend/crates/protocol/src/lib.rs`: versioned DTOs for world, chunks, tiles, snapshots, and server health.
- Create `backend/crates/sim-core/Cargo.toml`: pure simulation core package.
- Create `backend/crates/sim-core/src/lib.rs`: public module exports.
- Create `backend/crates/sim-core/src/ids.rs`: stable world/chunk/entity IDs and chunk coordinate helpers.
- Create `backend/crates/sim-core/src/tile.rs`: compact tile kind/property structs.
- Create `backend/crates/sim-core/src/chunk.rs`: dense chunk-local tile arrays, version counters, and dirty tracking.
- Create `backend/crates/sim-core/src/ecs_runtime.rs`: `bevy_ecs` materialization layer for dynamic objects.
- Create `backend/crates/sim-core/src/scheduler.rs`: chunk activity state and LOD policy.
- Create `backend/crates/sim-core/src/persistence.rs`: in-memory chunk snapshot adapter shape.
- Create `backend/crates/sim-server/Cargo.toml`: Axum server package.
- Create `backend/crates/sim-server/src/main.rs`: binary entrypoint.
- Create `backend/crates/sim-server/src/app.rs`: app state, `/health`, `/world`, and `/chunks/:x/:y` endpoints.
- Create `backend/crates/sim-server/tests/http.rs`: server integration tests.

### Task 1: Rust Workspace And Protocol DTOs

**Files:**
- Create: `backend/Cargo.toml`
- Create: `backend/README.md`
- Create: `backend/crates/protocol/Cargo.toml`
- Create: `backend/crates/protocol/src/lib.rs`

- [ ] **Step 1: Write the failing protocol test**

Create `backend/crates/protocol/src/lib.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_serializes_versioned_world() {
        let response = HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        };

        let json = serde_json::to_string(&response).expect("health response serializes");

        assert_eq!(
            json,
            r#"{"service":"abutown-sim","world_id":"abutown-main","ok":true,"protocol_version":1}"#
        );
    }
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol health_response_serializes_versioned_world
```

Expected: FAIL because the workspace and DTOs are not implemented.

- [ ] **Step 3: Add the workspace and protocol implementation**

Create `backend/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
  "crates/protocol",
  "crates/sim-core",
  "crates/sim-server",
]

[workspace.package]
edition = "2024"
license = "UNLICENSED"
publish = false

[workspace.dependencies]
anyhow = "1"
axum = { version = "0.8", features = ["ws"] }
bevy_ecs = "0.18"
http-body-util = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
time = { version = "0.3", features = ["serde", "formatting", "macros"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "sync", "time", "net"] }
tower = { version = "0.5", features = ["util"] }
tower-http = { version = "0.6", features = ["trace", "cors", "compression-full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
uuid = { version = "1", features = ["serde", "v7"] }
```

Create `backend/README.md`:

````markdown
# Abutown Backend

Rust authoritative simulation foundation for the single always-on `abutown-main` aquarium world.

Common commands:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
cargo fmt --manifest-path backend/Cargo.toml --all
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

Design rules:

- Rust owns hot simulation state.
- Tiles are durable but live as dense chunk arrays in memory.
- ECS is for materialized dynamic entities, not every tile.
- Database writes stay outside fixed-tick hot paths.
````

Create `backend/crates/protocol/Cargo.toml`:

```toml
[package]
name = "abutown-protocol"
version = "0.1.0"
edition.workspace = true
publish.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
time.workspace = true
uuid.workspace = true

[dev-dependencies]
serde_json.workspace = true
```

Replace `backend/crates/protocol/src/lib.rs` with:

```rust
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoordDto {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStateDto {
    Asleep,
    Warm,
    Active,
    Hot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileKindDto {
    Grass,
    Water,
    Road,
    BuildingFootprint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub service: String,
    pub world_id: WorldId,
    pub ok: bool,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSummaryDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
    pub loaded_chunks: Vec<ChunkCoordDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileMutationDto {
    pub local_index: u16,
    pub kind: TileKindDto,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkSnapshotDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub coord: ChunkCoordDto,
    pub chunk_state: ChunkStateDto,
    pub chunk_version: u64,
    pub tile_count: u16,
    pub dirty_tiles: Vec<TileMutationDto>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_serializes_versioned_world() {
        let response = HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        };

        let json = serde_json::to_string(&response).expect("health response serializes");

        assert_eq!(
            json,
            r#"{"service":"abutown-sim","world_id":"abutown-main","ok":true,"protocol_version":1}"#
        );
    }
}
```

- [ ] **Step 4: Verify protocol tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/Cargo.toml backend/README.md backend/crates/protocol/Cargo.toml backend/crates/protocol/src/lib.rs
git commit -m "feat: add backend protocol workspace"
```

### Task 2: Dense Chunk Tile Storage

**Files:**
- Create: `backend/crates/sim-core/Cargo.toml`
- Create: `backend/crates/sim-core/src/lib.rs`
- Create: `backend/crates/sim-core/src/ids.rs`
- Create: `backend/crates/sim-core/src/tile.rs`
- Create: `backend/crates/sim-core/src/chunk.rs`

- [ ] **Step 1: Write failing chunk tests**

Create `backend/crates/sim-core/src/chunk.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::TileKind;

    #[test]
    fn chunk_uses_dense_tiles_and_tracks_dirty_indices() {
        let mut chunk = Chunk::new(ChunkCoord { x: 2, y: -1 }, 32);

        assert_eq!(chunk.tile_count(), 1024);
        assert_eq!(chunk.dirty_indices(), Vec::<u16>::new());

        chunk.set_tile_kind(0, TileKind::Water).expect("index 0 exists");
        chunk.set_tile_kind(17, TileKind::Road).expect("index 17 exists");

        assert_eq!(chunk.kind_at(0), Some(TileKind::Water));
        assert_eq!(chunk.kind_at(17), Some(TileKind::Road));
        assert_eq!(chunk.version(), 2);
        assert_eq!(chunk.dirty_indices(), vec![0, 17]);
    }
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core chunk_uses_dense_tiles_and_tracks_dirty_indices
```

Expected: FAIL because `sim-core`, `Chunk`, `ChunkCoord`, and `TileKind` are missing.

- [ ] **Step 3: Implement sim-core IDs, tiles, and chunk arrays**

Create `backend/crates/sim-core/Cargo.toml`:

```toml
[package]
name = "sim-core"
version = "0.1.0"
edition.workspace = true
publish.workspace = true
license.workspace = true

[dependencies]
abutown-protocol = { path = "../protocol" }
bevy_ecs.workspace = true
serde.workspace = true
thiserror.workspace = true
uuid.workspace = true
```

Create `backend/crates/sim-core/src/lib.rs`:

```rust
pub mod chunk;
pub mod ecs_runtime;
pub mod ids;
pub mod persistence;
pub mod scheduler;
pub mod tile;
```

Create `backend/crates/sim-core/src/ids.rs`:

```rust
use abutown_protocol::ChunkCoordDto;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

impl From<ChunkCoord> for ChunkCoordDto {
    fn from(value: ChunkCoord) -> Self {
        Self { x: value.x, y: value.y }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StableEntityId(pub String);
```

Create `backend/crates/sim-core/src/tile.rs`:

```rust
use abutown_protocol::TileKindDto;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileKind {
    Grass,
    Water,
    Road,
    BuildingFootprint,
}

impl Default for TileKind {
    fn default() -> Self {
        Self::Grass
    }
}

impl From<TileKind> for TileKindDto {
    fn from(value: TileKind) -> Self {
        match value {
            TileKind::Grass => Self::Grass,
            TileKind::Water => Self::Water,
            TileKind::Road => Self::Road,
            TileKind::BuildingFootprint => Self::BuildingFootprint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TileFlags {
    pub blocks_movement: bool,
    pub modified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TileRecord {
    pub kind: TileKind,
    pub flags: TileFlags,
    pub version: u64,
}
```

Replace `backend/crates/sim-core/src/chunk.rs` with:

```rust
use std::collections::BTreeSet;

use thiserror::Error;

use crate::ids::ChunkCoord;
use crate::tile::{TileKind, TileRecord};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkError {
    #[error("tile index {index} is outside chunk tile count {tile_count}")]
    IndexOutOfBounds { index: u16, tile_count: u16 },
}

#[derive(Debug, Clone)]
pub struct Chunk {
    coord: ChunkCoord,
    chunk_size: u16,
    version: u64,
    tiles: Vec<TileRecord>,
    dirty: BTreeSet<u16>,
}

impl Chunk {
    pub fn new(coord: ChunkCoord, chunk_size: u16) -> Self {
        let tile_count = usize::from(chunk_size) * usize::from(chunk_size);
        Self {
            coord,
            chunk_size,
            version: 0,
            tiles: vec![TileRecord::default(); tile_count],
            dirty: BTreeSet::new(),
        }
    }

    pub fn coord(&self) -> ChunkCoord {
        self.coord
    }

    pub fn chunk_size(&self) -> u16 {
        self.chunk_size
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn tile_count(&self) -> u16 {
        self.tiles.len() as u16
    }

    pub fn kind_at(&self, index: u16) -> Option<TileKind> {
        self.tiles.get(usize::from(index)).map(|tile| tile.kind)
    }

    pub fn tile_at(&self, index: u16) -> Option<TileRecord> {
        self.tiles.get(usize::from(index)).copied()
    }

    pub fn dirty_indices(&self) -> Vec<u16> {
        self.dirty.iter().copied().collect()
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub fn set_tile_kind(&mut self, index: u16, kind: TileKind) -> Result<(), ChunkError> {
        let tile_count = self.tile_count();
        let tile = self
            .tiles
            .get_mut(usize::from(index))
            .ok_or(ChunkError::IndexOutOfBounds { index, tile_count })?;

        if tile.kind != kind {
            self.version += 1;
            tile.kind = kind;
            tile.version = self.version;
            tile.flags.modified = true;
            self.dirty.insert(index);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::TileKind;

    #[test]
    fn chunk_uses_dense_tiles_and_tracks_dirty_indices() {
        let mut chunk = Chunk::new(ChunkCoord { x: 2, y: -1 }, 32);

        assert_eq!(chunk.tile_count(), 1024);
        assert_eq!(chunk.dirty_indices(), Vec::<u16>::new());

        chunk.set_tile_kind(0, TileKind::Water).expect("index 0 exists");
        chunk.set_tile_kind(17, TileKind::Road).expect("index 17 exists");

        assert_eq!(chunk.kind_at(0), Some(TileKind::Water));
        assert_eq!(chunk.kind_at(17), Some(TileKind::Road));
        assert_eq!(chunk.version(), 2);
        assert_eq!(chunk.dirty_indices(), vec![0, 17]);
    }
}
```

- [ ] **Step 4: Verify chunk tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core chunk_uses_dense_tiles_and_tracks_dirty_indices
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/Cargo.toml backend/crates/sim-core/src/lib.rs backend/crates/sim-core/src/ids.rs backend/crates/sim-core/src/tile.rs backend/crates/sim-core/src/chunk.rs
git commit -m "feat: add dense chunk tile storage"
```

### Task 3: ECS Materialization Layer

**Files:**
- Create: `backend/crates/sim-core/src/ecs_runtime.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [ ] **Step 1: Write failing ECS runtime tests**

Create `backend/crates/sim-core/src/ecs_runtime.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ChunkCoord, StableEntityId};

    #[test]
    fn materialized_entities_keep_stable_ids_outside_ecs_indices() {
        let mut runtime = MaterializedRuntime::default();
        let stable_id = StableEntityId("item:bench:0001".to_string());

        let entity = runtime.spawn_materialized(
            stable_id.clone(),
            ChunkCoord { x: 0, y: 0 },
            MaterializedKind::Item,
        );

        assert_eq!(runtime.lookup(&stable_id), Some(entity));
        assert_eq!(runtime.materialized_count(), 1);
    }
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core materialized_entities_keep_stable_ids_outside_ecs_indices
```

Expected: FAIL because the runtime types are missing.

- [ ] **Step 3: Implement materialized runtime**

Replace `backend/crates/sim-core/src/ecs_runtime.rs` with:

```rust
use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::ids::{ChunkCoord, StableEntityId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializedKind {
    Player,
    Item,
    Machine,
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct StableIdComponent(pub StableEntityId);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkLocationComponent(pub ChunkCoord);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializedKindComponent(pub MaterializedKind);

#[derive(Default)]
pub struct MaterializedRuntime {
    world: World,
    by_stable_id: HashMap<StableEntityId, Entity>,
}

impl MaterializedRuntime {
    pub fn spawn_materialized(
        &mut self,
        stable_id: StableEntityId,
        chunk: ChunkCoord,
        kind: MaterializedKind,
    ) -> Entity {
        if let Some(entity) = self.by_stable_id.get(&stable_id) {
            return *entity;
        }

        let entity = self
            .world
            .spawn((
                StableIdComponent(stable_id.clone()),
                ChunkLocationComponent(chunk),
                MaterializedKindComponent(kind),
            ))
            .id();

        self.by_stable_id.insert(stable_id, entity);
        entity
    }

    pub fn lookup(&self, stable_id: &StableEntityId) -> Option<Entity> {
        self.by_stable_id.get(stable_id).copied()
    }

    pub fn materialized_count(&self) -> usize {
        self.by_stable_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ChunkCoord, StableEntityId};

    #[test]
    fn materialized_entities_keep_stable_ids_outside_ecs_indices() {
        let mut runtime = MaterializedRuntime::default();
        let stable_id = StableEntityId("item:bench:0001".to_string());

        let entity = runtime.spawn_materialized(
            stable_id.clone(),
            ChunkCoord { x: 0, y: 0 },
            MaterializedKind::Item,
        );

        assert_eq!(runtime.lookup(&stable_id), Some(entity));
        assert_eq!(runtime.materialized_count(), 1);
    }
}
```

- [ ] **Step 4: Verify ECS runtime tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core materialized_entities_keep_stable_ids_outside_ecs_indices
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/ecs_runtime.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: add ECS materialization runtime"
```

### Task 4: Chunk Activity Scheduler

**Files:**
- Create: `backend/crates/sim-core/src/scheduler.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [ ] **Step 1: Write failing scheduler tests**

Create `backend/crates/sim-core/src/scheduler.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_activity_scales_with_players_and_mutation_pressure() {
        assert_eq!(classify_chunk_activity(0, 0), ChunkActivity::Asleep);
        assert_eq!(classify_chunk_activity(0, 4), ChunkActivity::Warm);
        assert_eq!(classify_chunk_activity(1, 0), ChunkActivity::Active);
        assert_eq!(classify_chunk_activity(80, 0), ChunkActivity::Hot);
        assert_eq!(classify_chunk_activity(3, 400), ChunkActivity::Hot);
    }
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core chunk_activity_scales_with_players_and_mutation_pressure
```

Expected: FAIL because `ChunkActivity` and `classify_chunk_activity` are missing.

- [ ] **Step 3: Implement activity classification**

Replace `backend/crates/sim-core/src/scheduler.rs` with:

```rust
use abutown_protocol::ChunkStateDto;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkActivity {
    Asleep,
    Warm,
    Active,
    Hot,
}

impl From<ChunkActivity> for ChunkStateDto {
    fn from(value: ChunkActivity) -> Self {
        match value {
            ChunkActivity::Asleep => Self::Asleep,
            ChunkActivity::Warm => Self::Warm,
            ChunkActivity::Active => Self::Active,
            ChunkActivity::Hot => Self::Hot,
        }
    }
}

pub fn classify_chunk_activity(player_count: u32, dirty_tile_pressure: u32) -> ChunkActivity {
    if player_count >= 64 || dirty_tile_pressure >= 256 {
        return ChunkActivity::Hot;
    }
    if player_count > 0 {
        return ChunkActivity::Active;
    }
    if dirty_tile_pressure > 0 {
        return ChunkActivity::Warm;
    }
    ChunkActivity::Asleep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_activity_scales_with_players_and_mutation_pressure() {
        assert_eq!(classify_chunk_activity(0, 0), ChunkActivity::Asleep);
        assert_eq!(classify_chunk_activity(0, 4), ChunkActivity::Warm);
        assert_eq!(classify_chunk_activity(1, 0), ChunkActivity::Active);
        assert_eq!(classify_chunk_activity(80, 0), ChunkActivity::Hot);
        assert_eq!(classify_chunk_activity(3, 400), ChunkActivity::Hot);
    }
}
```

- [ ] **Step 4: Verify scheduler tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core chunk_activity_scales_with_players_and_mutation_pressure
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/scheduler.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: add chunk activity scheduler"
```

### Task 5: Snapshot And Persistence Boundary

**Files:**
- Create: `backend/crates/sim-core/src/persistence.rs`
- Modify: `backend/crates/sim-core/src/chunk.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [ ] **Step 1: Write failing snapshot tests**

Create `backend/crates/sim-core/src/persistence.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use crate::ids::ChunkCoord;
    use crate::scheduler::ChunkActivity;
    use crate::tile::TileKind;

    #[test]
    fn snapshot_contains_only_dirty_tiles_then_clears_dirty_state() {
        let mut chunk = Chunk::new(ChunkCoord { x: 1, y: 2 }, 32);
        chunk.set_tile_kind(3, TileKind::Water).expect("tile exists");
        chunk.set_tile_kind(9, TileKind::Road).expect("tile exists");

        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        assert_eq!(snapshot.dirty_tiles.len(), 2);
        assert_eq!(snapshot.dirty_tiles[0].local_index, 3);
        assert_eq!(snapshot.dirty_tiles[1].local_index, 9);

        chunk.clear_dirty();
        assert!(chunk.dirty_indices().is_empty());
    }
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core snapshot_contains_only_dirty_tiles_then_clears_dirty_state
```

Expected: FAIL because snapshot functions are missing.

- [ ] **Step 3: Implement snapshot builder and in-memory store**

Replace `backend/crates/sim-core/src/persistence.rs` with:

```rust
use std::collections::HashMap;

use abutown_protocol::{
    ChunkSnapshotDto, PROTOCOL_VERSION, TileMutationDto, WorldId,
};

use crate::chunk::Chunk;
use crate::ids::ChunkCoord;
use crate::scheduler::ChunkActivity;

pub fn build_chunk_snapshot(
    world_id: impl Into<String>,
    chunk: &Chunk,
    activity: ChunkActivity,
) -> ChunkSnapshotDto {
    let dirty_tiles = chunk
        .dirty_indices()
        .into_iter()
        .filter_map(|index| {
            chunk.tile_at(index).map(|tile| TileMutationDto {
                local_index: index,
                kind: tile.kind.into(),
                version: tile.version,
            })
        })
        .collect();

    ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId(world_id.into()),
        coord: chunk.coord().into(),
        chunk_state: activity.into(),
        chunk_version: chunk.version(),
        tile_count: chunk.tile_count(),
        dirty_tiles,
    }
}

#[derive(Default)]
pub struct InMemoryChunkSnapshotStore {
    snapshots: HashMap<ChunkCoord, ChunkSnapshotDto>,
}

impl InMemoryChunkSnapshotStore {
    pub fn write_snapshot(&mut self, snapshot: ChunkSnapshotDto) {
        self.snapshots.insert(
            ChunkCoord {
                x: snapshot.coord.x,
                y: snapshot.coord.y,
            },
            snapshot,
        );
    }

    pub fn read_snapshot(&self, coord: ChunkCoord) -> Option<&ChunkSnapshotDto> {
        self.snapshots.get(&coord)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use crate::ids::ChunkCoord;
    use crate::scheduler::ChunkActivity;
    use crate::tile::TileKind;

    #[test]
    fn snapshot_contains_only_dirty_tiles_then_clears_dirty_state() {
        let mut chunk = Chunk::new(ChunkCoord { x: 1, y: 2 }, 32);
        chunk.set_tile_kind(3, TileKind::Water).expect("tile exists");
        chunk.set_tile_kind(9, TileKind::Road).expect("tile exists");

        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        assert_eq!(snapshot.dirty_tiles.len(), 2);
        assert_eq!(snapshot.dirty_tiles[0].local_index, 3);
        assert_eq!(snapshot.dirty_tiles[1].local_index, 9);

        chunk.clear_dirty();
        assert!(chunk.dirty_indices().is_empty());
    }
}
```

- [ ] **Step 4: Verify persistence tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core snapshot_contains_only_dirty_tiles_then_clears_dirty_state
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/persistence.rs backend/crates/sim-core/src/chunk.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: add dirty chunk snapshot boundary"
```

### Task 6: Read-Only Simulation Server

**Files:**
- Create: `backend/crates/sim-server/Cargo.toml`
- Create: `backend/crates/sim-server/src/main.rs`
- Create: `backend/crates/sim-server/src/app.rs`
- Create: `backend/crates/sim-server/tests/http.rs`

- [ ] **Step 1: Write failing HTTP integration tests**

Create `backend/crates/sim-server/tests/http.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use sim_server::app::build_app;

#[tokio::test]
async fn health_and_world_summary_are_available() {
    let app = build_app();

    let health_response = app
        .clone()
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health_response.status(), StatusCode::OK);

    let world_response = app
        .oneshot(Request::builder().uri("/world").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(world_response.status(), StatusCode::OK);

    let body = world_response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["chunk_size"], 32);
}
```

- [ ] **Step 2: Run the failing HTTP test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server health_and_world_summary_are_available
```

Expected: FAIL because `sim-server` is not implemented.

- [ ] **Step 3: Implement app and endpoints**

Create `backend/crates/sim-server/Cargo.toml`:

```toml
[package]
name = "sim-server"
version = "0.1.0"
edition.workspace = true
publish.workspace = true
license.workspace = true

[lib]
name = "sim_server"
path = "src/app.rs"

[[bin]]
name = "sim-server"
path = "src/main.rs"

[dependencies]
abutown-protocol = { path = "../protocol" }
sim-core = { path = "../sim-core" }
anyhow.workspace = true
axum.workspace = true
serde_json.workspace = true
tokio.workspace = true
tower-http.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true

[dev-dependencies]
http-body-util.workspace = true
tower.workspace = true
```

Create `backend/crates/sim-server/src/app.rs`:

```rust
use abutown_protocol::{
    ChunkCoordDto, HealthResponse, PROTOCOL_VERSION, WorldId, WorldSummaryDto,
};
use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    persistence::build_chunk_snapshot,
    scheduler::ChunkActivity,
    tile::TileKind,
};

pub fn build_app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/world", get(world))
        .route("/chunks/{x}/{y}", get(chunk))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        service: "abutown-sim".to_string(),
        world_id: WorldId("abutown-main".to_string()),
        ok: true,
        protocol_version: PROTOCOL_VERSION,
    })
}

async fn world() -> Json<WorldSummaryDto> {
    Json(WorldSummaryDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        chunk_size: 32,
        loaded_chunks: vec![ChunkCoordDto { x: 0, y: 0 }],
    })
}

async fn chunk(Path((x, y)): Path<(i32, i32)>) -> Result<Json<abutown_protocol::ChunkSnapshotDto>, StatusCode> {
    if x != 0 || y != 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    let mut chunk = Chunk::new(ChunkCoord { x, y }, 32);
    chunk.set_tile_kind(0, TileKind::Road).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

    Ok(Json(snapshot))
}
```

Create `backend/crates/sim-server/src/main.rs`:

```rust
use std::net::SocketAddr;

use anyhow::Context;
use sim_server::app::build_app;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: SocketAddr = "127.0.0.1:8080".parse().context("parse listen address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind simulation server")?;

    tracing::info!(%addr, "starting sim-server");
    axum::serve(listener, build_app())
        .await
        .context("run simulation server")
}
```

- [ ] **Step 4: Verify HTTP tests pass**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server health_and_world_summary_are_available
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/Cargo.toml backend/crates/sim-server/src/main.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: expose simulation foundation endpoints"
```

### Task 7: Workspace Verification

**Files:**
- Modify: `backend/README.md`

- [ ] **Step 1: Run full backend tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: PASS for `abutown-protocol`, `sim-core`, and `sim-server`.

- [ ] **Step 2: Run Rust formatting**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

Expected: PASS. If it fails, run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
```

Then rerun the check command.

- [ ] **Step 3: Run clippy**

Run:

```bash
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Confirm no frontend files were required**

Run:

```bash
git diff --name-only HEAD
```

Expected: only backend files are listed for this implementation slice before commit.

- [ ] **Step 5: Commit final verification updates**

If `cargo fmt` changed files or `backend/README.md` needed command corrections, commit them:

```bash
git add backend
git commit -m "test: verify simulation foundation workspace"
```

If there are no changes, skip this commit.

## Self-Review

- Spec coverage: This plan covers Rust authoritative backend shape, tile arrays, ECS materialization, chunks, dirty snapshots, database boundary shape, and read-only server endpoints. Supabase/Postgres implementation is intentionally deferred because the v2 spec says no exact schema or implementation plan yet.
- Placeholder scan: clean.
- Type consistency: `WorldId`, `ChunkCoord`, `TileKind`, `ChunkActivity`, `Chunk`, and `MaterializedRuntime` are introduced before use in later tasks.
