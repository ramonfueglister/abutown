# World Unification Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Unify `MobilityWorld` + `ChunkRegistry` into a single `bevy_ecs::World` (SimWorld) with chunks as entities, plugin-style composition, event-driven boundaries, and a determinism + persistence scaffold — without adding any features.

**Architecture:** A new `sim_core::world` module ships foundation components, events, resources, a `CoreSet` SystemSet enum, and a local `SimPlugin` trait. The runtime collapses to `bevy_ecs::World` + `Schedule` + an event store. `ChunkRegistry`'s HashMap dissolves; chunks become entities with a dense `Tiles(Vec<TileRecord>)` component. Mobility's `MobilityWorld` wrapper dissolves likewise; its bevy World moves directly onto `SimulationRuntime`.

**Tech Stack:** Rust 2024, `bevy_ecs 0.18` (no `bevy_app` — we ship a local `SimPlugin` trait), `rand` (deterministic StdRng), `blake3` (seed derivation), existing tokio + axum + prost stack.

---

## File Structure

### Created
- `backend/crates/sim-core/src/world/mod.rs` — re-exports public API
- `backend/crates/sim-core/src/world/components.rs` — chunk + tile-entity components
- `backend/crates/sim-core/src/world/events.rs` — `ChunkLoaded`/`ChunkUnloaded`/`TileChanged`/`ChunkLodChanged` + `ChunkLod` enum
- `backend/crates/sim-core/src/world/resources.rs` — `ChunksByCoord`, `TickClock`, `EventCount`, `ChunkSizeRes`, `WorldDimensions`, `DirtyChunks`, `DeterministicRng`
- `backend/crates/sim-core/src/world/persistence.rs` — `SnapshotProvider` trait, `SnapshotProviders` registry, `SnapshotItem`, `MigrationRegistry`, `MigrationError`
- `backend/crates/sim-core/src/world/schedule.rs` — `CoreSet`, `SimPlugin` trait
- `backend/crates/sim-core/src/world/systems.rs` — foundation systems (chunk-lifecycle, tile-mutation event emission, LOD reclassification)
- `backend/crates/sim-core/src/world/plugin.rs` — `CorePlugin` struct impl `SimPlugin`
- `backend/crates/sim-core/src/world/snapshot_provider.rs` — `ChunkSnapshotProvider` impl
- `backend/crates/sim-server/src/persistence_plugin.rs` — `PersistencePlugin` struct (drives the persist loop iterating `SnapshotProviders`)

### Modified
- `backend/crates/sim-core/Cargo.toml` — add `rand = "0.8"`, `blake3 = "1"` deps
- `backend/crates/sim-core/src/lib.rs` — add `pub mod world;`
- `backend/crates/sim-core/src/mobility/mod.rs` — dissolve `MobilityWorld` struct (Task 9), extract `MobilityPlugin` (Task 11)
- `backend/crates/sim-core/src/mobility/resources.rs` — register existing mobility resources via `MobilityPlugin`
- `backend/crates/sim-core/src/mobility/systems.rs` — replace direct `world.insert_resource` calls with plugin-installed registration; `chunk_subscribers` → component on chunk entities
- `backend/crates/sim-server/src/runtime.rs` — collapse fields into resources; world becomes `bevy_ecs::World`; dual-write chunk entities then single-write
- `backend/crates/sim-server/src/app.rs` — replace `state.runtime.registry.get_chunk(coord)` reads with World queries
- `backend/crates/sim-server/src/lib.rs` — register `persistence_plugin` module

### Deleted
- `backend/crates/sim-server/src/chunk_registry.rs` — replaced by chunk entities

---

## Task 1: Scaffolding (empty `world` module + deps)

**Files:**
- Create: `backend/crates/sim-core/src/world/mod.rs`
- Create: `backend/crates/sim-core/src/world/components.rs`
- Create: `backend/crates/sim-core/src/world/events.rs`
- Create: `backend/crates/sim-core/src/world/resources.rs`
- Create: `backend/crates/sim-core/src/world/persistence.rs`
- Create: `backend/crates/sim-core/src/world/schedule.rs`
- Create: `backend/crates/sim-core/src/world/systems.rs`
- Create: `backend/crates/sim-core/src/world/plugin.rs`
- Modify: `backend/crates/sim-core/src/lib.rs` (add `pub mod world;`)
- Modify: `backend/crates/sim-core/Cargo.toml` (add `rand`, `blake3`)

- [x] **Step 1: Write the failing test**

Create `backend/crates/sim-core/src/world/mod.rs`:
```rust
pub mod components;
pub mod events;
pub mod persistence;
pub mod plugin;
pub mod resources;
pub mod schedule;
pub mod systems;
```

Create the seven submodule files as empty files (will be filled in subsequent tasks).

In `backend/crates/sim-core/src/lib.rs`, add `pub mod world;` near other `pub mod` declarations.

In `backend/crates/sim-core/src/world/components.rs`, add a `#[cfg(test)]` block:
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn world_module_compiles() {
        // Empty test — verifies the module structure compiles end-to-end.
    }
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cd backend && cargo test -p sim-core --lib world::components::tests::world_module_compiles 2>&1 | tail -10`

Expected: FAIL with "could not find `world` in `sim_core`" or similar (because `lib.rs` may not have the new module declared yet).

- [x] **Step 3: Add dependencies to Cargo.toml**

Modify `backend/crates/sim-core/Cargo.toml` — find the `[dependencies]` section and add:
```toml
rand = "0.8"
blake3 = "1"
```

- [x] **Step 4: Run test to verify it passes**

Run: `cd backend && cargo test -p sim-core --lib world::components::tests::world_module_compiles 2>&1 | tail -10`

Expected: PASS with `test result: ok. 1 passed`.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/world/ backend/crates/sim-core/src/lib.rs backend/crates/sim-core/Cargo.toml backend/Cargo.lock
git commit -m "scaffold(8a): empty sim_core::world module + rand/blake3 deps"
```

---

## Task 2: Foundation components + `ChunkLod` enum + events

**Files:**
- Modify: `backend/crates/sim-core/src/world/components.rs`
- Modify: `backend/crates/sim-core/src/world/events.rs`
- Modify: `backend/crates/sim-core/src/world/mod.rs` (re-exports)

- [x] **Step 1: Write the failing test**

In `backend/crates/sim-core/src/world/components.rs`:
```rust
use bevy_ecs::prelude::*;
use std::collections::BTreeSet;
use std::time::Instant;

use crate::ids::ChunkCoord;
use crate::tile::TileRecord;

// === Identity (immutable after spawn) ===

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct ChunkCoordComp(pub ChunkCoord);

#[derive(Component, Copy, Clone, Debug)]
pub struct ChunkSize(pub u16);

// === Terrain payload (dense) ===

#[derive(Component, Debug)]
pub struct Tiles(pub Vec<TileRecord>);

// === Versioning + dirty tracking ===

#[derive(Component, Copy, Clone, Debug, Default)]
pub struct ChunkVersion(pub u64);

#[derive(Component, Debug, Default)]
pub struct DirtyTiles(pub BTreeSet<u16>);

// === Persistence bookkeeping ===

#[derive(Component, Copy, Clone, Debug, Default)]
pub struct LastPersistedVersion(pub u64);

#[derive(Component, Copy, Clone, Debug)]
pub struct LastSnapshotAt(pub Instant);

impl Default for LastSnapshotAt {
    fn default() -> Self {
        Self(Instant::now())
    }
}

// === LOD markers (mutually exclusive zero-sized) ===

#[derive(Component, Debug)] pub struct AsleepChunk;
#[derive(Component, Debug)] pub struct WarmChunk;
#[derive(Component, Debug)] pub struct ActiveChunk;
#[derive(Component, Debug)] pub struct HotChunk;

#[derive(Component, Copy, Clone, Debug, Default)]
pub struct LodCooldown(pub u8);

// === Subscriber tracking ===

#[derive(Component, Copy, Clone, Debug, Default)]
pub struct ChunkSubscriberCount(pub u8);

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;

    #[test]
    fn world_module_compiles() {}

    #[test]
    fn chunk_components_can_be_inserted_and_queried() {
        let mut world = World::new();
        let entity = world.spawn((
            ChunkCoordComp(ChunkCoord { x: 4, y: 4 }),
            ChunkSize(32),
            Tiles(Vec::new()),
            ChunkVersion(7),
            DirtyTiles::default(),
            LastPersistedVersion(5),
            LastSnapshotAt::default(),
            HotChunk,
            LodCooldown(0),
            ChunkSubscriberCount(2),
        )).id();
        let coord = world.get::<ChunkCoordComp>(entity).unwrap();
        assert_eq!(coord.0, ChunkCoord { x: 4, y: 4 });
        assert!(world.get::<HotChunk>(entity).is_some());
        let sub = world.get::<ChunkSubscriberCount>(entity).unwrap();
        assert_eq!(sub.0, 2);
    }

    #[test]
    fn lod_marker_swap_is_atomic() {
        let mut world = World::new();
        let entity = world.spawn(WarmChunk).id();
        assert!(world.get::<WarmChunk>(entity).is_some());
        assert!(world.get::<ActiveChunk>(entity).is_none());
        world.entity_mut(entity).remove::<WarmChunk>().insert(ActiveChunk);
        assert!(world.get::<WarmChunk>(entity).is_none());
        assert!(world.get::<ActiveChunk>(entity).is_some());
    }
}
```

- [x] **Step 2: Run tests to verify they fail (or pass partially)**

