use crate::ids::{AgentId, RouteId, StopId, VehicleId};
use crate::mobility::components::*;
use crate::mobility::lod::{
    ACTIVITY_HYSTERESIS_TICKS, MobilityActivity, classify_chunk_mobility_activity,
};
use crate::mobility::records::{AgentMobilityState, PlanStage};
use crate::mobility::resources::*;
use bevy_ecs::prelude::*;

fn dir_at_progress(points: &[(f32, f32)], progress: f32) -> abutown_protocol::DirectionDto {
    crate::mobility_geometry::direction_at_progress_slice(points, progress)
}

/// Returns true if the chunk containing the entity is Active or Hot.
/// Asleep/Warm chunks are skipped by the Advance/Output systems so only
/// hot entities tick at full fidelity.
fn chunk_is_simulated(pos: &Position, activities: &ChunkActivities) -> bool {
    let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
    matches!(
        activities
            .0
            .get(&chunk)
            .copied()
            .unwrap_or(MobilityActivity::Asleep),
        MobilityActivity::Active | MobilityActivity::Hot,
    )
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum MobilitySet {
    LOD,
    Advance,
    Output,
    Bookkeeping,
}

pub fn install_systems(schedule: &mut Schedule) {
    schedule.configure_sets((
        MobilitySet::LOD,
        MobilitySet::Advance.after(MobilitySet::LOD),
        MobilitySet::Output.after(MobilitySet::Advance),
        MobilitySet::Bookkeeping.after(MobilitySet::Output),
    ));
    // LOD set: population tracking + classification + promote/demote
    schedule.add_systems((
        track_chunk_populations_system.in_set(MobilitySet::LOD),
        classify_activity_system
            .in_set(MobilitySet::LOD)
            .after(track_chunk_populations_system),
        promote_warm_to_active_system
            .in_set(MobilitySet::LOD)
            .after(classify_activity_system),
        demote_active_to_warm_system
            .in_set(MobilitySet::LOD)
            .after(classify_activity_system),
    ));
    // Advance set: existing Phase-5 systems + warm flow
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
    schedule.add_systems((
        update_link_polyline_cache_system.in_set(MobilitySet::Advance),
        walk_advance_system
            .in_set(MobilitySet::Advance)
            .after(update_link_polyline_cache_system),
        boarding_alighting_system
            .in_set(MobilitySet::Advance)
            .after(walk_advance_system),
        stop_arrival_system
            .in_set(MobilitySet::Advance)
            .after(boarding_alighting_system),
        vehicle_advance_system
            .in_set(MobilitySet::Advance)
            .after(stop_arrival_system),
        warm_chunk_flow_system.in_set(MobilitySet::Advance),
        // Output set
        compute_world_coord_system.in_set(MobilitySet::Output),
        compute_direction_system.in_set(MobilitySet::Output),
        // Bookkeeping
        tick_increment_system.in_set(MobilitySet::Bookkeeping),
    ));
}

pub fn walk_advance_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &mut AgentMobilityStateComponent,
            &WalkSpeed,
        ),
        With<AgentMarker>,
    >,
    activities: Res<ChunkActivities>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, mut state, speed) in query.iter_mut() {
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        if let AgentMobilityState::Walking { progress, .. } = &mut state.0 {
            let next = (*progress + speed.0).min(1.0);
            if next != *progress {
                *progress = next;
                dirty.0.insert(entity);
                if next >= 1.0 {
                    commands.entity(entity).insert(NearStop);
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn update_link_polyline_cache_system(
    mut agents: Query<
        (
            Entity,
            &AgentMobilityStateComponent,
            Option<&mut CurrentLinkPolyline>,
        ),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (
            Entity,
            &RoutePosition,
            Option<&mut CurrentLinkPolyline>,
        ),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    routes: Res<Routes>,
    link_polylines: Res<LinkPolylines>,
    mut commands: Commands,
) {
    use std::sync::Arc;

    // Agents: only Walking state has a link_id. Hot path is the steady
    // state where Walking agents stay on the same link tick after tick —
    // pass `want_id` by reference and only clone on the rare cache-miss
    // path. The previous implementation cloned the LinkId for every
    // Walking agent every tick, which at 100k agents cost ~3-4ms of
    // String allocations and exactly cancelled the Output-system win.
    for (entity, state, cached) in agents.iter_mut() {
        let want_id: Option<&crate::ids::LinkId> = match &state.0 {
            AgentMobilityState::Walking { link_id, .. } => Some(link_id),
            _ => None,
        };
        match (want_id, cached) {
            (Some(want_id), Some(mut c)) => {
                if c.link_id != *want_id
                    && let Some(points) = link_polylines.0.get(want_id)
                {
                    c.link_id = want_id.clone();
                    c.points = Arc::new(points.clone());
                }
            }
            (Some(want_id), None) => {
                if let Some(points) = link_polylines.0.get(want_id) {
                    commands.entity(entity).insert(CurrentLinkPolyline {
                        link_id: want_id.clone(),
                        points: Arc::new(points.clone()),
                    });
                }
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<CurrentLinkPolyline>();
            }
            (None, None) => {}
        }
    }

    // Vehicles: their link is routes[route_id].links[link_index]. Same
    // reference-pass optimization as the agent loop.
    for (entity, rp, cached) in vehicles.iter_mut() {
        let want_id: Option<&crate::ids::LinkId> = routes
            .0
            .get(&rp.route_id)
            .and_then(|r| r.links.get(rp.link_index));
        match (want_id, cached) {
            (Some(want_id), Some(mut c)) => {
                if c.link_id != *want_id
                    && let Some(points) = link_polylines.0.get(want_id)
                {
                    c.link_id = want_id.clone();
                    c.points = Arc::new(points.clone());
                }
            }
            (Some(want_id), None) => {
                if let Some(points) = link_polylines.0.get(want_id) {
                    commands.entity(entity).insert(CurrentLinkPolyline {
                        link_id: want_id.clone(),
                        points: Arc::new(points.clone()),
                    });
                }
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<CurrentLinkPolyline>();
            }
            (None, None) => {}
        }
    }
}

pub fn vehicle_advance_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &mut RoutePosition,
            &mut DwellTicksRemaining,
        ),
        With<VehicleMarker>,
    >,
    activities: Res<ChunkActivities>,
    routes: Res<Routes>,
    mut dirty: ResMut<DirtyVehicles>,
) {
    for (entity, world_pos, mut pos, mut dwell) in query.iter_mut() {
        if !chunk_is_simulated(world_pos, &activities) {
            continue;
        }
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

#[allow(clippy::type_complexity)]
pub fn stop_arrival_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &StableAgentId,
            &mut AgentMobilityStateComponent,
            &mut WalkPlan,
        ),
        (With<AgentMarker>, With<NearStop>),
    >,
    activities: Res<ChunkActivities>,
    mut stops: ResMut<Stops>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, stable, mut state, mut plan) in query.iter_mut() {
        // Skip without clearing the marker if the agent's chunk is asleep
        // this tick — we'll retry next tick when the chunk wakes. This
        // matters because walk_advance only inserts NearStop on the tick
        // progress saturates (next != *progress); if we removed the marker
        // here on a non-simulated tick, the agent would be stuck at
        // progress=1.0 forever without ever transitioning state.
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }

        // Chunk is simulated — always remove the marker now so the next
        // tick doesn't revisit this agent, even if the body falls through
        // to the catch-all arm (e.g., empty plan).
        commands.entity(entity).remove::<NearStop>();

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
                &Position,
                &StableAgentId,
                &mut AgentMobilityStateComponent,
                &mut WalkPlan,
            ),
            With<AgentMarker>,
        >,
        Query<
            (
                Entity,
                &Position,
                &StableVehicleId,
                &mut Occupants,
                &Capacity,
                &RoutePosition,
            ),
            With<VehicleMarker>,
        >,
    )>,
    activities: Res<ChunkActivities>,
    agent_index: Res<crate::mobility::resources::AgentIdIndex>,
    mut stops: ResMut<Stops>,
    mut dirty_agents: ResMut<DirtyAgents>,
    mut dirty_vehicles: ResMut<DirtyVehicles>,
) {
    // ----- PHASE A: BOARDING -----

    // A.1 — collect (stop_id, front agent, route/link/progress) for each stop
    // that has at least one waiting agent. Defer the chunk-activity filter to
    // A.2 so we don't pre-pass over all 100k agents.
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

    // A.2 — find a matching vehicle for each candidate. Both the candidate
    // agent AND the matched vehicle must live in an Active/Hot chunk.
    // Two-phase: first lookup candidate agent positions (p0 borrow), then
    // match against vehicles (p1 borrow) — ParamSet only permits one inner
    // query borrow at a time.
    let mut candidates_with_pos: Vec<(StopId, AgentId, RouteId, usize, f32)> = Vec::new();
    {
        let agents = sets.p0();
        for (stop_id, agent_id, route_id, link_index, stop_progress) in boarding_candidates {
            let Some(agent_entity) = agent_index.0.get(&agent_id).copied() else { continue };
            let Ok((_, pos, _, _, _)) = agents.get(agent_entity) else { continue };
            if !chunk_is_simulated(pos, &activities) {
                continue;
            }
            candidates_with_pos.push((stop_id, agent_id, route_id, link_index, stop_progress));
        }
    }

    let mut boardings: Vec<(StopId, AgentId, Entity, VehicleId, u16)> = Vec::new();
    {
        let vehicles = sets.p1();
        for (stop_id, agent_id, route_id, link_index, stop_progress) in candidates_with_pos {
            for (v_entity, v_pos_world, v_stable, v_occ, v_cap, v_pos) in vehicles.iter() {
                if !chunk_is_simulated(v_pos_world, &activities) {
                    continue;
                }
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
            if let Ok((_, _, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
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

    // A.5 — agent-side mutations: state becomes InVehicle. O(1) lookup via index.
    {
        let mut agents = sets.p0();
        for (_stop_id, agent_id, _v_entity, v_id, seat_index) in &boardings {
            let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                continue;
            };
            if let Ok((_, _, _, mut a_state, _)) = agents.get_mut(a_entity) {
                a_state.0 = AgentMobilityState::InVehicle {
                    vehicle_id: v_id.clone(),
                    seat_index: *seat_index,
                };
                dirty_agents.0.insert(a_entity);
            }
        }
    }

    // ----- PHASE B: ALIGHTING -----

    // B.1 — collect (vehicle_entity, vehicle_id, end-of-link stop_id, occupants)
    // for every vehicle parked at an end-of-link stop in an Active/Hot chunk.
    // Pre-index end-of-link stops by (RouteId, link_index) so each vehicle
    // does an O(1) lookup instead of scanning all stops. Original `find()`
    // pattern was the last O(N_vehicles × N_stops) in this system.
    let end_of_link_stops: std::collections::HashMap<(&RouteId, usize), &crate::mobility::records::StopRecord> = stops
        .0
        .values()
        .filter(|s| (s.progress - 1.0).abs() < 1e-6)
        .map(|s| ((&s.route_id, s.link_index), s))
        .collect();

    let mut alighting_candidates: Vec<(Entity, VehicleId, StopId, Vec<AgentId>)> = Vec::new();
    {
        let vehicles = sets.p1();
        for (v_entity, v_pos_world, v_stable, v_occ, _cap, v_pos) in vehicles.iter() {
            if !chunk_is_simulated(v_pos_world, &activities) {
                continue;
            }
            if (v_pos.progress - 1.0).abs() >= 1e-6 {
                continue;
            }
            if let Some(stop) = end_of_link_stops.get(&(&v_pos.route_id, v_pos.link_index)) {
                alighting_candidates.push((
                    v_entity,
                    v_stable.0.clone(),
                    stop.id.clone(),
                    v_occ.0.clone(),
                ));
            }
        }
    }

    // B.2 — for each occupant, check plan stage + state. O(1) lookups via index.
    let mut to_alight: Vec<(Entity, VehicleId, StopId, AgentId)> = Vec::new();
    {
        let agents = sets.p0();
        for (v_entity, v_id, stop_id, occupants) in &alighting_candidates {
            for agent_id in occupants {
                let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                    continue;
                };
                let Ok((_, a_pos, _, a_state, a_plan)) = agents.get(a_entity) else {
                    continue;
                };
                if !chunk_is_simulated(a_pos, &activities) {
                    continue;
                }
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
            }
        }
    }

    // B.3 — apply alighting mutations. O(1) lookup via index.
    for (v_entity, v_id, stop_id, agent_id) in &to_alight {
        {
            let mut vehicles = sets.p1();
            if let Ok((_, _, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
                v_occ.0.retain(|x| x != agent_id);
                dirty_vehicles.0.insert(*v_entity);
            }
        }
        {
            let mut agents = sets.p0();
            let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                continue;
            };
            if let Ok((_, _, _, mut a_state, mut a_plan)) = agents.get_mut(a_entity) {
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
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn compute_world_coord_system(
    mut agents: Query<
        (
            &AgentMobilityStateComponent,
            &mut Position,
            Option<&CurrentLinkPolyline>,
        ),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (
            &RoutePosition,
            &mut Position,
            Option<&CurrentLinkPolyline>,
        ),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    activities: Res<ChunkActivities>,
    routes: Res<Routes>,
    stops: Res<Stops>,
    link_polylines: Res<LinkPolylines>,
) {
    // Equality-guarded writes: bevy's `Mut<T>` marks the component changed
    // on every deref_mut, even if the new value is the same as the old one.
    // Without this guard, `Changed<Position>` fires for every entity every
    // tick and the incremental `track_chunk_populations_system` degenerates
    // into a full rebuild — destroying Task 6's win.
    for (rp, mut pos, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(&pos, &activities) {
            continue;
        }
        let new_xy = if let Some(c) = cached {
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                &c.points, rp.progress,
            ))
        } else {
            crate::mobility::vehicle_world_coord(rp, &routes, &link_polylines)
        };
        if let Some((x, y)) = new_xy
            && (pos.x != x || pos.y != y)
        {
            pos.x = x;
            pos.y = y;
        }
    }
    for (state, mut pos, cached) in agents.iter_mut() {
        if !chunk_is_simulated(&pos, &activities) {
            continue;
        }
        let new_xy = if let (AgentMobilityState::Walking { progress, .. }, Some(c)) =
            (&state.0, cached)
        {
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                &c.points, *progress,
            ))
        } else {
            crate::mobility::agent_world_coord(&state.0, &routes, &stops, &link_polylines)
        };
        if let Some((x, y)) = new_xy
            && (pos.x != x || pos.y != y)
        {
            pos.x = x;
            pos.y = y;
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn compute_direction_system(
    mut agents: Query<
        (
            &Position,
            &AgentMobilityStateComponent,
            &mut Direction,
            Option<&CurrentLinkPolyline>,
        ),
        (With<AgentMarker>, Without<VehicleMarker>),
    >,
    mut vehicles: Query<
        (
            &Position,
            &RoutePosition,
            &mut Direction,
            Option<&CurrentLinkPolyline>,
        ),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    activities: Res<ChunkActivities>,
    routes: Res<Routes>,
    link_polylines: Res<LinkPolylines>,
) {
    for (pos, rp, mut dir, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        if let Some(c) = cached {
            dir.0 = dir_at_progress(&c.points, rp.progress);
            continue;
        }
        // Slow path: resolve link from route table.
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
    for (pos, state, mut dir, cached) in agents.iter_mut() {
        if !chunk_is_simulated(pos, &activities) {
            continue;
        }
        if let AgentMobilityState::Walking { link_id, progress } = &state.0 {
            if let Some(c) = cached {
                dir.0 = dir_at_progress(&c.points, *progress);
            } else if let Some(points) = link_polylines.0.get(link_id) {
                dir.0 = dir_at_progress(points, *progress);
            }
        }
        // Other states: keep current Direction unchanged.
    }
}

pub fn tick_increment_system(mut tick: ResMut<Tick>) {
    tick.0 += 1;
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn track_chunk_populations_system(
    moved_agents: Query<
        (Entity, &Position),
        (With<AgentMarker>, Changed<Position>),
    >,
    moved_vehicles: Query<
        (Entity, &Position),
        (With<VehicleMarker>, Changed<Position>),
    >,
    all_agents: Query<(Entity, &Position), With<AgentMarker>>,
    all_vehicles: Query<(Entity, &Position), With<VehicleMarker>>,
    flow_cells: Res<FlowCells>,
    mut populations: ResMut<ChunkPopulations>,
    mut agents_by_chunk: ResMut<AgentsByChunk>,
    mut vehicles_by_chunk: ResMut<VehiclesByChunk>,
    mut previous: ResMut<crate::mobility::resources::PreviousChunkByEntity>,
    mut prev_flow: ResMut<crate::mobility::resources::PreviousFlowCellContrib>,
) {
    use std::collections::HashMap;

    let first_run = previous.0.is_empty();
    if first_run {
        // First run after world creation / hydration: full rebuild.
        agents_by_chunk.0.clear();
        vehicles_by_chunk.0.clear();
        populations.0.clear();
        for (entity, pos) in all_agents.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            agents_by_chunk.0.entry(chunk).or_default().push(entity);
            previous.0.insert(entity, chunk);
        }
        for (entity, pos) in all_vehicles.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            vehicles_by_chunk.0.entry(chunk).or_default().push(entity);
            previous.0.insert(entity, chunk);
        }
    } else {
        // Step A: undo the previous tick's FlowCell aggregate so the
        // entity-count deltas below operate on a clean entity-only base.
        for (chunk, amount) in prev_flow.0.drain() {
            if let Some(p) = populations.0.get_mut(&chunk) {
                *p = p.saturating_sub(amount);
            }
        }

        // Step B: incremental rebucketing of moved entities.
        for (entity, pos) in moved_agents.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = agents_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            agents_by_chunk.0.entry(new_chunk).or_default().push(entity);
            previous.0.insert(entity, new_chunk);
        }
        for (entity, pos) in moved_vehicles.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            vehicles_by_chunk.0.entry(new_chunk).or_default().push(entity);
            previous.0.insert(entity, new_chunk);
        }

        // Step C: reconcile despawns — any entity in `previous` that no
        // longer has Position is removed from its bucket.
        let stale: Vec<Entity> = previous
            .0
            .keys()
            .copied()
            .filter(|e| all_agents.get(*e).is_err() && all_vehicles.get(*e).is_err())
            .collect();
        for entity in stale {
            if let Some(old_chunk) = previous.0.remove(&entity) {
                if let Some(bucket) = agents_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.retain(|e| *e != entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
        }
    }

    // Step D: re-add current FlowCell aggregate and remember it for next tick.
    let mut current_flow: HashMap<crate::ids::ChunkCoord, u32> = HashMap::new();
    for (chunk, cell) in &flow_cells.0 {
        let aggregate = cell.population.floor().max(0.0) as u32;
        if aggregate > 0 {
            *populations.0.entry(*chunk).or_insert(0) += aggregate;
            current_flow.insert(*chunk, aggregate);
        }
    }
    prev_flow.0 = current_flow;

    // Drop empty buckets so demote doesn't pay for dead entries.
    agents_by_chunk.0.retain(|_, bucket| !bucket.is_empty());
    vehicles_by_chunk.0.retain(|_, bucket| !bucket.is_empty());
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
        let previous = activities
            .0
            .get(&chunk)
            .copied()
            .unwrap_or(MobilityActivity::Asleep);
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

pub fn promote_warm_to_active_system(
    transitions: Res<ChunkTransitions>,
    mut flow_cells: ResMut<FlowCells>,
    link_polylines: Res<LinkPolylines>,
    routes: Res<Routes>,
    stops: Res<Stops>,
    tick: Res<Tick>,
    mut commands: Commands,
) {
    for (chunk, prev, next) in &transitions.0 {
        if *prev != MobilityActivity::Warm {
            continue;
        }
        if !matches!(next, MobilityActivity::Active | MobilityActivity::Hot) {
            continue;
        }
        let Some(cell) = flow_cells.0.get_mut(chunk) else {
            continue;
        };
        let to_spawn = cell.population.floor() as u32;
        if to_spawn == 0 {
            continue;
        }

        // Find a link whose polyline passes through this chunk.
        let mut spawn_link: Option<crate::ids::LinkId> = None;
        for (link_id, points) in &link_polylines.0 {
            if points
                .iter()
                .any(|(x, y)| crate::mobility::chunk_of(*x, *y, 32) == *chunk)
            {
                spawn_link = Some(link_id.clone());
                break;
            }
        }
        let Some(spawn_link) = spawn_link else {
            continue;
        };

        for n in 0..to_spawn {
            let agent_id = crate::ids::AgentId(format!(
                "agent:lod:{}:{}:{}:{}",
                chunk.x, chunk.y, tick.0, n
            ));
            // Deterministic pseudo-random progress in [0, 1).
            let seed = lod_seed(chunk.x, chunk.y, tick.0, n as u64);
            let progress = (seed % 1000) as f32 / 1000.0;
            let sprite_key = format!("pedestrian:{}", seed % 16);
            let spawned_state = crate::mobility::records::AgentMobilityState::Walking {
                link_id: spawn_link.clone(),
                progress,
            };
            let (px, py) = crate::mobility::agent_world_coord(
                &spawned_state,
                &routes,
                &stops,
                &link_polylines,
            )
            .unwrap_or((0.0, 0.0));
            commands.spawn((
                AgentMarker,
                StableAgentId(agent_id),
                AgentMobilityStateComponent(spawned_state),
                WalkPlan {
                    stages: vec![crate::mobility::records::PlanStage::Activity {
                        activity_id: format!("activity:lod:{}:{}:{}", chunk.x, chunk.y, n),
                    }],
                    cursor: 0,
                },
                WalkSpeed(0.05),
                Position { x: px, y: py },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(sprite_key),
            ));
        }
        cell.population -= to_spawn as f32;
        cell.outflow.clear();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn demote_active_to_warm_system(
    transitions: Res<ChunkTransitions>,
    agents: Query<&AgentMobilityStateComponent, With<AgentMarker>>,
    agents_by_chunk: Res<AgentsByChunk>,
    vehicles_by_chunk: Res<VehiclesByChunk>,
    routes: Res<Routes>,
    link_polylines: Res<LinkPolylines>,
    stops: Res<Stops>,
    mut flow_cells: ResMut<FlowCells>,
    mut commands: Commands,
) {
    // Trigger on any transition *into* Warm, regardless of the previous state.
    // The legacy `Active|Hot → Warm` restriction missed the production path
    // where agents are seeded directly into chunks (snapshot hydration,
    // `from_network`) and the chunk's very first classification is
    // `Asleep → Warm`. Those agents would otherwise stay alive forever and
    // the per-tick Advance/Output systems would pay the full O(N) cost.
    for (chunk, _prev, next) in &transitions.0 {
        if *next != MobilityActivity::Warm {
            continue;
        }

        let Some(agent_entities) = agents_by_chunk.0.get(chunk) else {
            // No agents in this chunk — nothing to despawn. Vehicles might
            // still be present, fall through to vehicle handling.
            if !vehicles_by_chunk.0.contains_key(chunk) {
                continue;
            }
            despawn_vehicles_into_flow_cell(
                *chunk,
                vehicles_by_chunk
                    .0
                    .get(chunk)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                &mut flow_cells,
                &mut commands,
            );
            continue;
        };

        let mut despawn_count = 0u32;
        let mut outflow_counts: std::collections::HashMap<crate::ids::ChunkCoord, u32> =
            std::collections::HashMap::new();

        for entity in agent_entities {
            let Ok(state) = agents.get(*entity) else {
                continue;
            };
            let dest =
                agent_destination_chunk(state, &routes, &link_polylines, &stops).unwrap_or(*chunk);
            despawn_count += 1;
            *outflow_counts.entry(dest).or_insert(0) += 1;
            commands.entity(*entity).despawn();
        }

        if let Some(vehicle_entities) = vehicles_by_chunk.0.get(chunk) {
            despawn_count += vehicle_entities.len() as u32;
            *outflow_counts.entry(*chunk).or_insert(0) += vehicle_entities.len() as u32;
            for entity in vehicle_entities {
                commands.entity(*entity).despawn();
            }
        }

        if despawn_count == 0 {
            continue;
        }

        let cell = flow_cells.0.entry(*chunk).or_default();
        cell.population += despawn_count as f32;
        for (dest, count) in outflow_counts {
            let rate = count as f32 / 100.0; // amortise over ~100 ticks
            *cell.outflow.entry(dest).or_insert(0.0) += rate;
        }
    }
}

fn despawn_vehicles_into_flow_cell(
    chunk: crate::ids::ChunkCoord,
    vehicle_entities: &[Entity],
    flow_cells: &mut ResMut<FlowCells>,
    commands: &mut Commands,
) {
    if vehicle_entities.is_empty() {
        return;
    }
    let cell = flow_cells.0.entry(chunk).or_default();
    cell.population += vehicle_entities.len() as f32;
    *cell.outflow.entry(chunk).or_insert(0.0) += vehicle_entities.len() as f32 / 100.0;
    for entity in vehicle_entities {
        commands.entity(*entity).despawn();
    }
}

fn agent_destination_chunk(
    state: &AgentMobilityStateComponent,
    routes: &Routes,
    link_polylines: &LinkPolylines,
    stops: &Stops,
) -> Option<crate::ids::ChunkCoord> {
    match &state.0 {
        AgentMobilityState::Walking { link_id, .. } => link_polylines
            .0
            .get(link_id)
            .and_then(|points| points.last())
            .map(|(x, y)| crate::mobility::chunk_of(*x, *y, 32)),
        AgentMobilityState::WaitingAtStop { stop_id }
        | AgentMobilityState::Boarding { stop_id, .. }
        | AgentMobilityState::Alighting { stop_id, .. } => {
            let stop = stops.0.get(stop_id)?;
            let route = routes.0.get(&stop.route_id)?;
            let link_id = route.links.get(stop.link_index)?;
            link_polylines
                .0
                .get(link_id)
                .and_then(|p| p.last())
                .map(|(x, y)| crate::mobility::chunk_of(*x, *y, 32))
        }
        _ => None,
    }
}

pub fn warm_chunk_flow_system(
    tick: Res<Tick>,
    activities: Res<ChunkActivities>,
    mut flow_cells: ResMut<FlowCells>,
) {
    if !tick.0.is_multiple_of(10) {
        return;
    }

    let warm_chunks: Vec<crate::ids::ChunkCoord> = activities
        .0
        .iter()
        .filter(|(_, a)| matches!(a, MobilityActivity::Warm))
        .map(|(c, _)| *c)
        .collect();

    let mut transfers: Vec<(crate::ids::ChunkCoord, crate::ids::ChunkCoord, f32)> = Vec::new();
    for chunk in &warm_chunks {
        let Some(cell) = flow_cells.0.get(chunk) else {
            continue;
        };
        for (dest, rate) in &cell.outflow {
            let delta = (rate * 10.0).min(cell.population);
            if delta > 0.0 {
                transfers.push((*chunk, *dest, delta));
            }
        }
    }
    for (from, to, delta) in transfers {
        if let Some(cell) = flow_cells.0.get_mut(&from) {
            cell.population = (cell.population - delta).max(0.0);
            cell.last_tick = tick.0;
        }
        let dest_cell = flow_cells.0.entry(to).or_default();
        dest_cell.population += delta;
        dest_cell.last_tick = tick.0;
    }
}

fn lod_seed(x: i32, y: i32, tick: u64, n: u64) -> u64 {
    // FNV-1a hash for deterministic seeding (does NOT depend on RandomState).
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in (x as u32)
        .to_le_bytes()
        .iter()
        .chain((y as u32).to_le_bytes().iter())
        .chain(tick.to_le_bytes().iter())
        .chain(n.to_le_bytes().iter())
    {
        h ^= *byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a `ChunkActivities` resource pre-populated so that every chunk
    /// in a generous range around the origin is `Active`. Tests that exercise
    /// the LOD-filtered Advance/Output systems use this so the filter doesn't
    /// skip their fixtures.
    fn all_active() -> ChunkActivities {
        let mut a = ChunkActivities::default();
        for x in -10..=20 {
            for y in -10..=20 {
                a.0.insert(crate::ids::ChunkCoord { x, y }, MobilityActivity::Active);
            }
        }
        a
    }

    #[test]
    fn tick_increment_system_advances_tick_by_one_per_schedule_run() {
        let mut world = World::new();
        world.insert_resource(Tick(0));
        world.insert_resource(Routes::default());
        world.insert_resource(Stops::default());
        world.insert_resource(LinkPolylines::default());
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());
        world.insert_resource(all_active());
        // Resources required by the new LOD set
        world.insert_resource(ChunkSubscribers::default());
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(AgentsByChunk::default());
        world.insert_resource(VehiclesByChunk::default());
        world.insert_resource(ChunkActivityCooldowns::default());
        world.insert_resource(FlowCells::default());
        world.insert_resource(ChunkTransitions::default());
        world.insert_resource(crate::mobility::resources::AgentIdIndex::default());
        world.insert_resource(crate::mobility::resources::VehicleIdIndex::default());
        world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
        world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());

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
        world.insert_resource(all_active());

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
                NearStop,
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
        world.insert_resource(all_active());

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
        world.insert_resource(crate::mobility::resources::AgentIdIndex::default());

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
        world
            .resource_mut::<crate::mobility::resources::AgentIdIndex>()
            .0
            .insert(AgentId("a:1".into()), agent_entity);
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
        world.insert_resource(all_active());

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
        world.insert_resource(crate::mobility::resources::AgentIdIndex::default());

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
        world
            .resource_mut::<crate::mobility::resources::AgentIdIndex>()
            .0
            .insert(AgentId("a:1".into()), agent_entity);
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
        world.insert_resource(all_active());

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
        world.insert_resource(all_active());

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
        world.insert_resource(all_active());

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
        world.insert_resource(all_active());
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
        world.insert_resource(all_active());
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
        world.insert_resource(all_active());
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
        flow_cells.0.insert(
            ChunkCoord { x: 0, y: 0 },
            FlowCell {
                population: 3.7,
                outflow: std::collections::HashMap::new(),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        world.insert_resource(flow_cells);
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(AgentsByChunk::default());
        world.insert_resource(VehiclesByChunk::default());
        world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
        world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());

        for n in 0..2 {
            world.spawn((
                AgentMarker,
                StableAgentId(AgentId(format!("a:{n}"))),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l".into()),
                    progress: 0.0,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
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
            RoutePosition {
                route_id: RouteId("r".into()),
                link_index: 0,
                progress: 0.0,
                speed: 0.0,
            },
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
        world.insert_resource(all_active());
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
        assert_eq!(
            cd.0.get(&ChunkCoord { x: 0, y: 0 }),
            Some(&ACTIVITY_HYSTERESIS_TICKS)
        );
    }

    #[test]
    fn promote_warm_spawns_floor_population_agents() {
        use crate::ids::*;
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let mut world = World::new();
        let chunk = ChunkCoord { x: 0, y: 0 };

        let mut flow = FlowCells::default();
        flow.0.insert(
            chunk,
            FlowCell {
                population: 3.7,
                outflow: std::collections::HashMap::new(),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        world.insert_resource(flow);

        let mut polylines = LinkPolylines::default();
        polylines
            .0
            .insert(LinkId("l:0".into()), vec![(10.0, 10.0), (20.0, 10.0)]);
        world.insert_resource(polylines);

        let mut transitions = ChunkTransitions::default();
        transitions
            .0
            .push((chunk, MobilityActivity::Warm, MobilityActivity::Active));
        world.insert_resource(transitions);

        world.insert_resource(Tick(100));
        world.insert_resource(Routes::default());
        world.insert_resource(Stops::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(promote_warm_to_active_system);
        schedule.run(&mut world);

        let mut query = world.query_filtered::<Entity, With<AgentMarker>>();
        let spawned: Vec<Entity> = query.iter(&world).collect();
        assert_eq!(spawned.len(), 3);

        let cell = world.resource::<FlowCells>().0.get(&chunk).unwrap();
        assert!((cell.population - 0.7).abs() < 1e-6);
    }

    #[test]
    fn demote_active_to_warm_collapses_agents_into_flow_cell() {
        use crate::ids::*;
        use crate::mobility::lod::MobilityActivity;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        let chunk = ChunkCoord { x: 0, y: 0 };

        let mut polylines = LinkPolylines::default();
        polylines
            .0
            .insert(LinkId("l:end".into()), vec![(5.0, 5.0), (40.0, 5.0)]); // ends in chunk (1, 0)
        world.insert_resource(polylines);
        world.insert_resource(Routes::default());
        world.insert_resource(Stops::default());
        world.insert_resource(FlowCells::default());
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(AgentsByChunk::default());
        world.insert_resource(VehiclesByChunk::default());
        world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
        world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());

        let mut transitions = ChunkTransitions::default();
        transitions
            .0
            .push((chunk, MobilityActivity::Active, MobilityActivity::Warm));
        world.insert_resource(transitions);

        for n in 0..3 {
            world.spawn((
                AgentMarker,
                StableAgentId(AgentId(format!("a:{n}"))),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:end".into()),
                    progress: 0.1,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.05),
                Position {
                    x: 5.0 + n as f32,
                    y: 5.0,
                },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ));
        }

        let mut schedule = Schedule::default();
        schedule.add_systems((
            track_chunk_populations_system,
            demote_active_to_warm_system.after(track_chunk_populations_system),
        ));
        schedule.run(&mut world);

        let cell = world
            .resource::<FlowCells>()
            .0
            .get(&chunk)
            .expect("flow cell created");
        assert!((cell.population - 3.0).abs() < 1e-6);
        let dest = ChunkCoord { x: 1, y: 0 };
        assert!(
            cell.outflow.contains_key(&dest),
            "outflow should target end-of-link chunk"
        );

        let remaining: u32 = {
            let mut q = world.query_filtered::<Entity, With<AgentMarker>>();
            q.iter(&world).count() as u32
        };
        assert_eq!(remaining, 0, "agents despawned");
    }

    #[test]
    fn warm_chunk_flow_transfers_population_between_chunks() {
        use crate::ids::ChunkCoord;
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let mut world = World::new();
        world.insert_resource(Tick(10));
        let mut activities = ChunkActivities::default();
        activities
            .0
            .insert(ChunkCoord { x: 0, y: 0 }, MobilityActivity::Warm);
        world.insert_resource(activities);

        let mut flow = FlowCells::default();
        flow.0.insert(
            ChunkCoord { x: 0, y: 0 },
            FlowCell {
                population: 10.0,
                outflow: std::collections::HashMap::from([(ChunkCoord { x: 1, y: 0 }, 0.5)]),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        world.insert_resource(flow);

        let mut schedule = Schedule::default();
        schedule.add_systems(warm_chunk_flow_system);
        schedule.run(&mut world);

        let cells = world.resource::<FlowCells>();
        let src = cells.0.get(&ChunkCoord { x: 0, y: 0 }).unwrap();
        let dst = cells.0.get(&ChunkCoord { x: 1, y: 0 }).unwrap();
        assert!((src.population - 5.0).abs() < 1e-3);
        assert!((dst.population - 5.0).abs() < 1e-3);
    }

    #[test]
    fn warm_chunk_flow_skips_non_multiple_of_10_ticks() {
        use crate::ids::ChunkCoord;
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let mut world = World::new();
        world.insert_resource(Tick(5));
        let mut activities = ChunkActivities::default();
        activities
            .0
            .insert(ChunkCoord { x: 0, y: 0 }, MobilityActivity::Warm);
        world.insert_resource(activities);

        let mut flow = FlowCells::default();
        flow.0.insert(
            ChunkCoord { x: 0, y: 0 },
            FlowCell {
                population: 10.0,
                outflow: std::collections::HashMap::from([(ChunkCoord { x: 1, y: 0 }, 0.5)]),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        world.insert_resource(flow);

        let mut schedule = Schedule::default();
        schedule.add_systems(warm_chunk_flow_system);
        schedule.run(&mut world);

        let cells = world.resource::<FlowCells>();
        let src = cells.0.get(&ChunkCoord { x: 0, y: 0 }).unwrap();
        assert!(
            (src.population - 10.0).abs() < 1e-3,
            "skipped on non-multiple-of-10 tick"
        );
    }

    #[test]
    fn promote_warm_is_deterministic_across_runs() {
        use crate::ids::*;
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        fn run_promote() -> Vec<String> {
            let mut world = World::new();
            let chunk = ChunkCoord { x: 2, y: 3 };
            let mut flow = FlowCells::default();
            flow.0.insert(
                chunk,
                FlowCell {
                    population: 5.0,
                    outflow: std::collections::HashMap::new(),
                    attractiveness: 1.0,
                    last_tick: 0,
                },
            );
            world.insert_resource(flow);
            let mut polylines = LinkPolylines::default();
            polylines
                .0
                .insert(LinkId("l:0".into()), vec![(70.0, 100.0), (90.0, 100.0)]);
            world.insert_resource(polylines);
            let mut transitions = ChunkTransitions::default();
            transitions
                .0
                .push((chunk, MobilityActivity::Warm, MobilityActivity::Active));
            world.insert_resource(transitions);
            world.insert_resource(Tick(42));
            world.insert_resource(Routes::default());
            world.insert_resource(Stops::default());

            let mut schedule = Schedule::default();
            schedule.add_systems(promote_warm_to_active_system);
            schedule.run(&mut world);

            let mut query = world.query::<&StableAgentId>();
            let mut ids: Vec<String> = query.iter(&world).map(|s| s.0.0.clone()).collect();
            ids.sort();
            ids
        }

        let a = run_promote();
        let b = run_promote();
        assert_eq!(
            a, b,
            "promote must be deterministic across runs (same chunk + tick → same ids)"
        );
    }

    #[test]
    fn walk_advance_skips_agents_in_asleep_chunks() {
        use crate::ids::*;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        world.insert_resource(ChunkActivities::default()); // empty = all Asleep
        world.insert_resource(DirtyAgents::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:0".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:0".into()),
                    progress: 0.5,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.1),
                Position { x: 100.0, y: 100.0 },
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
                    (progress - 0.5).abs() < 1e-6,
                    "progress unchanged in Asleep chunk"
                );
            }
            _ => panic!(),
        }
        assert!(
            !world.resource::<DirtyAgents>().0.contains(&entity),
            "Asleep-chunk agent must not be marked dirty"
        );
    }

    #[test]
    fn walk_advance_advances_agents_in_active_chunks() {
        use crate::ids::*;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        let mut activities = ChunkActivities::default();
        // Position (100, 100) → chunk (3, 3) for chunk_size = 32.
        activities
            .0
            .insert(ChunkCoord { x: 3, y: 3 }, MobilityActivity::Active);
        world.insert_resource(activities);
        world.insert_resource(DirtyAgents::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:0".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:0".into()),
                    progress: 0.5,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.1),
                Position { x: 100.0, y: 100.0 },
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
                assert!((progress - 0.6).abs() < 1e-6);
            }
            _ => panic!(),
        }
        assert!(world.resource::<DirtyAgents>().0.contains(&entity));
    }

    #[test]
    fn walk_advance_inserts_near_stop_marker_when_progress_saturates() {
        use crate::ids::{AgentId, LinkId};

        let mut world = World::new();
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:1".into()),
                    progress: 0.99,
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

        let mut schedule = Schedule::default();
        schedule.add_systems(walk_advance_system);
        schedule.run(&mut world);

        assert!(
            world.get::<NearStop>(entity).is_some(),
            "walk_advance should add NearStop when progress saturates to 1.0"
        );
    }

    #[test]
    fn stop_arrival_removes_near_stop_marker_after_transition() {
        use crate::ids::{AgentId, LinkId, RouteId, StopId};
        use crate::mobility::records::StopRecord;
        use std::collections::VecDeque;

        let mut world = World::new();
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());

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
                WalkSpeed(0.05),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
                NearStop,
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(stop_arrival_system);
        schedule.run(&mut world);

        assert!(
            world.get::<NearStop>(entity).is_none(),
            "stop_arrival should remove NearStop after state transition"
        );
    }

    #[test]
    fn current_link_polyline_invalidates_on_walker_link_change() {
        use crate::ids::{AgentId, LinkId};
        use std::sync::Arc;

        let mut world = World::new();
        world.insert_resource(LinkPolylines::default());
        world.insert_resource(all_active());

        let mut links = LinkPolylines::default();
        links.0.insert(LinkId("l:a".into()), vec![(0.0, 0.0), (10.0, 0.0)]);
        links.0.insert(LinkId("l:b".into()), vec![(0.0, 0.0), (0.0, 10.0)]);
        world.insert_resource(links);
        world.insert_resource(Routes::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l:a".into()),
                    progress: 0.0,
                }),
                WalkPlan { stages: vec![], cursor: 0 },
                WalkSpeed(0.05),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
                CurrentLinkPolyline {
                    link_id: LinkId("l:a".into()),
                    points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(update_link_polyline_cache_system);

        // Tick 1: cache already matches → no change.
        schedule.run(&mut world);
        assert_eq!(
            world.get::<CurrentLinkPolyline>(entity).unwrap().link_id,
            LinkId("l:a".into())
        );

        // Mutate the agent to a different link.
        if let Some(mut s) = world.get_mut::<AgentMobilityStateComponent>(entity) {
            s.0 = AgentMobilityState::Walking {
                link_id: LinkId("l:b".into()),
                progress: 0.0,
            };
        }
        schedule.run(&mut world);
        assert_eq!(
            world.get::<CurrentLinkPolyline>(entity).unwrap().link_id,
            LinkId("l:b".into())
        );
        let cached = world.get::<CurrentLinkPolyline>(entity).unwrap();
        assert_eq!(cached.points.as_ref(), &vec![(0.0, 0.0), (0.0, 10.0)]);
    }

    #[test]
    fn current_link_polyline_invalidates_on_vehicle_link_change() {
        use crate::ids::{LinkId, RouteId, VehicleId};
        use crate::mobility::records::{RouteRecord, VehicleKind};
        use std::sync::Arc;

        let mut world = World::new();
        world.insert_resource(all_active());

        let mut routes = Routes::default();
        routes.0.insert(
            RouteId("r:1".into()),
            RouteRecord {
                id: RouteId("r:1".into()),
                links: vec![LinkId("l:a".into()), LinkId("l:b".into())],
            },
        );
        world.insert_resource(routes);

        let mut links = LinkPolylines::default();
        links.0.insert(LinkId("l:a".into()), vec![(0.0, 0.0), (10.0, 0.0)]);
        links.0.insert(LinkId("l:b".into()), vec![(0.0, 0.0), (0.0, 10.0)]);
        world.insert_resource(links);

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Car),
                RoutePosition {
                    route_id: RouteId("r:1".into()),
                    link_index: 0,
                    progress: 0.0,
                    speed: 0.1,
                },
                Capacity(1),
                Occupants(vec![]),
                DwellTicksRemaining(0),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
                CurrentLinkPolyline {
                    link_id: LinkId("l:a".into()),
                    points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(update_link_polyline_cache_system);

        if let Some(mut rp) = world.get_mut::<RoutePosition>(entity) {
            rp.link_index = 1;
        }
        schedule.run(&mut world);
        assert_eq!(
            world.get::<CurrentLinkPolyline>(entity).unwrap().link_id,
            LinkId("l:b".into())
        );
    }

    #[test]
    fn incremental_chunk_populations_matches_full_rebuild() {
        use crate::ids::{AgentId, LinkId};
        use crate::mobility::resources::{PreviousChunkByEntity, PreviousFlowCellContrib};

        let mut world = World::new();
        world.insert_resource(FlowCells::default());
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(AgentsByChunk::default());
        world.insert_resource(VehiclesByChunk::default());
        world.insert_resource(PreviousChunkByEntity::default());
        world.insert_resource(PreviousFlowCellContrib::default());

        // Spawn 200 agents scattered across multiple chunks.
        for i in 0..200 {
            let x = (i % 10) as f32 * 35.0; // chunks at 0, 35, 70, ...
            let y = (i / 10) as f32 * 35.0;
            world.spawn((
                AgentMarker,
                StableAgentId(AgentId(format!("a:{i}"))),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: LinkId("l".into()),
                    progress: 0.0,
                }),
                WalkPlan { stages: vec![], cursor: 0 },
                WalkSpeed(0.05),
                Position { x, y },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
            ));
        }

        let mut schedule = Schedule::default();
        schedule.add_systems(track_chunk_populations_system);

        // Tick 1: full rebuild path (all positions are "new").
        schedule.run(&mut world);

        // Mutate one agent's position to a new chunk.
        let mut q = world.query::<(Entity, &mut Position)>();
        let moved_entity = q.iter_mut(&mut world).next().map(|(e, mut p)| {
            p.x = 999.0;
            p.y = 999.0;
            e
        }).unwrap();

        // Tick 2: incremental path.
        schedule.run(&mut world);
        let after2_incremental: std::collections::HashMap<_, _> = world
            .resource::<AgentsByChunk>()
            .0
            .iter()
            .map(|(c, e)| {
                let mut e = e.clone();
                e.sort_by_key(|x| x.index());
                (*c, e)
            })
            .collect();

        // Compare against a fresh full rebuild from query state.
        let mut reference: std::collections::HashMap<crate::ids::ChunkCoord, Vec<Entity>> =
            std::collections::HashMap::new();
        let mut q2 = world.query::<(Entity, &Position, &AgentMarker)>();
        for (entity, pos, _) in q2.iter(&world) {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            reference.entry(chunk).or_default().push(entity);
        }
        for bucket in reference.values_mut() {
            bucket.sort_by_key(|x| x.index());
        }
        assert_eq!(after2_incremental, reference);
        // Ensure the moved entity actually moved buckets.
        assert!(after2_incremental.values().any(|v| v.contains(&moved_entity)));
    }
}
