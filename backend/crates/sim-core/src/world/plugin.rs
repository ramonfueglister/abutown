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
        // CorePlugin is single-install per World. Double-installing wipes
        // `ChunksByCoord` (and other resources), losing any chunk entities
        // already spawned. The double-install test below exists to document
        // this contract; in debug builds, we panic loudly if a caller
        // accidentally installs twice.
        debug_assert!(
            !world.contains_resource::<ChunksByCoord>(),
            "CorePlugin::install called twice on the same World — \
             this resets ChunksByCoord and loses spawned chunk entities. \
             Install CorePlugin exactly once per World.",
        );
        // Resources
        world.insert_resource(WorldIdRes(self.world_id.clone()));
        world.insert_resource(ChunkSizeRes(self.chunk_size));
        world.insert_resource(WorldDimensions {
            width_tiles: self.world_dimensions.0,
            height_tiles: self.world_dimensions.1,
        });
        world.insert_resource(ChunksByCoord::default());
        world.insert_resource(TickClock::default());
        world.insert_resource(EventCount::default());
        world.insert_resource(DirtyChunks::default());
        world.insert_resource(DeterministicRng::from_world_id(&self.world_id));
        world.insert_resource(crate::world::persistence::SnapshotProviders::default());
        world.insert_resource(crate::world::persistence::MigrationRegistry::default());

        // Messages (Bevy 0.18 — buffered events live as `Messages<T>` resources)
        world.insert_resource(Messages::<ChunkLoaded>::default());
        world.insert_resource(Messages::<ChunkUnloaded>::default());
        world.insert_resource(Messages::<TileChanged>::default());
        world.insert_resource(Messages::<ChunkLodChanged>::default());

        // System sets (ordering chain)
        schedule.configure_sets(
            (
                CoreSet::ChunkLifecycle,
                CoreSet::TileMutation,
                CoreSet::LodReclassify,
                CoreSet::EventEmit,
            ).chain()
        );

        // Chunk LOD reclassification — owns marker swaps + ChunkLodChanged events.
        schedule.add_systems(
            crate::world::systems::reclassify_chunk_lod_system
                .in_set(CoreSet::LodReclassify),
        );

        // Buffer maintenance for Messages<T> (Bevy requires periodic `.update()`).
        schedule.add_systems(crate::world::systems::flush_event_buffers.in_set(CoreSet::EventEmit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;

    #[test]
    fn core_plugin_installs_resources_and_messages() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        assert!(world.contains_resource::<ChunksByCoord>());
        assert!(world.contains_resource::<TickClock>());
        assert!(world.contains_resource::<DeterministicRng>());
        assert!(world.contains_resource::<Messages<ChunkLoaded>>());
        assert!(world.contains_resource::<Messages<TileChanged>>());
        assert_eq!(world.resource::<ChunkSizeRes>().0, 32);
    }

    #[test]
    #[should_panic(expected = "CorePlugin::install called twice")]
    fn core_plugin_install_is_not_idempotent_callers_must_install_once() {
        // Production code must install CorePlugin exactly once per World.
        // The second install would silently reset `ChunksByCoord` etc.; we
        // catch that misuse with a debug_assert at the top of `install`.
        let mut world = World::new();
        let mut schedule = Schedule::default();
        let plugin = CorePlugin::default();
        plugin.install(&mut world, &mut schedule);
        let entity = world.spawn_empty().id();
        world.resource_mut::<ChunksByCoord>().0.insert(
            ChunkCoord { x: 0, y: 0 },
            entity,
        );
        plugin.install(&mut world, &mut schedule);
    }
}