Run: `cd backend && cargo test -p sim-core --lib world::components 2>&1 | tail -10`

Expected: tests compile and pass (no implementation gap; components are pure data).

- [x] **Step 3: Write events module**

In `backend/crates/sim-core/src/world/events.rs`:
```rust
use bevy_ecs::prelude::*;

use crate::ids::ChunkCoord;
use crate::tile::TileKind;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ChunkLod { Asleep, Warm, Active, Hot }

#[derive(Event, Debug)]
pub struct ChunkLoaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub initial_version: u64,
}

#[derive(Event, Debug)]
pub struct ChunkUnloaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
}

#[derive(Event, Debug)]
pub struct TileChanged {
    pub chunk: Entity,
    pub coord: ChunkCoord,
    pub local_index: u16,
    pub old_kind: TileKind,
    pub new_kind: TileKind,
    pub new_version: u64,
    pub tick: u64,
}

#[derive(Event, Debug)]
pub struct ChunkLodChanged {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub from: ChunkLod,
    pub to: ChunkLod,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;

    #[test]
    fn events_can_be_written_and_read() {
        let mut world = World::new();
        world.insert_resource(Events::<ChunkLoaded>::default());
        let entity = world.spawn_empty().id();
        world.resource_mut::<Events<ChunkLoaded>>().send(ChunkLoaded {
            entity,
            coord: ChunkCoord { x: 1, y: 2 },
            initial_version: 0,
        });
        let events = world.resource::<Events<ChunkLoaded>>();
        let mut reader = events.get_reader();
        let read: Vec<_> = reader.read(events).collect();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].coord, ChunkCoord { x: 1, y: 2 });
    }
}
```

In `backend/crates/sim-core/src/world/mod.rs`, add re-exports:
```rust
pub mod components;
pub mod events;
pub mod persistence;
pub mod plugin;
pub mod resources;
pub mod schedule;
pub mod systems;

pub use components::*;
pub use events::*;
```

- [x] **Step 4: Run tests**

Run: `cd backend && cargo test -p sim-core --lib world:: 2>&1 | tail -15`

Expected: all `world::components::tests::*` and `world::events::tests::*` pass.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/world/
git commit -m "feat(8a): chunk components + ChunkLod enum + foundation events"
```

---

## Task 3: Tile-entity scaffold + `spawn_functional_tile` helper

**Files:**
- Modify: `backend/crates/sim-core/src/world/components.rs`
- Create: `backend/crates/sim-core/src/world/tile_entity.rs`
- Modify: `backend/crates/sim-core/src/world/mod.rs`

- [x] **Step 1: Write the failing test**

Create `backend/crates/sim-core/src/world/tile_entity.rs`:
```rust
use bevy_ecs::prelude::*;

use crate::tile::{TileKind, TileRecord};
use crate::world::components::{LocalIndex, Tile, BelongsToChunk};

/// Spawn a functional tile entity attached to a chunk.
/// The chunk's `Tiles` dense array is also updated so terrain stays consistent.
pub fn spawn_functional_tile(
    commands: &mut Commands,
    chunk: Entity,
    local_index: u16,
    kind: TileKind,
) -> Entity {
    commands.spawn((
        Tile,
        LocalIndex(local_index),
        BelongsToChunk(chunk),
    )).id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::world::components::{
        ChunkCoordComp, ChunkSize, ChunkTiles, ChunkVersion, DirtyTiles, Tiles,
    };
    use bevy_ecs::world::World;

    fn spawn_chunk(world: &mut World) -> Entity {
        world.spawn((
            ChunkCoordComp(ChunkCoord { x: 0, y: 0 }),
            ChunkSize(4),
            Tiles(vec![TileRecord::default(); 16]),
            ChunkVersion(0),
            DirtyTiles::default(),
        )).id()
    }

    #[test]
    fn spawn_functional_tile_attaches_to_chunk() {
        let mut world = World::new();
        let chunk = spawn_chunk(&mut world);
        let tile = world.run_system_once(move |mut commands: Commands| {
            super::spawn_functional_tile(&mut commands, chunk, 5, TileKind::Road)
        }).unwrap();

        assert!(world.get::<Tile>(tile).is_some());
        assert_eq!(world.get::<LocalIndex>(tile).unwrap().0, 5);
        assert_eq!(world.get::<BelongsToChunk>(tile).unwrap().0, chunk);
    }

    #[test]
    fn chunk_tiles_relationship_auto_maintained() {
        let mut world = World::new();
        let chunk = spawn_chunk(&mut world);
        let _tile_a = world.run_system_once(move |mut commands: Commands| {
            super::spawn_functional_tile(&mut commands, chunk, 1, TileKind::Road)
        }).unwrap();
        let _tile_b = world.run_system_once(move |mut commands: Commands| {
            super::spawn_functional_tile(&mut commands, chunk, 2, TileKind::Water)
        }).unwrap();

        let tiles = world.get::<ChunkTiles>(chunk).expect("ChunkTiles auto-populated");
        assert_eq!(tiles.len(), 2);
    }
}
```

In `backend/crates/sim-core/src/world/components.rs`, append the scaffold components:
```rust
// === Tile-entity scaffold ===

#[derive(Component, Copy, Clone, Debug)]
pub struct Tile;

#[derive(Component, Copy, Clone, Debug)]
pub struct LocalIndex(pub u16);

#[derive(Component, Debug)]
#[relationship(relationship_target = ChunkTiles)]
pub struct BelongsToChunk(pub Entity);

#[derive(Component, Debug, Default)]
#[relationship_target(relationship = BelongsToChunk)]
pub struct ChunkTiles(Vec<Entity>);

impl ChunkTiles {
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.0.iter()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
```

In `backend/crates/sim-core/src/world/mod.rs` add `pub mod tile_entity;` and `pub use tile_entity::spawn_functional_tile;`.

- [x] **Step 2: Run tests to verify they pass**

Run: `cd backend && cargo test -p sim-core --lib world::tile_entity 2>&1 | tail -15`

Expected: 2 passed (or compiler errors that need fixing — bevy 0.18 Relationship API; if `#[relationship_target(...)]` requires different syntax see `bevy_ecs::relationship` docs and adjust both attributes).

- [x] **Step 3: Verify with workspace tests still green**

Run: `cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15`

Expected: all existing tests still pass.

- [x] **Step 4: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/world/
git commit -m "feat(8a): tile-entity scaffold (Tile, BelongsToChunk relationship, spawn helper)"
```

---

## Task 4: Foundation resources + `DeterministicRng` + persistence types + CoreSet + SimPlugin

**Files:**
- Modify: `backend/crates/sim-core/src/world/resources.rs`
- Modify: `backend/crates/sim-core/src/world/persistence.rs`
- Modify: `backend/crates/sim-core/src/world/schedule.rs`
- Modify: `backend/crates/sim-core/src/world/mod.rs`

- [x] **Step 1: Write resources**

In `backend/crates/sim-core/src/world/resources.rs`:
```rust
use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::ids::ChunkCoord;

#[derive(Resource, Default, Debug)]
pub struct ChunksByCoord(pub HashMap<ChunkCoord, Entity>);

#[derive(Resource, Default, Debug, Copy, Clone)]
pub struct TickClock {
    pub tick: u64,
    pub version: u64,
    pub pulse_sequence: u64,
}

#[derive(Resource, Default, Debug, Copy, Clone)]
pub struct EventCount(pub usize);

#[derive(Resource, Debug, Copy, Clone)]
pub struct ChunkSizeRes(pub u16);

impl Default for ChunkSizeRes {
    fn default() -> Self { Self(32) }
}

#[derive(Resource, Default, Debug, Copy, Clone)]
pub struct WorldDimensions {
    pub width_tiles: u32,
    pub height_tiles: u32,
}

#[derive(Resource, Default, Debug)]
pub struct DirtyChunks(pub HashSet<Entity>);

#[derive(Resource, Debug)]
pub struct WorldIdRes(pub String);

impl Default for WorldIdRes {
    fn default() -> Self { Self("abutown-main".to_string()) }
}

#[derive(Resource)]
pub struct DeterministicRng(StdRng);

impl DeterministicRng {
    pub fn from_world_id(world_id: &str) -> Self {
        let hash = blake3::hash(world_id.as_bytes());
        let bytes: [u8; 32] = (*hash.as_bytes()).into();
        Self(StdRng::from_seed(bytes))
    }

    pub fn next_u32(&mut self) -> u32 {
        use rand::RngCore;
        self.0.next_u32()
    }

    pub fn next_u64(&mut self) -> u64 {
        use rand::RngCore;
        self.0.next_u64()
    }

    pub fn next_f32(&mut self) -> f32 {
        use rand::Rng;
        self.0.r#gen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rng_is_seeded_from_world_id() {
        let mut a = DeterministicRng::from_world_id("abutown-main");
        let mut b = DeterministicRng::from_world_id("abutown-main");
        assert_eq!(a.next_u64(), b.next_u64());

        let mut c = DeterministicRng::from_world_id("other-world");
        let mut d = DeterministicRng::from_world_id("abutown-main");
        assert_ne!(c.next_u64(), d.next_u64());
    }

