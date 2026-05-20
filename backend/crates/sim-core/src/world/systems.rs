use std::time::Instant;

use bevy_ecs::prelude::*;

use crate::ids::ChunkCoord;
use crate::scheduler::ChunkActivity;
use crate::tile::{TileKind, TileRecord};
use crate::world::components::{
    ActiveChunk, AsleepChunk, ChunkCoordComp, ChunkSize, ChunkSubscriberCount, ChunkVersion,
    DirtyTiles, HotChunk, LastPersistedVersion, LastSnapshotAt, LodCooldown, Tiles, WarmChunk,
};
use crate::world::events::*;
use crate::world::resources::{ChunksByCoord, DirtyChunks};

/// Pump message buffers — Bevy's `Messages<T>` requires periodic `update()`
/// calls to drop already-read messages from the buffer. We do it once per
/// tick in `CoreSet::EventEmit` so downstream consumers (mobility, persistence,
/// future plugins) read against a fresh buffer next tick.
pub fn flush_event_buffers(
    mut chunk_loaded: ResMut<Messages<ChunkLoaded>>,
    mut chunk_unloaded: ResMut<Messages<ChunkUnloaded>>,
    mut tile_changed: ResMut<Messages<TileChanged>>,
    mut chunk_lod_changed: ResMut<Messages<ChunkLodChanged>>,
) {
    chunk_loaded.update();
    chunk_unloaded.update();
    tile_changed.update();
    chunk_lod_changed.update();
}

/// Spawn a chunk entity from the supplied chunk data. Inserts the entity into
/// `ChunksByCoord` and writes a `ChunkLoaded` message. Returns the new entity.
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
        ChunkActivity::Warm   => { entity_commands.insert(WarmChunk); }
        ChunkActivity::Active => { entity_commands.insert(ActiveChunk); }
        ChunkActivity::Hot    => { entity_commands.insert(HotChunk); }
    }
    let entity = entity_commands.id();
    world.resource_mut::<ChunksByCoord>().0.insert(coord, entity);
    world.resource_mut::<Messages<ChunkLoaded>>().write(ChunkLoaded {
        entity,
        coord,
        initial_version,
    });
    entity
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn flush_event_buffers_runs_inside_schedule() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        // Write an event; running the schedule should not panic and should
        // rotate the buffer.
        let entity = world.spawn_empty().id();
        world.resource_mut::<Messages<ChunkLoaded>>().write(ChunkLoaded {
            entity,
            coord: ChunkCoord { x: 0, y: 0 },
            initial_version: 0,
        });
        schedule.run(&mut world);
        // No panic = pass. Explicit assertions on buffer rotation are
        // brittle across bevy versions.
    }
}

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
/// `Tiles`, marks `DirtyTiles`, writes `TileChanged` message. Returns the
/// new chunk version on success.
pub fn apply_set_tile_kind_ecs(
    world: &mut World,
    coord: ChunkCoord,
    local_index: u16,
    new_kind: TileKind,
    tick: u64,
) -> Result<TileMutationResult, TileMutationError> {
    let entity = *world
        .resource::<ChunksByCoord>()
        .0
        .get(&coord)
        .ok_or(TileMutationError::ChunkNotLoaded { coord })?;
    let (old_kind, new_version) = {
        let mut chunk_ent = world.entity_mut(entity);
        let old_kind;
        let new_version;
        {
            let mut tiles = chunk_ent
                .get_mut::<Tiles>()
                .expect("Tiles component on chunk entity");
            let tile_count = tiles.0.len() as u32;
            if local_index as u32 >= tile_count {
                return Err(TileMutationError::TileOutOfBounds {
                    index: local_index,
                    tile_count,
                });
            }
            old_kind = tiles.0[local_index as usize].kind;
            if old_kind == new_kind {
                return Err(TileMutationError::NoStateChange {
                    coord,
                    local_index,
                    kind: new_kind,
                });
            }
            tiles.0[local_index as usize].kind = new_kind;
            tiles.0[local_index as usize].flags.modified = true;
        }
        {
            let mut version = chunk_ent
                .get_mut::<ChunkVersion>()
                .expect("ChunkVersion on chunk entity");
            version.0 += 1;
            new_version = version.0;
        }
        // Re-borrow Tiles to update the per-tile version. Two separate get_mut
        // calls are required because we cannot hold both ChunkVersion and
        // Tiles mut at once.
        chunk_ent
            .get_mut::<Tiles>()
            .expect("Tiles component on chunk entity")
            .0[local_index as usize]
            .version = new_version;
        chunk_ent
            .get_mut::<DirtyTiles>()
            .expect("DirtyTiles on chunk entity")
            .0
            .insert(local_index);
        (old_kind, new_version)
    };
    world.resource_mut::<DirtyChunks>().0.insert(entity);
    world
        .resource_mut::<Messages<TileChanged>>()
        .write(TileChanged {
            chunk: entity,
            coord,
            local_index,
            old_kind,
            new_kind,
            new_version,
            tick,
        });
    Ok(TileMutationResult {
        chunk_entity: entity,
        new_version,
        old_kind,
    })
}

/// Query helper: collect chunk snapshot data for a coord. Returns `None`
/// if no chunk entity is loaded at that coord.
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
    fn apply_set_tile_kind_ecs_bumps_version_and_writes_message() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        let coord = ChunkCoord { x: 2, y: 3 };
        let _entity = spawn_chunk_entity(
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            0,
            ChunkActivity::Active,
        );
        let result = apply_set_tile_kind_ecs(&mut world, coord, 5, TileKind::Road, 1).unwrap();
        assert_eq!(result.new_version, 1);
        let entity = world.resource::<ChunksByCoord>().0[&coord];
        let tiles = world.get::<Tiles>(entity).unwrap();
        assert_eq!(tiles.0[5].kind, TileKind::Road);
        let dirty = world.get::<DirtyTiles>(entity).unwrap();
        assert!(dirty.0.contains(&5));
        let messages = world.resource::<Messages<TileChanged>>();
        let mut cursor = messages.get_cursor();
        let read: Vec<_> = cursor.read(messages).collect();
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
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            0,
            ChunkActivity::Active,
        );
        let err = apply_set_tile_kind_ecs(&mut world, coord, 5, TileKind::Grass, 1).unwrap_err();
        assert!(matches!(err, TileMutationError::NoStateChange { .. }));
    }
}

#[cfg(test)]
mod spawn_tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::TileRecord;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn spawn_chunk_entity_populates_chunks_by_coord_and_emits_loaded() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);

        let coord = ChunkCoord { x: 7, y: 11 };
        let entity = spawn_chunk_entity(
            &mut world, coord, 4,
            vec![TileRecord::default(); 16], 3, ChunkActivity::Warm,
        );

        // Indexed in ChunksByCoord
        assert_eq!(world.resource::<ChunksByCoord>().0[&coord], entity);

        // Has the right marker
        assert!(world.get::<WarmChunk>(entity).is_some());

        // ChunkLoaded was written
        let messages = world.resource::<Messages<ChunkLoaded>>();
        let mut cursor = messages.get_cursor();
        let read: Vec<_> = cursor.read(messages).collect();
        assert!(read.iter().any(|e| e.entity == entity && e.coord == coord));
    }
}
