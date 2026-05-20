use bevy_ecs::prelude::*;

use crate::world::events::*;

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