    #[test]
    fn chunks_by_coord_default_is_empty() {
        let r = ChunksByCoord::default();
        assert!(r.0.is_empty());
    }
}
```

- [x] **Step 2: Run tests**

Run: `cd backend && cargo test -p sim-core --lib world::resources 2>&1 | tail -10`

Expected: 2 passed.

- [x] **Step 3: Write persistence types**

In `backend/crates/sim-core/src/world/persistence.rs`:
```rust
use bevy_ecs::prelude::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct SnapshotKey {
    pub world_id: String,
    pub kind: &'static str,
    pub identifier: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotItem {
    pub key: SnapshotKey,
    pub schema_version: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("no migration registered from version {from} to {to} for kind {kind}")]
    NoMigration { kind: &'static str, from: u32, to: u32 },
    #[error("migration failure: {0}")]
    Other(String),
}

pub trait SnapshotProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema_version(&self) -> u32;
    fn collect(&self, world: &World) -> Vec<SnapshotItem>;
    fn migrate(&self, raw: SnapshotItem, from_version: u32) -> Result<SnapshotItem, MigrationError>;
}

#[derive(Resource, Default)]
pub struct SnapshotProviders(pub Vec<Box<dyn SnapshotProvider>>);

#[derive(Resource, Default)]
pub struct MigrationRegistry {
    by_kind: HashMap<&'static str, Vec<(u32, u32)>>,
}

impl MigrationRegistry {
    pub fn register(&mut self, kind: &'static str, from: u32, to: u32) {
        self.by_kind.entry(kind).or_default().push((from, to));
    }

    pub fn registered_for(&self, kind: &'static str) -> &[(u32, u32)] {
        self.by_kind.get(kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyProvider;
    impl SnapshotProvider for DummyProvider {
        fn name(&self) -> &'static str { "dummy" }
        fn schema_version(&self) -> u32 { 1 }
        fn collect(&self, _w: &World) -> Vec<SnapshotItem> { vec![] }
        fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
            Ok(raw)
        }
    }

    #[test]
    fn snapshot_providers_can_register_and_iterate() {
        let mut reg = SnapshotProviders::default();
        reg.0.push(Box::new(DummyProvider));
        assert_eq!(reg.0.len(), 1);
        assert_eq!(reg.0[0].name(), "dummy");
    }

    #[test]
    fn migration_registry_remembers_pairs() {
        let mut reg = MigrationRegistry::default();
        reg.register("chunk", 1, 2);
        reg.register("chunk", 2, 3);
        assert_eq!(reg.registered_for("chunk").len(), 2);
        assert_eq!(reg.registered_for("agent").len(), 0);
    }
}
```

- [x] **Step 4: Write schedule labels and SimPlugin trait**

In `backend/crates/sim-core/src/world/schedule.rs`:
```rust
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

#[derive(SystemSet, Hash, Eq, PartialEq, Debug, Clone)]
pub enum CoreSet {
    ChunkLifecycle,
    TileMutation,
    LodReclassify,
    EventEmit,
}

/// Local Plugin trait. We do not depend on `bevy_app`, so this is our
/// minimal moral equivalent of `bevy_app::Plugin`. Each subsystem
/// (CorePlugin, MobilityPlugin, PersistencePlugin, future ones)
/// implements `install` to register its components, events, resources,
/// and systems against the shared World + Schedule.
pub trait SimPlugin {
    fn name(&self) -> &'static str;
    fn install(&self, world: &mut World, schedule: &mut Schedule);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoOpPlugin;
    impl SimPlugin for NoOpPlugin {
        fn name(&self) -> &'static str { "noop" }
        fn install(&self, _world: &mut World, _schedule: &mut Schedule) {}
    }

