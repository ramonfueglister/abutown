use crate::ids::{AgentId, RouteId, StopId, VehicleId};
use crate::mobility::components::*;
use crate::mobility::lod::{classify_chunk_mobility_activity, MobilityActivity, ACTIVITY_HYSTERESIS_TICKS};
use crate::mobility::records::{AgentMobilityState, PlanStage};
use crate::mobility::resources::*;
use bevy_ecs::prelude::*;

fn coord_at_progress(points: &[(f32, f32)], progress: f32) -> (f32, f32) {
    crate::mobility_geometry::world_coord_at_progress_slice(points, progress)
}

fn dir_at_progress(points: &[(f32, f32)], progress: f32) -> abutown_protocol::DirectionDto {
    crate::mobility_geometry::direction_at_progress_slice(points, progress)
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum MobilitySet {
    Advance,
    Output,
    Bookkeeping,
}

pub fn install_systems(schedule: &mut Schedule) {
    schedule.configure_sets((
        MobilitySet::Advance,
        MobilitySet::Output.after(MobilitySet::Advance),
        MobilitySet::Bookkeeping.after(MobilitySet::Output),
    ));
    schedule.add_systems((
        // Ordering within Advance (each step observes the previous step's
        // output, but is staged so that "newly waiting" agents are not
        // immediately boarded in the same tick they arrived at the stop, and
        // "just alighted" agents do not immediately walk further in the same
        // tick they got off):
        //
        //   1. walk_advance        — push Walking agents along their link.
        //   2. boarding_alighting  — apply Phase-3 boarding + alighting using
        //                            the PRE-stop_arrival waiting queue. This
        //                            means an agent that arrived at the stop
        //                            in step 3 of this same tick won't board
        //                            until the next tick.
        //   3. stop_arrival        — convert progress=1.0 walkers into
        //                            WaitingAtStop / AtActivity.
        //   4. vehicle_advance     — decrement dwell or push progress.
        walk_advance_system.in_set(MobilitySet::Advance),
        boarding_alighting_system
            .in_set(MobilitySet::Advance)
            .after(walk_advance_system),
        stop_arrival_system
            .in_set(MobilitySet::Advance)
            .after(boarding_alighting_system),
        vehicle_advance_system
            .in_set(MobilitySet::Advance)
            .after(stop_arrival_system),
        compute_world_coord_system.in_set(MobilitySet::Output),
        compute_direction_system.in_set(MobilitySet::Output),
        tick_increment_system.in_set(MobilitySet::Bookkeeping),
    ));
}

pub fn walk_advance_system(
    mut query: Query<(Entity, &mut AgentMobilityStateComponent, &WalkSpeed), With<AgentMarker>>,
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
    mut query: Query<(Entity, &mut RoutePosition, &mut DwellTicksRemaining), With<VehicleMarker>>,
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
        let Some(route) = routes.0.get(&pos.route_id) else {
            continue;
        };
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

pub fn stop_arrival_system(
    mut query: Query<
        (
            Entity,
            &StableAgentId,
            &mut AgentMobilityStateComponent,
            &mut WalkPlan,
        ),
        With<AgentMarker>,
    >,
    mut stops: ResMut<Stops>,
    mut dirty: ResMut<DirtyAgents>,
) {
    for (entity, stable, mut state, mut plan) in query.iter_mut() {
        let completed_walking = matches!(
            &state.0,
            AgentMobilityState::Walking { progress, .. } if *progress >= 1.0
        );
        if !completed_walking {
            continue;
        }

        let stage = plan.stages.get(plan.cursor).cloned();
        match stage {
            Some(PlanStage::WalkToStop { stop_id, .. }) => {
                plan.cursor += 1;
                state.0 = AgentMobilityState::WaitingAtStop {
                    stop_id: stop_id.clone(),
                };
                if let Some(stop) = stops.0.get_mut(&stop_id)
                    && !stop.waiting_agents.contains(&stable.0)
                {
                    stop.waiting_agents.push_back(stable.0.clone());
                }
                dirty.0.insert(entity);
            }
            Some(PlanStage::WalkToActivity { activity_id, .. }) => {
                plan.cursor += 1;
                state.0 = AgentMobilityState::AtActivity { activity_id };
                dirty.0.insert(entity);
            }
            _ => {}
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn boarding_alighting_system(
    mut sets: ParamSet<(
        Query<
            (
                Entity,
                &StableAgentId,
                &mut AgentMobilityStateComponent,
                &mut WalkPlan,
            ),
            With<AgentMarker>,
        >,
        Query<
            (
                Entity,
                &StableVehicleId,
                &mut Occupants,
                &Capacity,
                &RoutePosition,
            ),
            With<VehicleMarker>,
        >,
    )>,
    mut stops: ResMut<Stops>,
    mut dirty_agents: ResMut<DirtyAgents>,
    mut dirty_vehicles: ResMut<DirtyVehicles>,
) {
    // ------------------------------------------------------------------
    // Phase A: BOARDING
    // ------------------------------------------------------------------
    // A.1 — collect (stop_id, front agent, route/link/progress) for each stop
    // that has at least one waiting agent.
    let mut boarding_candidates: Vec<(StopId, AgentId, RouteId, usize, f32)> = Vec::new();
    for (stop_id, stop) in stops.0.iter() {
        if let Some(agent_id) = stop.waiting_agents.front() {
            boarding_candidates.push((
                stop_id.clone(),
                agent_id.clone(),
                stop.route_id.clone(),
                stop.link_index,
                stop.progress,
            ));
        }
    }

    // A.2 — find a matching vehicle for each candidate.
    let mut boardings: Vec<(StopId, AgentId, Entity, VehicleId, u16)> = Vec::new();
    {
        let vehicles = sets.p1();
        for (stop_id, agent_id, route_id, link_index, stop_progress) in boarding_candidates {
            for (v_entity, v_stable, v_occ, v_cap, v_pos) in vehicles.iter() {
                if v_pos.route_id == route_id
                    && v_pos.link_index == link_index
                    && (v_pos.progress - stop_progress).abs() < 1e-6
                    && v_occ.0.len() < v_cap.0 as usize
                {
                    let seat_index = v_occ.0.len() as u16;
                    boardings.push((
                        stop_id.clone(),
                        agent_id.clone(),
                        v_entity,
                        v_stable.0.clone(),
                        seat_index,
                    ));
                    break;
                }
            }
        }
    }

    // A.3 — apply vehicle-side mutations (append occupant).
    {
        let mut vehicles = sets.p1();
        for (_stop_id, agent_id, v_entity, _v_id, _seat) in &boardings {
            if let Ok((_, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
                v_occ.0.push(agent_id.clone());
                dirty_vehicles.0.insert(*v_entity);
            }
        }
    }

    // A.4 — pop boarded agents from stop queues.
    for (stop_id, agent_id, _, _, _) in &boardings {
        if let Some(stop) = stops.0.get_mut(stop_id)
            && stop.waiting_agents.front() == Some(agent_id)
        {
            stop.waiting_agents.pop_front();
        }
    }

    // A.5 — agent-side mutations: state becomes InVehicle.
    {
        let mut agents = sets.p0();
        for (_stop_id, agent_id, _v_entity, v_id, seat_index) in &boardings {
            for (a_entity, a_stable, mut a_state, _a_plan) in agents.iter_mut() {
                if &a_stable.0 == agent_id {
                    a_state.0 = AgentMobilityState::InVehicle {
                        vehicle_id: v_id.clone(),
                        seat_index: *seat_index,
                    };
                    dirty_agents.0.insert(a_entity);
                    break;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase B: ALIGHTING
    // ------------------------------------------------------------------
    // B.1 — collect (vehicle_entity, vehicle_id, end-of-link stop_id, occupants)
    // for every vehicle parked at an end-of-link stop.
    let mut alighting_candidates: Vec<(Entity, VehicleId, StopId, Vec<AgentId>)> = Vec::new();
    {
        let vehicles = sets.p1();
        for (v_entity, v_stable, v_occ, _cap, v_pos) in vehicles.iter() {
            let stop_match = stops.0.values().find(|stop| {
                stop.route_id == v_pos.route_id
                    && stop.link_index == v_pos.link_index
                    && (stop.progress - v_pos.progress).abs() < 1e-6
                    && (stop.progress - 1.0).abs() < 1e-6
            });
            if let Some(stop) = stop_match {
                alighting_candidates.push((
                    v_entity,
                    v_stable.0.clone(),
                    stop.id.clone(),
                    v_occ.0.clone(),
                ));
            }
        }
    }

    // B.2 — for each occupant, check whether its current plan stage is a
    // RideToStop targeting this stop AND its state is InVehicle for this vehicle.
    let mut to_alight: Vec<(Entity, VehicleId, StopId, AgentId)> = Vec::new();
    {
        let agents = sets.p0();
        for (v_entity, v_id, stop_id, occupants) in &alighting_candidates {
            for agent_id in occupants {
                for (_a_entity, a_stable, a_state, a_plan) in agents.iter() {
                    if &a_stable.0 == agent_id {
                        let stage = a_plan.stages.get(a_plan.cursor);
                        let matches_alight = matches!(
                            stage,
                            Some(PlanStage::RideToStop { stop_id: target, .. }) if target == stop_id
                        );
                        let in_this_vehicle = matches!(
                            &a_state.0,
                            AgentMobilityState::InVehicle { vehicle_id, .. } if vehicle_id == v_id
                        );
                        if matches_alight && in_this_vehicle {
                            to_alight.push((
                                *v_entity,
                                v_id.clone(),
                                stop_id.clone(),
                                agent_id.clone(),
                            ));
                        }
                        break;
                    }
                }
            }
        }
    }

    // B.3 — apply alighting mutations (vehicle drops occupant, agent advances plan).
    for (v_entity, v_id, stop_id, agent_id) in &to_alight {
        {
            let mut vehicles = sets.p1();
            if let Ok((_, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
                v_occ.0.retain(|x| x != agent_id);
                dirty_vehicles.0.insert(*v_entity);
            }
        }
        {
            let mut agents = sets.p0();
            for (a_entity, a_stable, mut a_state, mut a_plan) in agents.iter_mut() {
                if &a_stable.0 == agent_id {
                    a_plan.cursor += 1;
                    let next = a_plan.stages.get(a_plan.cursor).cloned();
                    a_state.0 = match next {
                        Some(PlanStage::WalkToActivity { link_id, .. }) => {
                            AgentMobilityState::Walking {
                                link_id,
                                progress: 0.0,
                            }
                        }
                        Some(PlanStage::Activity { activity_id }) => {
                            a_plan.cursor += 1;
                            AgentMobilityState::AtActivity { activity_id }
                        }
                        _ => AgentMobilityState::Alighting {
                            vehicle_id: v_id.clone(),
                            stop_id: stop_id.clone(),
                        },
                    };
                    dirty_agents.0.insert(a_entity);
                    break;
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn compute_world_coord_system(
    mut agents: Query<
        (&AgentMobilityStateComponent, &mut Position),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (&RoutePosition, &mut Position),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    routes: Res<Routes>,
    stops: Res<Stops>,
    link_polylines: Res<LinkPolylines>,
) {
    // Vehicles first.
    for (rp, mut pos) in vehicles.iter_mut() {
        let Some(route) = routes.0.get(&rp.route_id) else {
            continue;
        };
        let Some(link_id) = route.links.get(rp.link_index) else {
            continue;
        };
        let Some(points) = link_polylines.0.get(link_id) else {
            continue;
        };
        let (x, y) = coord_at_progress(points, rp.progress);
        pos.x = x;
        pos.y = y;
    }

    // Agents.
    for (state, mut pos) in agents.iter_mut() {
        let coord = match &state.0 {
            AgentMobilityState::Walking { link_id, progress } => link_polylines
                .0
                .get(link_id)
                .map(|p| coord_at_progress(p, *progress)),
            AgentMobilityState::WaitingAtStop { stop_id }
            | AgentMobilityState::Boarding { stop_id, .. }
            | AgentMobilityState::Alighting { stop_id, .. } => stops.0.get(stop_id).and_then(|s| {
                let route = routes.0.get(&s.route_id)?;
                let link_id = route.links.get(s.link_index)?;
                let points = link_polylines.0.get(link_id)?;
                Some(coord_at_progress(points, s.progress))
            }),
            // InVehicle and AtActivity: leave Position unchanged.
            _ => None,
        };
        if let Some((x, y)) = coord {
            pos.x = x;
            pos.y = y;
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn compute_direction_system(
    mut agents: Query<
        (&AgentMobilityStateComponent, &mut Direction),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (&RoutePosition, &mut Direction),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    routes: Res<Routes>,
    link_polylines: Res<LinkPolylines>,
) {
    for (rp, mut dir) in vehicles.iter_mut() {
        let Some(route) = routes.0.get(&rp.route_id) else {
            continue;
        };
        let Some(link_id) = route.links.get(rp.link_index) else {
            continue;
        };
        let Some(points) = link_polylines.0.get(link_id) else {
            continue;
        };
        dir.0 = dir_at_progress(points, rp.progress);
    }
    for (state, mut dir) in agents.iter_mut() {
        if let AgentMobilityState::Walking { link_id, progress } = &state.0
            && let Some(points) = link_polylines.0.get(link_id)
        {
            dir.0 = dir_at_progress(points, *progress);
        }
        // Other states: keep current Direction unchanged.
    }
}

pub fn tick_increment_system(mut tick: ResMut<Tick>) {
    tick.0 += 1;
}

pub fn track_chunk_populations_system(
    agents: Query<&Position, With<AgentMarker>>,
    vehicles: Query<&Position, With<VehicleMarker>>,
    flow_cells: Res<FlowCells>,
    mut populations: ResMut<ChunkPopulations>,
) {
    populations.0.clear();
    for pos in agents.iter() {
        let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
        *populations.0.entry(chunk).or_insert(0) += 1;
    }
    for pos in vehicles.iter() {
        let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
        *populations.0.entry(chunk).or_insert(0) += 1;
    }
    for (chunk, cell) in &flow_cells.0 {
        let aggregate = cell.population.floor().max(0.0) as u32;
        if aggregate > 0 {
            *populations.0.entry(*chunk).or_insert(0) += aggregate;
        }
    }
}

pub fn classify_activity_system(
    subscribers: Res<ChunkSubscribers>,
    populations: Res<ChunkPopulations>,
    mut activities: ResMut<ChunkActivities>,
    mut cooldowns: ResMut<ChunkActivityCooldowns>,
    mut transitions: ResMut<ChunkTransitions>,
) {
    transitions.0.clear();
    let candidate_chunks: std::collections::HashSet<crate::ids::ChunkCoord> = subscribers
        .0
        .keys()
        .copied()
        .chain(populations.0.keys().copied())
        .chain(activities.0.keys().copied())
        .collect();

    for chunk in candidate_chunks {
        let subs = subscribers.0.get(&chunk).copied().unwrap_or(0);
        let pop = populations.0.get(&chunk).copied().unwrap_or(0);
        let previous = activities.0.get(&chunk).copied().unwrap_or(MobilityActivity::Asleep);
        let cooldown_now = cooldowns.0.get(&chunk).copied().unwrap_or(0);

        let next = classify_chunk_mobility_activity(subs, pop, previous, cooldown_now);

        if next != previous {
            transitions.0.push((chunk, previous, next));
            cooldowns.0.insert(chunk, ACTIVITY_HYSTERESIS_TICKS);
        } else if cooldown_now > 0 {
            cooldowns.0.insert(chunk, cooldown_now - 1);
        }
        activities.0.insert(chunk, next);
    }

    activities.0.retain(|chunk, activity| {
        !matches!(activity, MobilityActivity::Asleep)
            || subscribers.0.contains_key(chunk)
            || populations.0.contains_key(chunk)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increment_system_advances_tick_by_one_per_schedule_run() {
        let mut world = World::new();
        world.insert_resource(Tick(0));
        world.insert_resource(Routes::default());
        world.insert_resource(Stops::default());
        world.insert_resource(LinkPolylines::default());
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());

        let mut schedule = Schedule::default();
        install_systems(&mut schedule);
        schedule.run(&mut world);
        assert_eq!(world.resource::<Tick>().0, 1);
        schedule.run(&mut world);
        assert_eq!(world.resource::<Tick>().0, 2);
    }

    #[test]
    fn stop_arrival_transitions_walking_agent_to_waiting_at_stop() {
        use crate::ids::{AgentId, LinkId, RouteId, StopId};
        use crate::mobility::records::{AgentMobilityState, PlanStage, StopRecord};
        use std::collections::VecDeque;

        let mut world = World::new();
        world.insert_resource(DirtyAgents::default());

        let mut stops = Stops::default();
        stops.0.insert(
            StopId("s:1".into()),
            StopRecord {
                id: StopId("s:1".into()),
                route_id: RouteId("r:1".into()),
                link_index: 0,
                progress: 1.0,
                waiting_agents: VecDeque::new(),
            },
        );
        world.insert_resource(stops);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:1".into()),
                    progress: 1.0,
                }),
                WalkPlan {
                    stages: vec![PlanStage::WalkToStop {
                        link_id: LinkId("l:1".into()),
                        stop_id: StopId("s:1".into()),
                    }],
                    cursor: 0,
                },
                WalkSpeed(0.1),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(stop_arrival_system);
        schedule.run(&mut world);

        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        match &state.0 {
            AgentMobilityState::WaitingAtStop { stop_id } => {
                assert_eq!(stop_id, &StopId("s:1".into()));
            }
            other => panic!("expected WaitingAtStop, got {other:?}"),
        }
        let plan = world.get::<WalkPlan>(entity).unwrap();
        assert_eq!(plan.cursor, 1);
        let stop = world
            .resource::<Stops>()
            .0
            .get(&StopId("s:1".into()))
            .unwrap();
        assert_eq!(stop.waiting_agents.front(), Some(&AgentId("a:1".into())));
        assert!(world.resource::<DirtyAgents>().0.contains(&entity));
    }

    #[test]
    fn boarding_system_moves_waiting_agent_into_matching_vehicle() {
        use crate::ids::{AgentId, RouteId, StopId, VehicleId};
        use crate::mobility::records::{AgentMobilityState, StopRecord, VehicleKind};
        use std::collections::VecDeque;

        let mut world = World::new();
        world.insert_resource(Tick(0));
        world.insert_resource(LinkPolylines::default());
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());

        let mut stops = Stops::default();
        stops.0.insert(
            StopId("s:1".into()),
            StopRecord {
                id: StopId("s:1".into()),
                route_id: RouteId("r:1".into()),
                link_index: 0,
                progress: 0.0,
                waiting_agents: VecDeque::from(vec![AgentId("a:1".into())]),
            },
        );
        world.insert_resource(stops);
        world.insert_resource(Routes::default());

        let agent_entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::WaitingAtStop {
                    stop_id: StopId("s:1".into()),
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.05),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();
        let vehicle_entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    route_id: RouteId("r:1".into()),
                    link_index: 0,
                    progress: 0.0,
                    speed: 0.1,
                },
                Capacity(4),
                Occupants(vec![]),
                DwellTicksRemaining(0),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(boarding_alighting_system);
        schedule.run(&mut world);

        let agent_state = world
            .get::<AgentMobilityStateComponent>(agent_entity)
            .unwrap();
        match &agent_state.0 {
            AgentMobilityState::InVehicle {
                vehicle_id,
                seat_index,
            } => {
                assert_eq!(vehicle_id, &VehicleId("v:1".into()));
                assert_eq!(*seat_index, 0);
            }
            other => panic!("expected InVehicle, got {other:?}"),
        }
        let occ = world.get::<Occupants>(vehicle_entity).unwrap();
        assert_eq!(occ.0, vec![AgentId("a:1".into())]);
        let stop = world
            .resource::<Stops>()
            .0
            .get(&StopId("s:1".into()))
            .unwrap();
        assert!(stop.waiting_agents.is_empty());
        assert!(world.resource::<DirtyAgents>().0.contains(&agent_entity));
        assert!(
            world
                .resource::<DirtyVehicles>()
                .0
                .contains(&vehicle_entity)
        );
    }

    #[test]
    fn alighting_system_drops_occupant_at_end_of_link_destination_stop() {
        use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};
        use crate::mobility::records::{AgentMobilityState, PlanStage, StopRecord, VehicleKind};
        use std::collections::VecDeque;

        let mut world = World::new();
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());

        let mut stops = Stops::default();
        stops.0.insert(
            StopId("s:end".into()),
            StopRecord {
                id: StopId("s:end".into()),
                route_id: RouteId("r:1".into()),
                link_index: 0,
                progress: 1.0,
                waiting_agents: VecDeque::new(),
            },
        );
        world.insert_resource(stops);

        let agent_entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::InVehicle {
                    vehicle_id: VehicleId("v:1".into()),
                    seat_index: 0,
                }),
                WalkPlan {
                    stages: vec![
                        PlanStage::RideToStop {
                            route_id: RouteId("r:1".into()),
                            stop_id: StopId("s:end".into()),
                        },
                        PlanStage::WalkToActivity {
                            link_id: LinkId("l:2".into()),
                            activity_id: "home".into(),
                        },
                    ],
                    cursor: 0,
                },
                WalkSpeed(0.05),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();
        let vehicle_entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    route_id: RouteId("r:1".into()),
                    link_index: 0,
                    progress: 1.0,
                    speed: 0.1,
                },
                Capacity(4),
                Occupants(vec![AgentId("a:1".into())]),
                DwellTicksRemaining(0),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(boarding_alighting_system);
        schedule.run(&mut world);

        let agent_state = world
            .get::<AgentMobilityStateComponent>(agent_entity)
            .unwrap();
        match &agent_state.0 {
            AgentMobilityState::Walking { link_id, progress } => {
                assert_eq!(link_id, &LinkId("l:2".into()));
                assert!((*progress - 0.0).abs() < 1e-6);
            }
            other => panic!("expected Walking after alighting, got {other:?}"),
        }
        let plan = world.get::<WalkPlan>(agent_entity).unwrap();
        assert_eq!(plan.cursor, 1);
        let occ = world.get::<Occupants>(vehicle_entity).unwrap();
        assert!(occ.0.is_empty());
        assert!(world.resource::<DirtyAgents>().0.contains(&agent_entity));
        assert!(
            world
                .resource::<DirtyVehicles>()
                .0
                .contains(&vehicle_entity)
        );
    }

    #[test]
    fn walk_advance_advances_progress_by_walk_speed() {
        use crate::ids::{AgentId, LinkId};
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        world.insert_resource(Tick(0));
        world.insert_resource(DirtyAgents::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("link:test".into()),
                    progress: 0.2,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.1),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(walk_advance_system);
        schedule.run(&mut world);

        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        match &state.0 {
            AgentMobilityState::Walking { progress, .. } => {
                assert!(
                    (progress - 0.3).abs() < 1e-6,
                    "progress should be 0.3, got {progress}"
                );
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

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:near".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("link:test".into()),
                    progress: 0.95,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.1),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(walk_advance_system);
        schedule.run(&mut world);

        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        match &state.0 {
            AgentMobilityState::Walking { progress, .. } => {
                assert!(
                    (progress - 1.0).abs() < 1e-6,
                    "progress clamped to 1.0, got {progress}"
                );
            }
            _ => panic!(),
        }
        assert!(world.resource::<DirtyAgents>().0.contains(&entity));
    }

    #[test]
    fn vehicle_advance_decrements_dwell_when_positive() {
        use crate::ids::{RouteId, VehicleId};
        use crate::mobility::records::VehicleKind;

        let mut world = World::new();
        world.insert_resource(Routes::default());
        world.insert_resource(DirtyVehicles::default());

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    route_id: RouteId("r:1".into()),
                    link_index: 0,
                    progress: 0.5,
                    speed: 0.1,
                },
                Capacity(4),
                Occupants(vec![]),
                DwellTicksRemaining(3),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(vehicle_advance_system);
        schedule.run(&mut world);

        let dwell = world.get::<DwellTicksRemaining>(entity).unwrap();
        assert_eq!(dwell.0, 2);
        let pos = world.get::<RoutePosition>(entity).unwrap();
        assert!(
            (pos.progress - 0.5).abs() < 1e-6,
            "progress unchanged during dwell"
        );
        assert!(world.resource::<DirtyVehicles>().0.contains(&entity));
    }

    #[test]
    fn vehicle_advance_progresses_when_not_dwelling() {
        use crate::ids::{LinkId, RouteId, VehicleId};
        use crate::mobility::records::{RouteRecord, VehicleKind};

        let mut world = World::new();
        let mut routes = Routes::default();
        routes.0.insert(
            RouteId("r:1".into()),
            RouteRecord {
                id: RouteId("r:1".into()),
                links: vec![LinkId("l:1".into())],
            },
        );
        world.insert_resource(routes);
        world.insert_resource(DirtyVehicles::default());

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    route_id: RouteId("r:1".into()),
                    link_index: 0,
                    progress: 0.4,
                    speed: 0.1,
                },
                Capacity(4),
                Occupants(vec![]),
                DwellTicksRemaining(0),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(vehicle_advance_system);
        schedule.run(&mut world);

        let pos = world.get::<RoutePosition>(entity).unwrap();
        assert!((pos.progress - 0.5).abs() < 1e-6);
        assert!(world.resource::<DirtyVehicles>().0.contains(&entity));
    }

    #[test]
    fn compute_world_coord_system_writes_position_for_walking_agent() {
        use crate::ids::{AgentId, LinkId};
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        let mut polylines = LinkPolylines::default();
        polylines
            .0
            .insert(LinkId("l:1".into()), vec![(0.0, 0.0), (10.0, 0.0)]);
        world.insert_resource(polylines);
        world.insert_resource(Routes::default());
        world.insert_resource(Stops::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:1".into()),
                    progress: 0.5,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.0),
                Position { x: 99.0, y: 99.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(compute_world_coord_system);
        schedule.run(&mut world);

        let pos = world.get::<Position>(entity).unwrap();
        assert!(
            (pos.x - 5.0).abs() < 1e-3,
            "x at midpoint of 0..10 = 5.0, got {}",
            pos.x
        );
        assert!(pos.y.abs() < 1e-3);
    }

    #[test]
    fn compute_direction_system_writes_direction_for_walking_agent() {
        use crate::ids::{AgentId, LinkId};
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        let mut polylines = LinkPolylines::default();
        polylines
            .0
            .insert(LinkId("l:1".into()), vec![(0.0, 0.0), (10.0, 0.0)]);
        world.insert_resource(polylines);
        world.insert_resource(Routes::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:1".into()),
                    progress: 0.5,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.0),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(compute_direction_system);
        schedule.run(&mut world);

        let dir = world.get::<Direction>(entity).unwrap();
        // East-pointing polyline → DirectionDto::E
        assert_eq!(dir.0, abutown_protocol::DirectionDto::E);
    }

    #[test]
    fn track_chunk_populations_sums_agents_vehicles_and_flow_cells() {
        use crate::ids::*;
        use crate::mobility::lod::FlowCell;
        use crate::mobility::records::{AgentMobilityState, VehicleKind};

        let mut world = World::new();
        let mut flow_cells = FlowCells::default();
        flow_cells.0.insert(ChunkCoord { x: 0, y: 0 }, FlowCell {
            population: 3.7,
            outflow: Vec::new(),
            attractiveness: 1.0,
            last_tick: 0,
        });
        world.insert_resource(flow_cells);
        world.insert_resource(ChunkPopulations::default());

        for n in 0..2 {
            world.spawn((
                AgentMarker,
                StableAgentId(AgentId(format!("a:{n}"))),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l".into()),
                    progress: 0.0,
                }),
                WalkPlan { stages: vec![], cursor: 0 },
                WalkSpeed(0.0),
                Position { x: 40.0, y: 16.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ));
        }
        world.spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Tram),
            RoutePosition { route_id: RouteId("r".into()), link_index: 0, progress: 0.0, speed: 0.0 },
            Capacity(1),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 80.0, y: 16.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(track_chunk_populations_system);
        schedule.run(&mut world);

        let pops = world.resource::<ChunkPopulations>();
        assert_eq!(pops.0.get(&ChunkCoord { x: 1, y: 0 }), Some(&2)); // two agents
        assert_eq!(pops.0.get(&ChunkCoord { x: 2, y: 0 }), Some(&1)); // one vehicle
        assert_eq!(pops.0.get(&ChunkCoord { x: 0, y: 0 }), Some(&3)); // floor(3.7) flow cell
    }

    #[test]
    fn compute_world_coord_writes_position_for_vehicle() {
        use crate::ids::{LinkId, RouteId, VehicleId};
        use crate::mobility::records::{RouteRecord, VehicleKind};

        let mut world = World::new();
        let mut polylines = LinkPolylines::default();
        polylines
            .0
            .insert(LinkId("l:1".into()), vec![(0.0, 0.0), (20.0, 0.0)]);
        world.insert_resource(polylines);
        let mut routes = Routes::default();
        routes.0.insert(
            RouteId("r:1".into()),
            RouteRecord {
                id: RouteId("r:1".into()),
                links: vec![LinkId("l:1".into())],
            },
        );
        world.insert_resource(routes);
        world.insert_resource(Stops::default());

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    route_id: RouteId("r:1".into()),
                    link_index: 0,
                    progress: 0.25,
                    speed: 0.0,
                },
                Capacity(4),
                Occupants(vec![]),
                DwellTicksRemaining(0),
                Position { x: 99.0, y: 99.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(compute_world_coord_system);
        schedule.run(&mut world);

        let pos = world.get::<Position>(entity).unwrap();
        assert!(
            (pos.x - 5.0).abs() < 1e-3,
            "0.25 of 0..20 = 5.0, got {}",
            pos.x
        );
        assert!(pos.y.abs() < 1e-3);
    }

    #[test]
    fn classify_activity_marks_subscribed_chunk_active() {
        use crate::ids::ChunkCoord;

        let mut world = World::new();
        let mut subs = ChunkSubscribers::default();
        subs.0.insert(ChunkCoord { x: 4, y: 4 }, 1);
        world.insert_resource(subs);
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(ChunkActivities::default());
        world.insert_resource(ChunkActivityCooldowns::default());
        world.insert_resource(ChunkTransitions::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(classify_activity_system);
        schedule.run(&mut world);

        let activities = world.resource::<ChunkActivities>();
        assert_eq!(
            activities.0.get(&ChunkCoord { x: 4, y: 4 }),
            Some(&MobilityActivity::Active),
        );
    }

    #[test]
    fn classify_activity_records_transitions_and_starts_cooldown() {
        use crate::ids::ChunkCoord;
        let mut world = World::new();
        let mut subs = ChunkSubscribers::default();
        subs.0.insert(ChunkCoord { x: 0, y: 0 }, 1);
        world.insert_resource(subs);
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(ChunkActivities::default());
        world.insert_resource(ChunkActivityCooldowns::default());
        world.insert_resource(ChunkTransitions::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(classify_activity_system);
        schedule.run(&mut world);

        let transitions = world.resource::<ChunkTransitions>();
        assert_eq!(transitions.0.len(), 1);
        let (chunk, prev, next) = transitions.0[0];
        assert_eq!(chunk, ChunkCoord { x: 0, y: 0 });
        assert_eq!(prev, MobilityActivity::Asleep);
        assert_eq!(next, MobilityActivity::Active);
        let cd = world.resource::<ChunkActivityCooldowns>();
        assert_eq!(cd.0.get(&ChunkCoord { x: 0, y: 0 }), Some(&ACTIVITY_HYSTERESIS_TICKS));
    }
}
