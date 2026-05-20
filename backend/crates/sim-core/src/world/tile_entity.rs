use bevy_ecs::prelude::*;

use crate::tile::TileKind;
use crate::world::components::{BelongsToChunk, LocalIndex, Tile};

/// Spawn a functional tile entity attached to a chunk.
///
/// The chunk's dense `Tiles` array is the canonical terrain payload; this
/// helper only spawns a separate Entity carrying domain components (Home,
/// Workplace, …) that future plugins attach via Bevy `Commands`. Foundation
/// (Phase 8a) ships NO domain components — they come in later phases.
pub fn spawn_functional_tile(
    commands: &mut Commands,
    chunk: Entity,
    local_index: u16,
    _kind: TileKind,
) -> Entity {
    commands
        .spawn((Tile, LocalIndex(local_index), BelongsToChunk(chunk)))
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::TileRecord;
    use crate::world::components::{
        ChunkCoordComp, ChunkSize, ChunkTiles, ChunkVersion, DirtyTiles, Tiles,
    };
    use bevy_ecs::system::RunSystemOnce;
    use bevy_ecs::world::World;

    fn spawn_chunk(world: &mut World) -> Entity {
        world
            .spawn((
                ChunkCoordComp(ChunkCoord { x: 0, y: 0 }),
                ChunkSize(4),
                Tiles(vec![TileRecord::default(); 16]),
                ChunkVersion(0),
                DirtyTiles::default(),
            ))
            .id()
    }

    #[test]
    fn spawn_functional_tile_attaches_to_chunk() {
        let mut world = World::new();
        let chunk = spawn_chunk(&mut world);
        let tile = world
            .run_system_once(move |mut commands: Commands| {
                spawn_functional_tile(&mut commands, chunk, 5, TileKind::Road)
            })
            .unwrap();

        assert!(world.get::<Tile>(tile).is_some());
        assert_eq!(world.get::<LocalIndex>(tile).unwrap().0, 5);
        assert_eq!(world.get::<BelongsToChunk>(tile).unwrap().0, chunk);
    }

    #[test]
    fn chunk_tiles_relationship_auto_maintained() {
        let mut world = World::new();
        let chunk = spawn_chunk(&mut world);
        let _tile_a = world
            .run_system_once(move |mut commands: Commands| {
                spawn_functional_tile(&mut commands, chunk, 1, TileKind::Road)
            })
            .unwrap();
        let _tile_b = world
            .run_system_once(move |mut commands: Commands| {
                spawn_functional_tile(&mut commands, chunk, 2, TileKind::Water)
            })
            .unwrap();

        let tiles = world
            .get::<ChunkTiles>(chunk)
            .expect("ChunkTiles auto-populated");
        assert_eq!(tiles.len(), 2);
    }
}