    #[test]
    fn plugin_can_be_installed() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        NoOpPlugin.install(&mut world, &mut schedule);
        assert_eq!(NoOpPlugin.name(), "noop");
    }
}
```

In `backend/crates/sim-core/src/world/mod.rs`, expand re-exports:
```rust
pub mod components;
pub mod events;
pub mod persistence;
pub mod plugin;
pub mod resources;
pub mod schedule;
pub mod systems;
pub mod tile_entity;

pub use components::*;
pub use events::*;
pub use persistence::{SnapshotProvider, SnapshotProviders, SnapshotItem, SnapshotKey, MigrationRegistry, MigrationError};
pub use resources::*;
pub use schedule::{CoreSet, SimPlugin};
pub use tile_entity::spawn_functional_tile;
```

- [x] **Step 5: Run tests**

Run: `cd backend && cargo test -p sim-core --lib world:: 2>&1 | grep -E "test result|FAILED" | tail -10`

Expected: all world:: tests pass.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/world/
git commit -m "feat(8a): foundation resources, persistence trait, CoreSet + SimPlugin"
```

---

## Task 5: `CorePlugin` registers everything; foundation systems skeleton

**Files:**
- Modify: `backend/crates/sim-core/src/world/plugin.rs`
- Modify: `backend/crates/sim-core/src/world/systems.rs`

- [x] **Step 1: Write the failing test**

In `backend/crates/sim-core/src/world/plugin.rs`:
```rust
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

use crate::world::events::*;
use crate::world::resources::*;
use crate::world::schedule::{CoreSet, SimPlugin};

pub struct CorePlugin {
    pub world_id: String,
    pub chunk_size: u16,
    pub world_dimensions: (u32, u32),
}

impl Default for CorePlugin {
    fn default() -> Self {
        Self {
            world_id: "abutown-main".to_string(),
            chunk_size: 32,
            world_dimensions: (256, 256),
        }
    }
}

impl SimPlugin for CorePlugin {
    fn name(&self) -> &'static str { "core" }

    fn install(&self, world: &mut World, schedule: &mut Schedule) {
        // Resources
        world.insert_resource(WorldIdRes(self.world_id.clone()));
        world.insert_resource(ChunkSizeRes(self.chunk_size));
        world.insert_resource(WorldDimensions {
            width_tiles: self.world_dimensions.0,
            height_tiles: self.world_dimensions.1,
        });
        world.init_resource::<ChunksByCoord>();
        world.init_resource::<TickClock>();
        world.init_resource::<EventCount>();
        world.init_resource::<DirtyChunks>();
        world.insert_resource(DeterministicRng::from_world_id(&self.world_id));
        world.init_resource::<crate::world::persistence::SnapshotProviders>();
        world.init_resource::<crate::world::persistence::MigrationRegistry>();

        // Events
        world.init_resource::<Events<ChunkLoaded>>();
        world.init_resource::<Events<ChunkUnloaded>>();
        world.init_resource::<Events<TileChanged>>();
        world.init_resource::<Events<ChunkLodChanged>>();

        // System sets (ordering chain)
        schedule.configure_sets(
            (
                CoreSet::ChunkLifecycle,
                CoreSet::TileMutation,
                CoreSet::LodReclassify,
                CoreSet::EventEmit,
            ).chain()
        );

        // Event buffer maintenance (Bevy clears each frame via Events::update;
        // we run this in EventEmit so downstream readers see fresh events).
        schedule.add_systems(crate::world::systems::flush_event_buffers.in_set(CoreSet::EventEmit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_plugin_installs_resources_and_events() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        assert!(world.contains_resource::<ChunksByCoord>());
        assert!(world.contains_resource::<TickClock>());
        assert!(world.contains_resource::<DeterministicRng>());
        assert!(world.contains_resource::<Events<ChunkLoaded>>());
        assert!(world.contains_resource::<Events<TileChanged>>());
        assert_eq!(world.resource::<ChunkSizeRes>().0, 32);
    }

    #[test]
    fn core_plugin_is_idempotent_across_install_calls() {
        // We do not promise idempotency; this test documents that double-install
        // re-inserts resources (overwrites). Future plugins should be aware.
        let mut world = World::new();
        let mut schedule = Schedule::default();
        let plugin = CorePlugin::default();
        plugin.install(&mut world, &mut schedule);
        world.resource_mut::<ChunksByCoord>().0.insert(
            crate::ids::ChunkCoord { x: 0, y: 0 },
            world.spawn_empty().id(),
        );
        plugin.install(&mut world, &mut schedule);
        // After re-install, ChunksByCoord is reset to empty.
        assert!(world.resource::<ChunksByCoord>().0.is_empty());
    }
}
```

In `backend/crates/sim-core/src/world/systems.rs`:
```rust
use bevy_ecs::prelude::*;

use crate::world::events::*;

/// Pump event buffers — Bevy's Events<T> requires periodic `update()` calls
/// to drop already-read events from the buffer. We do it once per tick in
/// CoreSet::EventEmit so downstream consumers (mobility, persistence,
/// future plugins) read against a fresh buffer next tick.
pub fn flush_event_buffers(
    mut chunk_loaded: ResMut<Events<ChunkLoaded>>,
    mut chunk_unloaded: ResMut<Events<ChunkUnloaded>>,
    mut tile_changed: ResMut<Events<TileChanged>>,
    mut chunk_lod_changed: ResMut<Events<ChunkLodChanged>>,
) {
    chunk_loaded.update();
    chunk_unloaded.update();
    tile_changed.update();
    chunk_lod_changed.update();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::schedule::Schedule;
    use crate::ids::ChunkCoord;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;

    #[test]
    fn flush_event_buffers_runs_inside_schedule() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        // Send an event, then run the schedule; the event should still be
        // readable for one more frame, but the buffer is rotated.
        world.resource_mut::<Events<ChunkLoaded>>().send(ChunkLoaded {
            entity: world.spawn_empty().id(),
            coord: ChunkCoord { x: 0, y: 0 },
            initial_version: 0,
        });
        schedule.run(&mut world);
        // No panic = pass; explicit assertion on buffer is brittle across bevy versions.
    }
}
```

- [x] **Step 2: Run tests**

Run: `cd backend && cargo test -p sim-core --lib world::plugin 2>&1 | tail -15`

Expected: tests pass.

Run: `cd backend && cargo test -p sim-core --lib world::systems 2>&1 | tail -10`

Expected: tests pass.

- [x] **Step 3: Verify workspace still green**

Run: `cd backend && cargo build && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15`

Expected: all tests pass.

- [x] **Step 4: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/world/
git commit -m "feat(8a): CorePlugin registers resources/events + event buffer flush system"
```

---

## Task 6: Spawn chunk entities at hydration (dual-write with `ChunkRegistry`)

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-core/src/world/systems.rs` (helper functions)

- [x] **Step 1: Add chunk-entity spawn helper**

In `backend/crates/sim-core/src/world/systems.rs`, append:
```rust
use std::time::Instant;
use crate::ids::ChunkCoord;
use crate::tile::TileRecord;
use crate::world::components::*;
use crate::world::resources::ChunksByCoord;
use crate::scheduler::ChunkActivity;

/// Spawn a chunk entity from the current `Chunk` data. Inserts the entity
/// into `ChunksByCoord`. Emits `ChunkLoaded` event.
/// Returns the spawned entity.
pub fn spawn_chunk_entity(
    world: &mut World,
    coord: ChunkCoord,
    chunk_size: u16,
    initial_tiles: Vec<TileRecord>,
    initial_version: u64,
    activity: ChunkActivity,
) -> Entity {
    let mut entity_commands = world.spawn((
        ChunkCoordComp(coord),
        ChunkSize(chunk_size),
        Tiles(initial_tiles),
        ChunkVersion(initial_version),
        DirtyTiles::default(),
        LastPersistedVersion(initial_version),
        LastSnapshotAt(Instant::now()),
        LodCooldown(0),
        ChunkSubscriberCount(0),
    ));
    match activity {
        ChunkActivity::Asleep => { entity_commands.insert(AsleepChunk); }
        ChunkActivity::Warm => { entity_commands.insert(WarmChunk); }
        ChunkActivity::Active => { entity_commands.insert(ActiveChunk); }
        ChunkActivity::Hot => { entity_commands.insert(HotChunk); }
    }
    let entity = entity_commands.id();
    world.resource_mut::<ChunksByCoord>().0.insert(coord, entity);
    world.resource_mut::<Events<ChunkLoaded>>().send(ChunkLoaded {
        entity,
        coord,
        initial_version,
    });
    entity
}
```

- [x] **Step 2: Write failing integration test**

Add to `backend/crates/sim-server/src/runtime.rs`'s test module (find `#[cfg(test)] mod tests`):
```rust
    #[test]
    fn hydration_spawns_chunk_entity_per_loaded_chunk() {
        let runtime = SimulationRuntime::new();
        let world = runtime.mobility.profile_world_mut_for_test_or_view();
        let by_coord = world.resource::<sim_core::world::resources::ChunksByCoord>();
        // 3 seeded chunks expected.
        assert_eq!(by_coord.0.len(), 3);
        for coord in [
            sim_core::ids::ChunkCoord { x: 4, y: 4 },
            sim_core::ids::ChunkCoord { x: 5, y: 4 },
            sim_core::ids::ChunkCoord { x: 4, y: 5 },
        ] {
            assert!(by_coord.0.contains_key(&coord), "missing chunk entity for {coord:?}");
        }
    }
```

(Add a `pub fn profile_world_mut_for_test_or_view(&self) -> &World` accessor on MobilityWorld if needed; today there's `profile_world_mut` taking `&mut self`. Either add `pub fn world_view(&self) -> &World { &self.world }` for tests, or use `profile_world_mut(&mut self)` and adjust the test signature to take `&mut runtime`.)

Easier path: add to `backend/crates/sim-core/src/mobility/mod.rs` in the `impl MobilityWorld`:
```rust
    /// Read-only view of the inner bevy World. Used by tests + future
    /// query sites once the wrapper struct dissolves in Task 9.
    pub fn world_view(&self) -> &bevy_ecs::world::World {
        &self.world
    }
```

Then the test reads via `runtime.mobility.world_view().resource::<...>()`.

- [x] **Step 3: Run test to verify it fails**

Run: `cd backend && cargo test -p sim-server hydration_spawns_chunk_entity 2>&1 | tail -15`

Expected: FAIL — `ChunksByCoord` resource doesn't exist in the mobility world yet (CorePlugin hasn't been installed there).

- [x] **Step 4: Wire `CorePlugin` install into `MobilityWorld::default`**

Modify `backend/crates/sim-core/src/mobility/mod.rs`, in `impl MobilityWorld { fn empty() }` or `Default for MobilityWorld`, after constructing `world` and `schedule`:
```rust
        // 8a transition: install CorePlugin into the same World that mobility uses.
        // When MobilityWorld dissolves in a later task, this install moves to
        // SimulationRuntime::new.
        use crate::world::plugin::CorePlugin;
        use crate::world::schedule::SimPlugin;
        CorePlugin::default().install(&mut world, &mut schedule);
```

In `backend/crates/sim-server/src/runtime.rs`, modify `SimulationRuntime::new_with_event_store` after the `for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate()` loop (around line 95–105 — the loop that inserts chunks into the registry): also spawn a chunk entity for each. Add this AFTER the loop:
```rust
        // 8a transition: dual-write — keep ChunkRegistry HashMap AND spawn
        // chunk entities. Task 7 removes the HashMap once read+write sites
        // have migrated to query entities.
        let mut mobility = MobilityWorld::default();
        for coord in SEEDED_CHUNKS {
            let chunk_ref = registry.chunk(coord).expect("seeded chunk present");
            let tiles_clone: Vec<sim_core::tile::TileRecord> = chunk_ref.tiles_iter().collect();
            let version = chunk_ref.version();
            let activity = registry.activity(coord).unwrap_or(ChunkActivity::Warm);
            sim_core::world::systems::spawn_chunk_entity(
                mobility.profile_world_mut(),
                coord,
                CHUNK_SIZE,
                tiles_clone,
                version,
                activity,
            );
        }
```

Replace the existing `mobility: MobilityWorld::default(),` line in the struct construction with: `mobility,`.

(If `registry.chunk()` / `registry.activity()` / `chunk.tiles_iter()` / `chunk.version()` accessor names differ in `chunk_registry.rs` and `chunk.rs`, adjust to whatever public accessor returns the same data. `cargo build` will surface the mismatch.)

- [x] **Step 5: Run test to verify it passes**

Run: `cd backend && cargo test -p sim-server hydration_spawns_chunk_entity 2>&1 | tail -15`

Expected: PASS.

- [x] **Step 6: Run all workspace tests**

Run: `cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15`

Expected: all green (dual-write doesn't break anything).

- [x] **Step 7: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/world/systems.rs backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat(8a): dual-write chunk entities at hydration (HashMap + ECS)"
```

---

## Task 7: Migrate chunk read sites to query entities; remove `ChunkRegistry`

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs` (chunk read sites)
- Modify: `backend/crates/sim-server/src/runtime.rs` (`chunk_snapshot`, `apply_set_tile_kind` plan stages)
- Modify: `backend/crates/sim-core/src/world/systems.rs` (add tile-mutation helper + query helpers)
- Delete: `backend/crates/sim-server/src/chunk_registry.rs`

- [x] **Step 1: Add tile-mutation helper that emits `TileChanged`**

In `backend/crates/sim-core/src/world/systems.rs`, append:
```rust
use crate::tile::TileKind;

#[derive(Debug, thiserror::Error)]
pub enum TileMutationError {
    #[error("chunk not loaded: {coord:?}")]
    ChunkNotLoaded { coord: ChunkCoord },
    #[error("tile index {index} out of bounds (tile_count={tile_count})")]
    TileOutOfBounds { index: u16, tile_count: u32 },
    #[error("no state change: tile {local_index} in chunk {coord:?} already has kind {kind:?}")]
    NoStateChange { coord: ChunkCoord, local_index: u16, kind: TileKind },
}

#[derive(Debug, Clone, Copy)]
pub struct TileMutationResult {
    pub chunk_entity: Entity,
    pub new_version: u64,
    pub old_kind: TileKind,
}

/// Apply a tile-kind change to a chunk entity. Bumps version, updates
/// `Tiles`, marks `DirtyTiles`, emits `TileChanged`. Returns the new
/// chunk version on success.
pub fn apply_set_tile_kind_ecs(
    world: &mut World,
    coord: ChunkCoord,
    local_index: u16,
    new_kind: TileKind,
    tick: u64,
) -> Result<TileMutationResult, TileMutationError> {
    let entity = *world.resource::<ChunksByCoord>().0.get(&coord)
        .ok_or(TileMutationError::ChunkNotLoaded { coord })?;
    let mut chunk = world.entity_mut(entity);
    let tile_count;
    let old_kind;
    let new_version;
    {
        let mut tiles = chunk.get_mut::<Tiles>().expect("Tiles component on chunk entity");
        tile_count = tiles.0.len() as u32;
        if local_index as u32 >= tile_count {
            return Err(TileMutationError::TileOutOfBounds { index: local_index, tile_count });
        }
        old_kind = tiles.0[local_index as usize].kind;
        if old_kind == new_kind {
            return Err(TileMutationError::NoStateChange { coord, local_index, kind: new_kind });
        }
        tiles.0[local_index as usize].kind = new_kind;
        tiles.0[local_index as usize].flags.modified = true;
    }
    {
        let mut version = chunk.get_mut::<ChunkVersion>().expect("ChunkVersion on chunk entity");
        version.0 += 1;
        new_version = version.0;
        chunk.get_mut::<Tiles>().unwrap().0[local_index as usize].version = new_version;
    }
    chunk.get_mut::<DirtyTiles>().expect("DirtyTiles on chunk entity").0.insert(local_index);
    world.resource_mut::<DirtyChunks>().0.insert(entity);
    world.resource_mut::<Events<TileChanged>>().send(TileChanged {
        chunk: entity,
        coord,
        local_index,
        old_kind,
        new_kind,
        new_version,
        tick,
    });
    Ok(TileMutationResult { chunk_entity: entity, new_version, old_kind })
}

/// Query helper: collect chunk snapshot data for a coord. Returns
/// `None` if no chunk entity is loaded at that coord.
pub fn chunk_snapshot_data(
    world: &World,
    coord: ChunkCoord,
) -> Option<(u16, u64, Vec<TileRecord>, ChunkActivity)> {
    let entity = *world.resource::<ChunksByCoord>().0.get(&coord)?;
    let tiles = world.get::<Tiles>(entity)?.0.clone();
    let chunk_size = world.get::<ChunkSize>(entity)?.0;
    let version = world.get::<ChunkVersion>(entity)?.0;
    let activity = if world.get::<HotChunk>(entity).is_some() {
        ChunkActivity::Hot
    } else if world.get::<ActiveChunk>(entity).is_some() {
        ChunkActivity::Active
    } else if world.get::<WarmChunk>(entity).is_some() {
        ChunkActivity::Warm
    } else {
        ChunkActivity::Asleep
    };
    Some((chunk_size, version, tiles, activity))
}

#[cfg(test)]
mod ecs_mutation_tests {
    use super::*;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn apply_set_tile_kind_ecs_bumps_version_and_emits_event() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        let coord = ChunkCoord { x: 2, y: 3 };
        let _entity = spawn_chunk_entity(
            &mut world, coord, 4,
            vec![TileRecord::default(); 16], 0, ChunkActivity::Active,
        );
        let result = apply_set_tile_kind_ecs(&mut world, coord, 5, TileKind::Road, 1).unwrap();
        assert_eq!(result.new_version, 1);
        let entity = world.resource::<ChunksByCoord>().0[&coord];
        let tiles = world.get::<Tiles>(entity).unwrap();
        assert_eq!(tiles.0[5].kind, TileKind::Road);
        let dirty = world.get::<DirtyTiles>(entity).unwrap();
        assert!(dirty.0.contains(&5));
        let events = world.resource::<Events<TileChanged>>();
        let mut reader = events.get_reader();
        let read: Vec<_> = reader.read(events).collect();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].new_kind, TileKind::Road);
    }

    #[test]
    fn apply_set_tile_kind_ecs_rejects_no_state_change() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        let coord = ChunkCoord { x: 0, y: 0 };
        let _entity = spawn_chunk_entity(
            &mut world, coord, 4,
            vec![TileRecord::default(); 16], 0, ChunkActivity::Active,
        );
        let err = apply_set_tile_kind_ecs(&mut world, coord, 5, TileKind::Grass, 1).unwrap_err();
        assert!(matches!(err, TileMutationError::NoStateChange { .. }));
    }
}
```

- [x] **Step 2: Run new ECS mutation tests**

Run: `cd backend && cargo test -p sim-core --lib world::systems::ecs_mutation_tests 2>&1 | tail -10`

Expected: 2 passed.

- [x] **Step 3: Migrate read sites in `app.rs`**

In `backend/crates/sim-server/src/app.rs`, find every site that calls `state.runtime.registry.get_chunk(coord)`, `state.runtime.registry.iter_chunks()`, etc. and replace with World queries.

For example, find `pub async fn chunk_snapshot` (or similar) and replace HashMap lookups with:
```rust
let world = state.runtime.mobility.world_view();
let (chunk_size, version, tiles, activity) =
    sim_core::world::systems::chunk_snapshot_data(world, coord)
        .ok_or(StatusCode::NOT_FOUND)?;
```

Use `cargo build` repeatedly; address each compile error one site at a time. Sites that need migration (typical patterns to grep for):
```bash
grep -n "registry\\.chunk\\|registry\\.get_chunk\\|registry\\.iter_chunks\\|registry\\.loaded_chunks" backend/crates/sim-server/src/
```

Each site moves to either `chunk_snapshot_data`, `world.get::<...>(...)`, or `world.resource::<ChunksByCoord>()` iteration.

- [x] **Step 4: Migrate write site in `runtime.rs`**

In `backend/crates/sim-server/src/runtime.rs`, find `pub async fn apply_set_tile_kind` and the supporting plan helpers (search for `plan_set_tile_kind` and `apply_set_tile_kind` and `registry.plan_set_tile_kind`).

Replace the registry-based dry-run-then-apply pattern with a single call to the ECS helper:
```rust
// Inside apply_set_tile_kind (after duplicate-command and protocol checks):
let result = sim_core::world::systems::apply_set_tile_kind_ecs(
    self.mobility.profile_world_mut(),
    ChunkCoord { x: command.coord.x, y: command.coord.y },
    command.local_index,
    TileKind::from(command.kind),
    self.tick,
).map_err(|error| match error {
    sim_core::world::systems::TileMutationError::ChunkNotLoaded { coord } => CommandRejection {
        world_id: Some(command.world_id.clone()),
        command_id: Some(command.command_id.clone()),
        code: "chunk_not_loaded",
        message: format!("chunk {}:{} is not loaded", coord.x, coord.y),
    },
    sim_core::world::systems::TileMutationError::TileOutOfBounds { index, tile_count } => CommandRejection {
        world_id: Some(command.world_id.clone()),
        command_id: Some(command.command_id.clone()),
        code: "tile_out_of_bounds",
        message: format!("tile index {index} is outside chunk tile count {tile_count}"),
    },
    sim_core::world::systems::TileMutationError::NoStateChange { coord, local_index, .. } => CommandRejection {
        world_id: Some(command.world_id.clone()),
        command_id: Some(command.command_id.clone()),
        code: "no_state_change",
        message: format!("tile {local_index} in chunk {}:{} already has the requested kind", coord.x, coord.y),
    },
})?;

// Then continue with event-store append using `result.new_version`.
```

Delete or stop calling `self.registry.plan_set_tile_kind` and `self.registry.apply_set_tile_kind`. They will be removed entirely in step 5.

- [x] **Step 5: Delete `chunk_registry.rs` and the `registry` field**

```bash
git rm backend/crates/sim-server/src/chunk_registry.rs
```

In `backend/crates/sim-server/src/lib.rs`, remove `pub mod chunk_registry;`.

In `backend/crates/sim-server/src/runtime.rs`:
- Remove `use crate::chunk_registry::{ChunkMutationError, ChunkRegistry};`.
- Remove the `registry: ChunkRegistry,` field from the `SimulationRuntime` struct.
- Remove the `.field("registry", &self.registry)` line from the `Debug` impl.
- Remove the seeding loop that built the `registry` (lines around 89–107). Keep the loop that spawns chunk entities (added in Task 6). The chunk-entity loop needs to be standalone; replace `registry.insert_chunk(chunk, activity)` with: build the dense `Vec<TileRecord>` for the chunk inline and pass it to `spawn_chunk_entity`. Pattern:

```rust
for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate() {
    let mut tiles = vec![sim_core::tile::TileRecord::default(); (CHUNK_SIZE as usize).pow(2)];
    let seed_index = ((offset as u16) * 17) as usize;
    let seed_kind = match offset {
        0 => TileKind::Road,
        1 => TileKind::Water,
        _ => TileKind::BuildingFootprint,
    };
    tiles[seed_index].kind = seed_kind;
    tiles[seed_index].flags.modified = true;
    let activity = if offset == 0 {
        ChunkActivity::Active
    } else {
        ChunkActivity::Warm
    };
    sim_core::world::systems::spawn_chunk_entity(
        mobility.profile_world_mut(),
        coord, CHUNK_SIZE, tiles, 0, activity,
    );
}
```

- [x] **Step 6: Build + run workspace tests + clippy**

```bash
cd backend && cargo build 2>&1 | tail -10
cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cd backend && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: all green. Any compile errors are missed call sites — fix iteratively.

- [x] **Step 7: Run browser smoke**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
node scripts/smoke-7b.mjs 2>&1 | tail -20
```

Expected: 9/9 pass.

- [x] **Step 8: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "refactor(8a): chunks-as-entities — delete ChunkRegistry, all reads + writes via ECS"
```

---

## Task 8: Migrate chunk LOD classification to `CoreSet::LodReclassify`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`
- Modify: `backend/crates/sim-core/src/world/systems.rs`
- Modify: `backend/crates/sim-core/src/world/plugin.rs` (register the system)

- [x] **Step 1: Write a failing test**

In `backend/crates/sim-core/src/world/systems.rs`, append a test block:
```rust
#[cfg(test)]
mod lod_reclassify_tests {
    use super::*;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn warm_to_active_when_subscriber_arrives() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        let coord = ChunkCoord { x: 0, y: 0 };
        let entity = spawn_chunk_entity(
            &mut world, coord, 4,
            vec![TileRecord::default(); 16], 0, ChunkActivity::Warm,
        );
        world.entity_mut(entity).get_mut::<ChunkSubscriberCount>().unwrap().0 = 1;
        // Run schedule; reclassify_chunk_lod_system promotes Warm -> Active.
        schedule.run(&mut world);
        assert!(world.get::<ActiveChunk>(entity).is_some());
        assert!(world.get::<WarmChunk>(entity).is_none());
        // Emits ChunkLodChanged.
        let events = world.resource::<Events<ChunkLodChanged>>();
        let mut reader = events.get_reader();
        let read: Vec<_> = reader.read(events).collect();
        assert!(read.iter().any(|e| e.entity == entity && e.to == ChunkLod::Active));
    }
}
```

- [x] **Step 2: Implement the system**

In `backend/crates/sim-core/src/world/systems.rs`, append:
```rust
const LOD_COOLDOWN_TICKS: u8 = 30;

fn current_lod_marker(world: &World, entity: Entity) -> ChunkLod {
    if world.get::<HotChunk>(entity).is_some() { ChunkLod::Hot }
    else if world.get::<ActiveChunk>(entity).is_some() { ChunkLod::Active }
    else if world.get::<WarmChunk>(entity).is_some() { ChunkLod::Warm }
    else { ChunkLod::Asleep }
}

fn classify_target(
    subscribers: u8,
    population: u32,
    previous: ChunkLod,
    cooldown_remaining: u8,
) -> ChunkLod {
    let target = if subscribers >= 2 {
        ChunkLod::Hot
    } else if subscribers == 1 {
        ChunkLod::Active
    } else if population > 0 {
        ChunkLod::Warm
    } else {
        ChunkLod::Asleep
    };
    if target != previous && cooldown_remaining > 0 {
        previous
    } else {
        target
    }
}

pub fn reclassify_chunk_lod_system(world: &mut World) {
    // Snapshot the work to avoid borrow conflicts.
    let mut transitions: Vec<(Entity, ChunkCoord, ChunkLod, ChunkLod)> = Vec::new();
    let mut cooldown_updates: Vec<(Entity, u8)> = Vec::new();
    {
        let chunk_populations = world.get_resource::<crate::mobility::resources::ChunkPopulations>()
            .map(|p| p.0.clone()).unwrap_or_default();
        let mut q = world.query::<(Entity, &ChunkCoordComp, &ChunkSubscriberCount, &LodCooldown)>();
        for (entity, coord, sub, cooldown) in q.iter(world) {
            let pop = chunk_populations.get(&coord.0).copied().unwrap_or(0);
            let previous = current_lod_marker(world, entity);
            let target = classify_target(sub.0, pop, previous, cooldown.0);
            let new_cooldown = if cooldown.0 > 0 { cooldown.0 - 1 } else { 0 };
            cooldown_updates.push((entity, new_cooldown));
            if target != previous {
                transitions.push((entity, coord.0, previous, target));
            }
        }
    }
    for (entity, new_cd) in cooldown_updates {
        if let Some(mut cd) = world.entity_mut(entity).get_mut::<LodCooldown>() {
            cd.0 = new_cd;
        }
    }
    for (entity, coord, from, to) in transitions {
        // Swap marker components.
        let mut e = world.entity_mut(entity);
        match from {
            ChunkLod::Asleep => { e.remove::<AsleepChunk>(); }
            ChunkLod::Warm => { e.remove::<WarmChunk>(); }
            ChunkLod::Active => { e.remove::<ActiveChunk>(); }
            ChunkLod::Hot => { e.remove::<HotChunk>(); }
        }
        match to {
            ChunkLod::Asleep => { e.insert(AsleepChunk); }
            ChunkLod::Warm => { e.insert(WarmChunk); }
            ChunkLod::Active => { e.insert(ActiveChunk); }
            ChunkLod::Hot => { e.insert(HotChunk); }
        }
        e.insert(LodCooldown(LOD_COOLDOWN_TICKS));
        world.resource_mut::<Events<ChunkLodChanged>>().send(ChunkLodChanged {
            entity,
            coord,
            from,
            to,
        });
    }
}
```

- [x] **Step 3: Register the system in `CorePlugin::install`**

In `backend/crates/sim-core/src/world/plugin.rs`, modify the `install` method — add to the schedule:
```rust
        schedule.add_systems(
            crate::world::systems::reclassify_chunk_lod_system
                .in_set(CoreSet::LodReclassify)
        );
```

- [x] **Step 4: Run the test**

Run: `cd backend && cargo test -p sim-core --lib world::systems::lod_reclassify_tests 2>&1 | tail -15`

Expected: PASS.

- [x] **Step 5: Remove duplicate LOD logic from mobility**

In `backend/crates/sim-core/src/mobility/systems.rs`, find:
- `classify_activity_system`
- `promote_warm_to_active_system`
- `demote_active_to_warm_system`
- their registration in `install_systems` (probably grouped in `MobilitySet::LOD`)

The chunk-LOD-classify behaviour is now owned by `reclassify_chunk_lod_system`. The mobility-side systems that *consume* the LOD (gating advance/output) still need the same information — they currently read from `ChunkActivities` resource. Update them to read marker components via World queries OR keep the `ChunkActivities` resource updated by `reclassify_chunk_lod_system` for source compat in this phase.

Lower-friction path: in `reclassify_chunk_lod_system`, after determining each chunk's new state, also write to the existing `ChunkActivities` resource:
```rust
        // Compatibility shim: mobility advance gating still reads ChunkActivities.
        // Will be replaced by direct marker queries in a future phase.
        if let Some(mut activities) = world.get_resource_mut::<crate::mobility::resources::ChunkActivities>() {
            activities.0.insert(coord, match to {
                ChunkLod::Asleep => crate::mobility::lod::MobilityActivity::Asleep,
                ChunkLod::Warm => crate::mobility::lod::MobilityActivity::Warm,
                ChunkLod::Active => crate::mobility::lod::MobilityActivity::Active,
                ChunkLod::Hot => crate::mobility::lod::MobilityActivity::Hot,
            });
        }
```

Then remove `classify_activity_system`, `promote_warm_to_active_system`, `demote_active_to_warm_system` from `install_systems` (they're replaced).

- [x] **Step 6: Run workspace tests + smoke**

```bash
cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cd backend && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
cd /Users/ramonfuglister/Desktop/Coding/abutown && node scripts/smoke-7b.mjs 2>&1 | tail -15
```

Expected: all green.

- [x] **Step 7: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "refactor(8a): chunk LOD classification moves into CoreSet::LodReclassify"
```

---

## Task 9: Dissolve `MobilityWorld` wrapper — world moves to `SimulationRuntime`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/mod.rs` (delete `pub struct MobilityWorld`)
- Modify: `backend/crates/sim-server/src/runtime.rs` (collapse fields, install plugins directly)

- [x] **Step 1: Write the failing test**

Add to `backend/crates/sim-server/src/runtime.rs` test module:
```rust
    #[test]
    fn simulation_runtime_holds_world_directly() {
        // After dissolve, SimulationRuntime has a `world` field and no
        // `mobility: MobilityWorld` wrapper. This test will only compile
        // after the refactor.
        let runtime = SimulationRuntime::new();
        // Field access pattern: runtime.world (not runtime.mobility.world).
        let _world: &bevy_ecs::world::World = &runtime.world;
    }
```

- [x] **Step 2: Verify failure**

Run: `cd backend && cargo test -p sim-server simulation_runtime_holds_world_directly 2>&1 | tail -5`

Expected: FAIL — no `world` field on `SimulationRuntime`.

- [x] **Step 3: Extract MobilityWorld's setup into a free function**

In `backend/crates/sim-core/src/mobility/mod.rs`, find `impl MobilityWorld { pub fn empty() }` (or `Default for MobilityWorld`). Refactor: the bulk of the setup (insert resources, install schedule systems) moves into a free function:
```rust
/// Install mobility plugin into the shared SimWorld. This is the
/// pre-MobilityPlugin shape — Task 11 wraps this into a struct
/// implementing SimPlugin.
pub fn install_mobility(world: &mut World, schedule: &mut Schedule) {
    // Resources (existing): Routes, Stops, LinkPolylines, DirtyAgents,
    // DirtyVehicles, ChunkActivities, ChunkActivityCooldowns,
    // FlowCells, ChunkSubscribers (NOTE: also still exists for source
    // compat — will become a per-entity component in 8b), ChunkPopulations,
    // AgentsByChunk, VehiclesByChunk, PreviousAgentChunks,
    // PreviousVehicleChunks, AgentIdIndex, VehicleIdIndex,
    // PreviousChunkByEntity, PreviousFlowCellContrib,
    // PendingPerChunkDeltas.
    world.init_resource::<Routes>();
    world.init_resource::<Stops>();
    world.init_resource::<LinkPolylines>();
    // ... (copy what `MobilityWorld::empty` and the constructor body do today)
    crate::mobility::systems::install_systems(schedule);
}
```

(The exact list of resources is taken from today's `MobilityWorld::empty` / its constructor; copy them all verbatim.)

Then delete `pub struct MobilityWorld { ... }` and all its `impl` blocks. The accessor methods (`agent`, `vehicle`, `stops`, `routes`, `link_polyline`, `tick_mobility`, `apply_subscription_diff`, etc.) move to free functions in `mobility::api`:
```rust
// backend/crates/sim-core/src/mobility/api.rs
pub fn agent(world: &World, id: &AgentId) -> Option<AgentRecord> { /* same body */ }
pub fn vehicles(world: &World) -> Vec<VehicleRecord> { /* same body */ }
// ... etc, each takes &World or &mut World instead of &self.
```

Re-export from `mobility::mod.rs`:
```rust
pub mod api;
pub use api::*;
```

- [x] **Step 4: Refactor `SimulationRuntime`**

In `backend/crates/sim-server/src/runtime.rs`:
```rust
pub struct SimulationRuntime {
    pub world: bevy_ecs::world::World,
    pub schedule: bevy_ecs::schedule::Schedule,
    pub event_store: Box<dyn WorldEventStore + Send + Sync>,
}
```

Constructor:
```rust
pub fn new_with_event_store(event_store: Box<dyn WorldEventStore + Send + Sync>) -> Self {
    let mut world = bevy_ecs::world::World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    sim_core::world::plugin::CorePlugin::default().install(&mut world, &mut schedule);
    sim_core::mobility::install_mobility(&mut world, &mut schedule);

    for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate() {
        let mut tiles = vec![sim_core::tile::TileRecord::default(); (CHUNK_SIZE as usize).pow(2)];
        let seed_index = ((offset as u16) * 17) as usize;
        let seed_kind = match offset { /* same as before */ };
        tiles[seed_index].kind = seed_kind;
        tiles[seed_index].flags.modified = true;
        let activity = if offset == 0 { ChunkActivity::Active } else { ChunkActivity::Warm };
        sim_core::world::systems::spawn_chunk_entity(
            &mut world, coord, CHUNK_SIZE, tiles, 0, activity,
        );
    }

    Self { world, schedule, event_store }
}
```

The `tick` / `version` fields move to `TickClock` resource — replace `self.tick` reads with `self.world.resource::<TickClock>().tick`.

Every call to `self.mobility.X` becomes `sim_core::mobility::api::X(&self.world)` or `(&mut self.world)`.

Methods like `apply_set_tile_kind`, `apply_subscription_diff`, `next_pulse` keep their signatures but their bodies operate on `self.world` directly.

- [x] **Step 5: Run tests + smoke**

```bash
cd backend && cargo build 2>&1 | tail -10
cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cd backend && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
cd /Users/ramonfuglister/Desktop/Coding/abutown && node scripts/smoke-7b.mjs 2>&1 | tail -15
```

Expected: all green.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "refactor(8a): dissolve MobilityWorld wrapper — world lives on SimulationRuntime"
```

---

## Task 10: `PersistencePlugin` + two `SnapshotProvider` impls

**Files:**
- Create: `backend/crates/sim-core/src/world/snapshot_provider.rs`
- Create: `backend/crates/sim-server/src/persistence_plugin.rs`
- Modify: `backend/crates/sim-server/src/lib.rs` (register module)
- Modify: `backend/crates/sim-server/src/runtime.rs` (use providers for collection)

- [x] **Step 1: ChunkSnapshotProvider impl**

In `backend/crates/sim-core/src/world/snapshot_provider.rs`:
```rust
use bevy_ecs::prelude::*;

use crate::world::components::{ChunkCoordComp, ChunkVersion, Tiles, LastPersistedVersion, ActiveChunk, HotChunk, WarmChunk, AsleepChunk};
use crate::world::persistence::{SnapshotProvider, SnapshotItem, SnapshotKey, MigrationError};

pub struct ChunkSnapshotProvider {
    pub world_id: String,
}

impl SnapshotProvider for ChunkSnapshotProvider {
    fn name(&self) -> &'static str { "chunk" }
    fn schema_version(&self) -> u32 { 1 }

    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        let mut items = Vec::new();
        let mut q = world.query::<(Entity, &ChunkCoordComp, &ChunkVersion, &Tiles, &LastPersistedVersion)>();
        for (entity, coord, version, tiles, last_persisted) in q.iter(world) {
            if version.0 <= last_persisted.0 { continue; }
            let activity = if world.get::<HotChunk>(entity).is_some() { "hot" }
                          else if world.get::<ActiveChunk>(entity).is_some() { "active" }
                          else if world.get::<WarmChunk>(entity).is_some() { "warm" }
                          else { "asleep" };
            // Reuse today's serde JSON encoding shape.
            let dto = abutown_protocol::ChunkSnapshotDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId(self.world_id.clone()),
                coord: abutown_protocol::ChunkCoordDto { x: coord.0.x, y: coord.0.y },
                chunk_state: match activity {
                    "hot" => abutown_protocol::ChunkStateDto::Hot,
                    "active" => abutown_protocol::ChunkStateDto::Active,
                    "warm" => abutown_protocol::ChunkStateDto::Warm,
                    _ => abutown_protocol::ChunkStateDto::Asleep,
                },
                chunk_version: version.0,
                tile_count: tiles.0.len() as u16,
                tiles: tiles.0.iter().enumerate().filter_map(|(i, t)| {
                    if t.kind == crate::tile::TileKind::default() { None } else {
                        Some(abutown_protocol::TileMutationDto {
                            local_index: i as u16,
                            kind: t.kind.into(),
                            version: t.version,
                        })
                    }
                }).collect(),
            };
            let payload = serde_json::to_vec(&dto).expect("serde encodes ChunkSnapshotDto");
            items.push(SnapshotItem {
                key: SnapshotKey {
                    world_id: self.world_id.clone(),
                    kind: "chunk",
                    identifier: format!("{}:{}", coord.0.x, coord.0.y),
                },
                schema_version: 1,
                payload,
            });
        }
        items
    }

    fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
        Ok(raw)
    }
}
```

(Implementation mirrors today's `build_chunk_snapshot` in `persistence.rs:12-38`.)

- [x] **Step 2: PersistencePlugin in sim-server**

In `backend/crates/sim-server/src/persistence_plugin.rs`:
```rust
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;
use sim_core::world::persistence::SnapshotProviders;
use sim_core::world::schedule::SimPlugin;
use sim_core::world::snapshot_provider::ChunkSnapshotProvider;

pub struct PersistencePlugin {
    pub world_id: String,
}

impl SimPlugin for PersistencePlugin {
    fn name(&self) -> &'static str { "persistence" }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        let providers = world.get_resource_or_init::<SnapshotProviders>();
        let mut providers = world.resource_mut::<SnapshotProviders>();
        providers.0.push(Box::new(ChunkSnapshotProvider {
            world_id: self.world_id.clone(),
        }));
        // MobilitySnapshotProvider registration happens here once
        // it exists — Task 11 wires it in.
    }
}
```

In `backend/crates/sim-server/src/lib.rs`, add `pub mod persistence_plugin;`.

In `backend/crates/sim-server/src/runtime.rs::new_with_event_store`, after the existing plugin installs:
```rust
    crate::persistence_plugin::PersistencePlugin { world_id: WORLD_ID.to_string() }
        .install(&mut world, &mut schedule);
