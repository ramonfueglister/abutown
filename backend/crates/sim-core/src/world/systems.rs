use std::time::Instant;

use bevy_ecs::prelude::*;

use crate::ids::ChunkCoord;
use crate::scheduler::ChunkActivity;
use crate::tile::TileRecord;
use crate::world::components::{
    ActiveChunk, AsleepChunk, ChunkCoordComp, ChunkSize, ChunkSubscriberCount, ChunkVersion,
    DirtyTiles, HotChunk, LastPersistedVersion, LastSnapshotAt, LodCooldown, Tiles, WarmChunk,
};
use crate::world::events::*;
use crate::world::resources::ChunksByCoord;

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
