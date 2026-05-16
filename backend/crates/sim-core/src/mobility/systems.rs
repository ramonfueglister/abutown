use bevy_ecs::prelude::*;
use crate::mobility::resources::Tick;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum MobilitySet {
    Advance,
    Output,
    Bookkeeping,
}

pub fn install_systems(schedule: &mut Schedule) {
    schedule.configure_sets(
        (
            MobilitySet::Advance,
            MobilitySet::Output.after(MobilitySet::Advance),
            MobilitySet::Bookkeeping.after(MobilitySet::Output),
        )
    );
    schedule.add_systems((
        walk_advance_system.in_set(MobilitySet::Advance),
        vehicle_advance_system.in_set(MobilitySet::Advance),
        stop_arrival_system.in_set(MobilitySet::Advance),
        boarding_alighting_system.in_set(MobilitySet::Advance),
        compute_world_coord_system.in_set(MobilitySet::Output),
        compute_direction_system.in_set(MobilitySet::Output),
        tick_increment_system.in_set(MobilitySet::Bookkeeping),
    ));
}

// Stubs: real bodies in Tasks 7-9.

pub fn walk_advance_system() {}

pub fn vehicle_advance_system() {}

pub fn stop_arrival_system() {}

pub fn boarding_alighting_system() {}

pub fn compute_world_coord_system() {}

pub fn compute_direction_system() {}

pub fn tick_increment_system(mut tick: ResMut<Tick>) {
    tick.0 += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increment_system_advances_tick_by_one_per_schedule_run() {
        let mut world = World::new();
        world.insert_resource(Tick(0));
        let mut schedule = Schedule::default();
        install_systems(&mut schedule);
        schedule.run(&mut world);
        assert_eq!(world.resource::<Tick>().0, 1);
        schedule.run(&mut world);
        assert_eq!(world.resource::<Tick>().0, 2);
    }
}