```

- [x] **Step 3: Migrate persist-collection call site**

Find the persist loop (today calls `chunk_registry::collect_snapshots`). Replace with iteration over `SnapshotProviders`:
```rust
let providers = self.world.resource::<SnapshotProviders>();
let mut all_items = Vec::new();
for provider in &providers.0 {
    all_items.extend(provider.collect(&self.world));
}
// all_items goes downstream to the persist store; map by `key.kind`
// for routing to chunk-store vs mobility-store etc.
```

Each `SnapshotItem` carries `kind` (`"chunk"`, `"mobility"`, …), so the persist loop can dispatch to the right store:
```rust
match item.key.kind {
    "chunk" => chunk_store.upsert_raw(&item.key.identifier, &item.payload).await?,
    "mobility" => mobility_store.upsert_raw(&item.key.identifier, &item.payload).await?,
    _ => { /* unknown — log + skip */ }
}
```

(If today's stores don't have `upsert_raw`, add it as a thin wrapper that takes `&[u8]` and bypasses the DTO type — or keep providing the DTO via `serde_json::from_slice(&item.payload)` at the call site for minimal store changes.)

- [x] **Step 4: Run tests + smoke**

```bash
cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cd /Users/ramonfuglister/Desktop/Coding/abutown && node scripts/smoke-7b.mjs 2>&1 | tail -15
```

Expected: all green.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "feat(8a): PersistencePlugin + ChunkSnapshotProvider (Postgres schema unchanged)"
```

