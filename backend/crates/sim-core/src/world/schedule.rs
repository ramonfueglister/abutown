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
        fn name(&self) -> &'static str {
            "noop"
        }
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
