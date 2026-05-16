use bevy_ecs::prelude::*;
use crate::mobility::components::*;
use crate::mobility::records::AgentMobilityState;
use crate::mobility::resources::*;

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

pub fn walk_advance_system(
    mut query: Query<
        (Entity, &mut AgentMobilityStateComponent, &WalkSpeed),
        With<AgentMarker>,
    >,
    mut dirty: ResMut<DirtyAgents>,
) {
    for (entity, mut state, speed) in query.iter_mut() {
        if let AgentMobilityState::Walking { progress, .. } = &mut state.0 {
            let next = (*progress + speed.0).min(1.0);
            if next != *progress {
                *progress = next;
                dirty.0.insert(entity);
            }
        }
    }
}

pub fn vehicle_advance_system(
    mut query: Query<
        (Entity, &mut RoutePosition, &mut DwellTicksRemaining),
        With<VehicleMarker>,
    >,
    routes: Res<Routes>,
    mut dirty: ResMut<DirtyVehicles>,
) {
    for (entity, mut pos, mut dwell) in query.iter_mut() {
        // dwell counts down first
        if dwell.0 > 0 {
            dwell.0 -= 1;
            dirty.0.insert(entity);
            continue;
        }
        // can only advance if route exists and progress < 1.0
        let Some(route) = routes.0.get(&pos.route_id) else { continue; };
        if route.links.is_empty() || pos.progress >= 1.0 {
            continue;
        }
        let next = (pos.progress + pos.speed).min(1.0);
        if next != pos.progress {
            pos.progress = next;
            dirty.0.insert(entity);
        }
    }
}

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

    #[test]
    fn walk_advance_advances_progress_by_walk_speed() {
        use crate::ids::{AgentId, LinkId};
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        world.insert_resource(Tick(0));
        world.insert_resource(DirtyAgents::default());

        let entity = world.spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: LinkId("link:test".into()),
                progress: 0.2,
            }),
            WalkPlan { stages: vec![], cursor: 0 },
            WalkSpeed(0.1),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        )).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(walk_advance_system);
        schedule.run(&mut world);

        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        match &state.0 {
            AgentMobilityState::Walking { progress, .. } => {
                assert!((progress - 0.3).abs() < 1e-6, "progress should be 0.3, got {progress}");
            }
            other => panic!("expected Walking, got {other:?}"),
        }
        assert!(world.resource::<DirtyAgents>().0.contains(&entity));
    }

    #[test]
    fn walk_advance_clamps_at_one_and_marks_dirty() {
        use crate::ids::{AgentId, LinkId};
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        world.insert_resource(DirtyAgents::default());

        let entity = world.spawn((
            AgentMarker,
            StableAgentId(AgentId("a:near".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: LinkId("link:test".into()),
                progress: 0.95,
            }),
            WalkPlan { stages: vec![], cursor: 0 },
            WalkSpeed(0.1),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        )).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(walk_advance_system);
        schedule.run(&mut world);

        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        match &state.0 {
            AgentMobilityState::Walking { progress, .. } => {
                assert!((progress - 1.0).abs() < 1e-6, "progress clamped to 1.0, got {progress}");
            }
            _ => panic!(),
        }
        assert!(world.resource::<DirtyAgents>().0.contains(&entity));
    }

    #[test]
    fn vehicle_advance_decrements_dwell_when_positive() {
        use crate::ids::{VehicleId, RouteId};
        use crate::mobility::records::VehicleKind;

        let mut world = World::new();
        world.insert_resource(Routes::default());
        world.insert_resource(DirtyVehicles::default());

        let entity = world.spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Tram),
            RoutePosition { route_id: RouteId("r:1".into()), link_index: 0, progress: 0.5, speed: 0.1 },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(3),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        )).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(vehicle_advance_system);
        schedule.run(&mut world);

        let dwell = world.get::<DwellTicksRemaining>(entity).unwrap();
        assert_eq!(dwell.0, 2);
        let pos = world.get::<RoutePosition>(entity).unwrap();
        assert!((pos.progress - 0.5).abs() < 1e-6, "progress unchanged during dwell");
        assert!(world.resource::<DirtyVehicles>().0.contains(&entity));
    }

    #[test]
    fn vehicle_advance_progresses_when_not_dwelling() {
        use crate::ids::{VehicleId, RouteId, LinkId};
        use crate::mobility::records::{VehicleKind, RouteRecord};

        let mut world = World::new();
        let mut routes = Routes::default();
        routes.0.insert(RouteId("r:1".into()), RouteRecord {
            id: RouteId("r:1".into()),
            links: vec![LinkId("l:1".into())],
        });
        world.insert_resource(routes);
        world.insert_resource(DirtyVehicles::default());

        let entity = world.spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Tram),
            RoutePosition { route_id: RouteId("r:1".into()), link_index: 0, progress: 0.4, speed: 0.1 },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        )).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(vehicle_advance_system);
        schedule.run(&mut world);

        let pos = world.get::<RoutePosition>(entity).unwrap();
        assert!((pos.progress - 0.5).abs() < 1e-6);
        assert!(world.resource::<DirtyVehicles>().0.contains(&entity));
    }
}