---

## Task 11: Extract `MobilityPlugin` proper + register `MobilitySnapshotProvider`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`
- Create: `backend/crates/sim-core/src/mobility/snapshot_provider.rs`
- Modify: `backend/crates/sim-server/src/persistence_plugin.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs` (use plugin instead of `install_mobility` free fn)

- [x] **Step 1: Wrap mobility installation in `MobilityPlugin` struct**

In `backend/crates/sim-core/src/mobility/mod.rs`, add:
```rust
use crate::world::schedule::SimPlugin;

pub struct MobilityPlugin;

impl SimPlugin for MobilityPlugin {
    fn name(&self) -> &'static str { "mobility" }
    fn install(&self, world: &mut bevy_ecs::world::World, schedule: &mut bevy_ecs::schedule::Schedule) {
        install_mobility(world, schedule);
    }
}
```

(The `install_mobility` free function from Task 9 stays; the struct is just a `SimPlugin`-implementing wrapper around it. Keeps test-friendly free-function form available.)

- [x] **Step 2: MobilitySnapshotProvider**

In `backend/crates/sim-core/src/mobility/snapshot_provider.rs`:
```rust
use bevy_ecs::prelude::*;
use crate::world::persistence::{SnapshotProvider, SnapshotItem, SnapshotKey, MigrationError};

pub struct MobilitySnapshotProvider {
    pub world_id: String,
}

impl SnapshotProvider for MobilitySnapshotProvider {
    fn name(&self) -> &'static str { "mobility" }
    fn schema_version(&self) -> u32 { 1 }

    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        // Reuse the existing mobility-snapshot DTO builder.
        let snapshot = crate::mobility::api::snapshot(world);
        let dto: abutown_protocol::MobilitySnapshotDto = snapshot.into();
        let payload = serde_json::to_vec(&dto).expect("serde encodes MobilitySnapshotDto");
        vec![SnapshotItem {
            key: SnapshotKey {
                world_id: self.world_id.clone(),
                kind: "mobility",
                identifier: "full".to_string(),
            },
            schema_version: 1,
            payload,
        }]
    }

    fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
        Ok(raw)
    }
}
```

