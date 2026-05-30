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
        providers.0.push(Box::new(
            sim_core::mobility::snapshot_provider::MobilitySnapshotProvider {
                world_id: self.world_id.clone(),
            },
        ));
        providers
            .0
            .push(Box::new(sim_core::economy::EconomySnapshotProvider {
                world_id: self.world_id.clone(),
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::world::plugin::CorePlugin;

    #[test]
    fn persistence_plugin_registers_both_providers() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
        PersistencePlugin {
            world_id: "test".to_string(),
        }
        .install(&mut world, &mut schedule);
        let providers = world.resource::<SnapshotProviders>();
        assert_eq!(providers.0.len(), 3);
        let names: Vec<&str> = providers.0.iter().map(|p| p.name()).collect();
        assert!(names.contains(&"chunk"));
        assert!(names.contains(&"mobility"));
        assert!(names.contains(&"economy"));
    }
}
