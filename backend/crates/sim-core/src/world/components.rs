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