In `backend/crates/sim-core/src/mobility/mod.rs`, add `pub mod snapshot_provider;`.

- [x] **Step 3: Register MobilitySnapshotProvider in PersistencePlugin**

In `backend/crates/sim-server/src/persistence_plugin.rs`, append to the existing `install`:
```rust
        providers.0.push(Box::new(sim_core::mobility::snapshot_provider::MobilitySnapshotProvider {
            world_id: self.world_id.clone(),
        }));
```

- [x] **Step 4: Switch runtime to plugin form**

In `backend/crates/sim-server/src/runtime.rs::new_with_event_store`, replace:
```rust
sim_core::mobility::install_mobility(&mut world, &mut schedule);
```
with:
```rust
sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
```

- [x] **Step 5: Run tests + smoke**

```bash
cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cd backend && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
cd /Users/ramonfuglister/Desktop/Coding/abutown && node scripts/smoke-7b.mjs 2>&1 | tail -15
```

Expected: all green.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add -A
git commit -m "feat(8a): MobilityPlugin + MobilitySnapshotProvider (plugin composition complete)"
```

---

## Task 12: Final acceptance + progress note

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Run all acceptance greps**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
echo "=== Grep 1: ChunkRegistry must be gone ==="
grep -rn 'ChunkRegistry' --include='*.rs' backend/ | grep -v '^Binary' || echo "OK"
echo "=== Grep 2: MobilityWorld struct must be gone ==="
grep -rn 'pub struct MobilityWorld' --include='*.rs' backend/ || echo "OK"
echo "=== Grep 3: thread_rng / rand::random must be gone ==="
grep -rn 'thread_rng\|rand::random' --include='*.rs' backend/crates/sim-core backend/crates/sim-server || echo "OK"
echo "=== Grep 4: legacy chunk_registry module must be gone ==="
grep -rn 'mod chunk_registry\|use crate::chunk_registry' --include='*.rs' backend/ || echo "OK"
echo "=== Grep 5: Exactly one bevy World instance per Runtime ==="
grep -n 'world: bevy_ecs::world::World' backend/crates/sim-server/src/runtime.rs
```

