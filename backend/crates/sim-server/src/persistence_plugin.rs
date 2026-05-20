use sim_core::bevy_ecs::prelude::*;
use sim_core::bevy_ecs::schedule::Schedule;
use sim_core::world::persistence::SnapshotProviders;
use sim_core::world::schedule::SimPlugin;
use sim_core::world::snapshot_provider::ChunkSnapshotProvider;

pub struct PersistencePlugin {
    pub world_id: String,
}

impl SimPlugin for PersistencePlugin {
    fn name(&self) -> &'static str {
        "persistence"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        // SnapshotProviders is already inserted by CorePlugin; we just
        // append our providers to it.
        let mut providers = world.resource_mut::<SnapshotProviders>();
        providers.0.push(Box::new(ChunkSnapshotProvider {
            world_id: self.world_id.clone(),
        }));
        // MobilitySnapshotProvider gets pushed in Task 11.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::world::plugin::CorePlugin;

    #[test]
    fn persistence_plugin_registers_chunk_snapshot_provider() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        PersistencePlugin {
            world_id: "test".to_string(),
        }
        .install(&mut world, &mut schedule);
        let providers = world.resource::<SnapshotProviders>();
        assert_eq!(providers.0.len(), 1);
        assert_eq!(providers.0[0].name(), "chunk");
    }
}