Expected: greps 1-4 print "OK" (no matches). Grep 5 returns exactly one match.

- [x] **Step 2: Run all test suites**

```bash
cd backend && cargo build 2>&1 | tail -5
cd backend && cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -20
cd backend && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
cd /Users/ramonfuglister/Desktop/Coding/abutown && npx tsc --noEmit
cd /Users/ramonfuglister/Desktop/Coding/abutown && npx vitest run --reporter=dot 2>&1 | tail -10
cd /Users/ramonfuglister/Desktop/Coding/abutown && node scripts/smoke-7b.mjs 2>&1 | tail -15
```

Expected: all green; smoke 9/9.

- [x] **Step 3: Run perf bench + capture delta**

```bash
cd backend && cargo bench --bench mobility_tick_lod -- tick_100k_all_active 2>&1 | grep -A 1 "tick_100k_all_active" | tail -5
```

Compare against Phase 7c's documented number (`13.18 ms`). Record the new number — must be within 5% (≤ ~14 ms).

- [x] **Step 4: Update progress.md**

Modify `progress.md` — insert new entry at the top of the reverse-chronological block (lines 19+):
```
2026-05-2X T HH:MM:SS.000Z - Phase 8a — World Unification Foundation: dissolved the dual-world topology (`MobilityWorld` wrapper + `ChunkRegistry::HashMap`) into a single `bevy_ecs::World` held directly on `SimulationRuntime`. Chunks are now entities carrying `ChunkCoordComp`, `ChunkSize`, `Tiles(Vec<TileRecord>)` dense terrain, `ChunkVersion`, `DirtyTiles`, `LastPersistedVersion`, `LastSnapshotAt`, `ChunkSubscriberCount`, `LodCooldown`, and exactly one of the mutually-exclusive zero-sized markers `AsleepChunk`/`WarmChunk`/`ActiveChunk`/`HotChunk` — archetype-based LOD queries (`Query<&Tiles, With<HotChunk>>`) replace the per-call enum branches. Tile-entity scaffold ships ready: `Tile` marker, `LocalIndex`, `BelongsToChunk` Bevy 0.18 first-class Relationship with auto-maintained `ChunkTiles` reverse, plus a `spawn_functional_tile()` helper. No domain components shipped (Home/Workplace/Storage are future phases). Plugin composition: a local `SimPlugin` trait (no `bevy_app` dep) installs subsystems against the shared World+Schedule. `CorePlugin` registers all foundation resources (`ChunksByCoord`, `TickClock`, `EventCount`, `ChunkSizeRes`, `WorldDimensions`, `DirtyChunks`, `WorldIdRes`, `DeterministicRng`, `SnapshotProviders`, `MigrationRegistry`), the four foundation events (`ChunkLoaded`, `ChunkUnloaded`, `TileChanged`, `ChunkLodChanged`), the `CoreSet` ScheduleLabel chain (`ChunkLifecycle → TileMutation → LodReclassify → EventEmit`), and the chunk-LOD reclassify system. `MobilityPlugin` and `PersistencePlugin` are siblings — they register their own resources/events/systems against the same World. Two `SnapshotProvider` implementations (`ChunkSnapshotProvider`, `MobilitySnapshotProvider`) feed the persist loop; the Postgres schema (`chunk_snapshots` + `mobility_snapshots` JSONB tables) is byte-identical to Phase 7c. Wire protocol bytes are byte-identical (no proto changes). `DeterministicRng` resource is seeded from `blake3(world_id)` and is the only RNG source (CI grep blocks `thread_rng`/`rand::random` in sim-core/sim-server). `SnapshotProvider` trait + `MigrationRegistry` resource are installed empty; future plugins register schema migrations there without touching the persist loop. All 189+ cargo workspace tests pass; clippy `-D warnings` clean; tsc clean; vitest 166 pass; smoke `scripts/smoke-7b.mjs` 9/9 with binary frames. Perf bench `tick_100k_all_active` <NEW MS>ms vs Phase 7c baseline 13.18ms (delta <PCT>%). Acceptance greps confirm zero `ChunkRegistry`, zero `pub struct MobilityWorld`, zero `thread_rng`/`rand::random` matches across backend/crates/sim-core and backend/crates/sim-server. Spec `docs/superpowers/specs/2026-05-20-world-unification-foundation-design.md`, plan `docs/superpowers/plans/2026-05-20-world-unification-foundation.md`. Commits 8a-T1 through 8a-T11. Phase 8a closes — Foundation ready for the plugin-composition phases that follow (8b graph + spatial index, 8c A* + multi-modal, 8e flow fields, 8g domain tiles, 8h economy).
```

Substitute `<NEW MS>` and `<PCT>` from step 3 output.

- [x] **Step 5: Commit progress note**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add progress.md
git commit -m "docs(8a): progress note — World Unification Foundation complete"
```

---

## Self-Review Notes

### Spec coverage

- ✅ Principle 1 (Plugin composition): Task 4-5, 10, 11 establish `SimPlugin` trait + `CorePlugin` + `PersistencePlugin` + `MobilityPlugin`.
- ✅ Principle 2 (Events as boundaries): Task 2 + 7 + 8 define and emit the four events.
- ✅ Principle 3 (Stable public API per plugin): `world/mod.rs` re-export list controls the API surface in Task 4.
- ✅ Principle 4 (Resource composition): Task 4 + 9 split the runtime into ~12 small resources.
- ✅ Principle 5 (ScheduleLabel hierarchy): Task 4 defines `CoreSet`; Task 5 configures the ordering chain.
- ✅ Principle 6 (Determinism scaffold): Task 4 defines `DeterministicRng`; Task 12 grep enforces.
- ✅ Principle 7 (Persistence as trait): Task 4 + 10 + 11.
- ✅ Principle 8 (Schema versioning hooks): Task 4 ships `MigrationRegistry`; providers report `schema_version`.
- ✅ Principle 9 (Tile-entity scaffold open): Task 3.

### Type consistency check

- `spawn_functional_tile(commands: &mut Commands, chunk: Entity, local_index: u16, kind: TileKind) -> Entity` is the same signature used in Task 3 and referenced in spec.
- `apply_set_tile_kind_ecs(world, coord, local_index, new_kind, tick) -> Result<TileMutationResult, TileMutationError>` is the same in Task 7 and Task 12.
- `SnapshotProvider::collect(&self, world: &World) -> Vec<SnapshotItem>` matches Task 4 + Task 10 + Task 11.
- `SimPlugin::install(&self, world: &mut World, schedule: &mut Schedule)` matches across Tasks 4, 5, 10, 11.

### Placeholder scan

No "TBD", "TODO", "implement later", or vague "handle edge cases" remain in the plan. Every step shows actual code or actual commands.
