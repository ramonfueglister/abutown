use crate::ids::{AgentId, VehicleId};
use crate::mobility::components::*;
use crate::mobility::records::{AgentMobilityState, PlanStage};
use crate::mobility::resources::*;
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk, WarmChunk};
use crate::world::events::{ChunkLod, ChunkLodChanged};
use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use std::collections::HashSet;

fn dir_at_progress(points: &[(f32, f32)], progress: f32) -> abutown_protocol::DirectionDto {
    crate::mobility_geometry::direction_at_progress_slice(points, progress)
}

/// Returns true if the chunk containing the entity is Active or Hot.
/// Asleep/Warm chunks are skipped by the Advance/Output systems so only
/// hot entities tick at full fidelity. Source of truth: chunk-entity LOD
/// markers; this lookup goes through the `SimulatedChunks` derived view
/// refreshed each tick by `refresh_simulated_chunks_system`.
fn chunk_is_simulated(pos: &Position, simulated: &SimulatedChunks) -> bool {
    let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
    simulated.0.contains(&chunk)
}

fn edge_progress_for_node(
    graph: &crate::routing::Graph,
    edge_id: crate::routing::EdgeId,
    node_id: crate::routing::NodeId,
) -> Option<f32> {
    let edge = graph.edge(edge_id);
    let node = graph.node(node_id);
    let mut travelled = 0.0_f32;
    for win in edge.polyline.windows(2) {
        if win[0] == node.position {
            return Some((travelled / edge.length.max(0.001)).clamp(0.0, 1.0));
        }
        let dx = win[1].0 - win[0].0;
        let dy = win[1].1 - win[0].1;
        travelled += (dx * dx + dy * dy).sqrt();
    }
    edge.polyline
        .last()
        .filter(|point| **point == node.position)
        .map(|_| 1.0)
}

fn line_edge_stops_for_node(
    graph: &crate::routing::Graph,
    transit_lines: &crate::routing::TransitLines,
    node_id: crate::routing::NodeId,
) -> Vec<(crate::routing::LineId, usize, f32)> {
    let mut out = Vec::new();
    for line in transit_lines.iter() {
        for (edge_index, edge_id) in line.edges.iter().enumerate() {
            if let Some(progress) = edge_progress_for_node(graph, *edge_id, node_id) {
                out.push((line.id, edge_index, progress));
            }
        }
    }
    out
}

fn current_route_origin(
    graph: &crate::routing::Graph,
    canonical_edge_key: &str,
    progress: f32,
) -> Option<crate::routing::NodeId> {
    let edge_id = crate::mobility::api::edge_by_canonical_key(graph, canonical_edge_key)?;
    let edge = graph.edge(edge_id);
    if progress >= 1.0 {
        Some(edge.to)
    } else {
        Some(edge.from)
    }
}

fn destination_for_stage(
    graph: &crate::routing::Graph,
    stage: &PlanStage,
    spatial: Option<&crate::routing::NodeSpatialIndex>,
) -> Option<crate::routing::NodeId> {
    match stage {
        PlanStage::WalkToStop { stop_id, .. } => graph.node_by_legacy(stop_id),
        PlanStage::WalkToActivity { activity_id, .. } => {
            graph.node_by_legacy(activity_id).or_else(|| {
                let coord = crate::mobility_geometry::activity_geometry(activity_id)?.coord;
                spatial?.nearest(coord)
            })
        }
        _ => None,
    }
}

fn materialize_route_steps(
    graph: &crate::routing::Graph,
    field: &crate::routing::FlowField,
    origin: crate::routing::NodeId,
    initial_mode: crate::routing::ModeState,
) -> Option<Vec<RouteStep>> {
    let mut steps = Vec::new();
    let mut node = origin;
    let mut mode = initial_mode;
    let mut seen = HashSet::new();

    while node != field.destination {
        if !seen.insert((node, mode)) {
            return None;
        }
        let entry = field.entry(node, mode)?;
        let edge_id = entry.next_edge?;
        let edge = graph.edge(edge_id);
        steps.push(RouteStep {
            edge_id,
            mode: entry.next_mode,
            canonical_edge_key: crate::mobility::api::canonical_edge_key(graph, edge_id),
            length: edge.length,
        });
        node = edge.to;
        mode = entry.next_mode;

        if steps.len() > graph.edge_count() {
            return None;
        }
    }

    Some(steps)
}

fn complete_walk_stage_at_destination(
    entity: Entity,
    stable: &StableAgentId,
    state: &mut AgentMobilityStateComponent,
    plan: &mut WalkPlan,
    stage: PlanStage,
    destination: crate::routing::NodeId,
    waiting: &mut crate::routing::WaitingAgents,
    dirty: &mut DirtyAgents,
    commands: &mut Commands,
) -> bool {
    match stage {
        PlanStage::WalkToStop { stop_id, .. } => {
            plan.cursor += 1;
            state.0 = AgentMobilityState::WaitingAtStop { stop_id };
            let already_waiting = waiting
                .queue(destination)
                .map(|queue| queue.contains(&stable.0))
                .unwrap_or(false);
            if !already_waiting {
                waiting.enqueue(destination, stable.0.clone());
            }
            commands.entity(entity).remove::<(ActiveRoute, NearStop)>();
            dirty.0.insert(entity);
            true
        }
        PlanStage::WalkToActivity { activity_id, .. } => {
            plan.cursor += 1;
            state.0 = AgentMobilityState::AtActivity { activity_id };
            commands.entity(entity).remove::<(ActiveRoute, NearStop)>();
            dirty.0.insert(entity);
            true
        }
        _ => false,
    }
}

fn invalidate_active_route(
    entity: Entity,
    stats: &mut RouteAssignmentStats,
    dirty: &mut DirtyAgents,
    commands: &mut Commands,
) {
    stats.failed += 1;
    commands.entity(entity).remove::<ActiveRoute>();
    dirty.0.insert(entity);
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum MobilitySet {
    LOD,
    Advance,
    Output,
    Bookkeeping,
}

pub fn install_systems(schedule: &mut Schedule) {
    use crate::world::schedule::CoreSet;
    schedule.configure_sets((
        MobilitySet::LOD,
        MobilitySet::Advance.after(MobilitySet::LOD),
        MobilitySet::Output.after(MobilitySet::Advance),
        MobilitySet::Bookkeeping.after(MobilitySet::Output),
    ));
    // Mobility's LOD set drains the `ChunkLodChanged` event stream produced
    // by `reclassify_chunk_lod_system` (in `CoreSet::LodReclassify`), so it
    // must run after the reclassifier. The population tracker runs BEFORE
    // the reclassifier so populated-but-unsubscribed chunks get classified
    // (and demoted) in the same tick they were seeded.
    schedule.configure_sets(MobilitySet::LOD.after(CoreSet::LodReclassify));
    schedule.configure_sets(MobilitySet::Bookkeeping.before(CoreSet::EventEmit));
    // Population tracking is intentionally NOT in MobilitySet::LOD: it must
    // run BEFORE `CoreSet::LodReclassify` so reclassify sees same-tick
    // populations and can emit the Asleep→Warm transition that drives
    // demote within the same schedule run.
    schedule.add_systems(track_chunk_populations_system.before(CoreSet::LodReclassify));
    schedule.add_systems((
        refresh_simulated_chunks_system.in_set(MobilitySet::LOD),
        consume_chunk_lod_transitions_system.in_set(MobilitySet::LOD),
        promote_warm_to_active_system
            .in_set(MobilitySet::LOD)
            .after(consume_chunk_lod_transitions_system),
        demote_active_to_warm_system
            .in_set(MobilitySet::LOD)
            .after(consume_chunk_lod_transitions_system),
    ));
    // Advance set: existing Phase-5 systems + warm flow
    // Ordering within Advance (each step observes the previous step's
    // output, but is staged so that "newly waiting" agents are not
    // immediately boarded in the same tick they arrived at the stop, and
    // "just alighted" agents do not immediately walk further in the same
    // tick they got off):
    //
    //   1. route_assignment    — assign graph routes to un-routed walkers.
    //   2. route_advance       — move completed route edges to the next edge.
    //   3. update_link_cache   — refresh edge polylines after route changes.
    //   4. walk_advance        — push Walking agents along their link.
    //   5. boarding_alighting  — apply Phase-3 boarding + alighting using
    //                            the PRE-stop_arrival waiting queue. This
    //                            means an agent that arrived at the stop
    //                            in step 6 of this same tick won't board
    //                            until the next tick.
    //   6. stop_arrival        — convert progress=1.0 walkers into
    //                            WaitingAtStop / AtActivity.
    //   7. vehicle_advance     — decrement dwell or push progress.
    schedule.add_systems((
        route_assignment_system.in_set(MobilitySet::Advance),
        route_advance_system
            .in_set(MobilitySet::Advance)
            .after(route_assignment_system),
        update_link_polyline_cache_system
            .in_set(MobilitySet::Advance)
            .after(route_advance_system),
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

#[allow(clippy::type_complexity)]
pub fn route_assignment_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &StableAgentId,
            &mut AgentMobilityStateComponent,
            &mut WalkPlan,
        ),
        (With<AgentMarker>, Without<ActiveRoute>),
    >,
    simulated: Res<SimulatedChunks>,
    graph: Res<crate::routing::Graph>,
    hpa: Option<Res<crate::routing::HpaIndex>>,
    spatial: Option<Res<crate::routing::NodeSpatialIndex>>,
    mut cache: Option<ResMut<crate::routing::FlowFieldCache>>,
    mut stats: ResMut<RouteAssignmentStats>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    let Some(hpa) = hpa else {
        for (_, pos, _, state, _) in query.iter() {
            if chunk_is_simulated(pos, &simulated)
                && matches!(state.0, AgentMobilityState::Walking { .. })
            {
                stats.skipped += 1;
            }
        }
        return;
    };
    let Some(cache) = cache.as_deref_mut() else {
        for (_, pos, _, state, _) in query.iter() {
            if chunk_is_simulated(pos, &simulated)
                && matches!(state.0, AgentMobilityState::Walking { .. })
            {
                stats.skipped += 1;
            }
        }
        return;
    };

    for (entity, pos, stable, mut state, mut plan) in query.iter_mut() {
        let AgentMobilityState::Walking { link_id, progress } = &state.0 else {
            continue;
        };
        let link_id = link_id.clone();
        let progress = *progress;
        if !chunk_is_simulated(pos, &simulated) {
            stats.skipped += 1;
            continue;
        }
        let Some(stage) = plan.stages.get(plan.cursor).cloned() else {
            stats.skipped += 1;
            continue;
        };
        let Some(destination) = destination_for_stage(&graph, &stage, spatial.as_deref()) else {
            stats.failed += 1;
            continue;
        };
        let Some(origin) = current_route_origin(&graph, &link_id, progress) else {
            stats.failed += 1;
            continue;
        };
        if origin == destination {
            if complete_walk_stage_at_destination(
                entity,
                stable,
                &mut state,
                &mut plan,
                stage,
                destination,
                &mut waiting,
                &mut dirty,
                &mut commands,
            ) {
                stats.skipped += 1;
            } else {
                stats.failed += 1;
            }
            continue;
        }
        let profile_key = crate::routing::RoutingProfileKey::Walk;
        let Ok(corridor) = hpa.corridor_between(origin, destination, profile_key) else {
            stats.failed += 1;
            continue;
        };

        let mut corridor_key: Vec<_> = corridor.iter().copied().collect();
        corridor_key.sort_unstable();

        let profile = crate::routing::RoutingProfile::for_key(profile_key);
        let key =
            crate::routing::FlowFieldCacheKey::new(destination, profile_key, 0, &corridor_key);
        let scope = crate::routing::FlowFieldScope::Corridor(corridor);

        let Ok(field) =
            cache.get_or_build_with_cluster_lookup(&graph, key, profile, scope, |node| {
                hpa.cluster_of_node(node)
            })
        else {
            stats.failed += 1;
            continue;
        };
        let Some(steps) =
            materialize_route_steps(&graph, &field, origin, crate::routing::ModeState::Walking)
        else {
            stats.failed += 1;
            continue;
        };
        if steps.is_empty() {
            stats.failed += 1;
            continue;
        }

        let first_step_link_id = steps[0].canonical_edge_key.clone();
        if progress >= 1.0 || link_id != first_step_link_id {
            state.0 = AgentMobilityState::Walking {
                link_id: first_step_link_id,
                progress: 0.0,
            };
            dirty.0.insert(entity);
        }

        commands.entity(entity).insert(ActiveRoute {
            destination,
            profile: profile_key,
            steps,
            cursor: 0,
        });
        stats.assigned += 1;
    }
}

#[allow(clippy::type_complexity)]
pub fn route_advance_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &StableAgentId,
            &mut AgentMobilityStateComponent,
            &mut WalkPlan,
            &mut ActiveRoute,
        ),
        With<AgentMarker>,
    >,
    simulated: Res<SimulatedChunks>,
    graph: Res<crate::routing::Graph>,
    spatial: Option<Res<crate::routing::NodeSpatialIndex>>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    mut stats: ResMut<RouteAssignmentStats>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, stable, mut state, mut plan, mut route) in query.iter_mut() {
        if !chunk_is_simulated(pos, &simulated) {
            continue;
        }
        let AgentMobilityState::Walking { link_id, progress } = &state.0 else {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        };
        let Some(current_step) = route.steps.get(route.cursor).cloned() else {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        };
        if link_id != &current_step.canonical_edge_key {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        }
        let Some(current_edge_id) =
            crate::mobility::api::edge_by_canonical_key(&graph, &current_step.canonical_edge_key)
        else {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        };
        if current_edge_id != current_step.edge_id {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        }
        if *progress < 1.0 {
            continue;
        }

        let next_cursor = route.cursor + 1;
        if let Some(next_step) = route.steps.get(next_cursor).cloned() {
            let Some(next_edge_id) =
                crate::mobility::api::edge_by_canonical_key(&graph, &next_step.canonical_edge_key)
            else {
                invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
                continue;
            };
            if next_edge_id != next_step.edge_id
                || graph.edge(current_edge_id).to != graph.edge(next_edge_id).from
            {
                invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
                continue;
            }
            route.cursor = next_cursor;
            state.0 = AgentMobilityState::Walking {
                link_id: next_step.canonical_edge_key,
                progress: 0.0,
            };
            dirty.0.insert(entity);
            continue;
        }

        if graph.edge(current_edge_id).to != route.destination {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        }

        let Some(stage) = plan.stages.get(plan.cursor).cloned() else {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        };
        if destination_for_stage(&graph, &stage, spatial.as_deref()) != Some(route.destination) {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
            continue;
        }
        if !complete_walk_stage_at_destination(
            entity,
            stable,
            &mut state,
            &mut plan,
            stage,
            route.destination,
            &mut waiting,
            &mut dirty,
            &mut commands,
        ) {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
        }
    }
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
    simulated: Res<SimulatedChunks>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, mut state, speed) in query.iter_mut() {
        if !chunk_is_simulated(pos, &simulated) {
            continue;
        }
        let AgentMobilityState::Walking { progress, .. } = &mut state.0 else {
            continue;
        };
        // `WalkSpeed` is already expressed as progress-per-tick; preserve
        // wire/behavior parity with the legacy integration.
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
        (Entity, &RoutePosition, Option<&mut CurrentLinkPolyline>),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    transit_lines: Res<crate::routing::TransitLines>,
    graph: Res<crate::routing::Graph>,
    mut commands: Commands,
) {
    use std::sync::Arc;

    // Agents: only Walking state has a link_id. Hot path is the steady
    // state where Walking agents stay on the same link tick after tick —
    // pass `want_id` by reference and only clone on the rare cache-miss
    // path. The previous implementation cloned the link id for every
    // Walking agent every tick, which at 100k agents cost ~3-4ms of
    // String allocations and exactly cancelled the Output-system win.
    for (entity, state, cached) in agents.iter_mut() {
        let want_id: Option<&String> = match &state.0 {
            AgentMobilityState::Walking { link_id, .. } => Some(link_id),
            _ => None,
        };
        match (want_id, cached) {
            (Some(want_id), Some(mut c)) => {
                if c.link_id != *want_id
                    && let Some(edge_id) =
                        crate::mobility::api::edge_by_canonical_key(&graph, want_id)
                {
                    let points = graph.edge(edge_id).polyline.clone();
                    c.link_id = want_id.clone();
                    c.points = Arc::new(points);
                }
            }
            (Some(want_id), None) => {
                if let Some(edge_id) = crate::mobility::api::edge_by_canonical_key(&graph, want_id)
                {
                    let points = graph.edge(edge_id).polyline.clone();
                    commands.entity(entity).insert(CurrentLinkPolyline {
                        link_id: want_id.clone(),
                        points: Arc::new(points),
                    });
                }
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<CurrentLinkPolyline>();
            }
            (None, None) => {}
        }
    }

    // Vehicles: resolve the current edge via TransitLines + Graph (the
    // single source of truth post-8b). Synthesize a stable link id from
    // the edge's legacy id (or its numeric EdgeId if the edge has no
    // legacy ancestry) so the cache-miss / cache-hit comparison still
    // works against the existing component shape.
    for (entity, rp, cached) in vehicles.iter_mut() {
        let resolved: Option<(String, Vec<(f32, f32)>)> =
            if (rp.line_id.0 as usize) >= transit_lines.count() {
                None
            } else {
                let line = transit_lines.line(rp.line_id);
                line.edges.get(rp.edge_index).map(|edge_id| {
                    let edge = graph.edge(*edge_id);
                    let lid = edge
                        .legacy_id
                        .clone()
                        .unwrap_or_else(|| format!("edge:{}", edge.id.0));
                    (lid, edge.polyline.clone())
                })
            };
        match (resolved, cached) {
            (Some((want_id, points)), Some(mut c)) => {
                if c.link_id != want_id {
                    c.link_id = want_id;
                    c.points = Arc::new(points);
                }
            }
            (Some((want_id, points)), None) => {
                commands.entity(entity).insert(CurrentLinkPolyline {
                    link_id: want_id,
                    points: Arc::new(points),
                });
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
    simulated: Res<SimulatedChunks>,
    transit_lines: Res<crate::routing::TransitLines>,
    mut dirty: ResMut<DirtyVehicles>,
) {
    for (entity, world_pos, mut pos, mut dwell) in query.iter_mut() {
        if !chunk_is_simulated(world_pos, &simulated) {
            continue;
        }
        // dwell counts down first
        if dwell.0 > 0 {
            dwell.0 -= 1;
            dirty.0.insert(entity);
            continue;
        }
        if (pos.line_id.0 as usize) >= transit_lines.count()
            || transit_lines.line(pos.line_id).edges.is_empty()
        {
            continue;
        }
        if pos.progress >= 1.0 {
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
            Option<&ActiveRoute>,
        ),
        (With<AgentMarker>, With<NearStop>),
    >,
    simulated: Res<SimulatedChunks>,
    graph: Res<crate::routing::Graph>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, pos, stable, mut state, mut plan, active_route) in query.iter_mut() {
        // Skip without clearing the marker if the agent's chunk is asleep
        // this tick — we'll retry next tick when the chunk wakes. This
        // matters because walk_advance only inserts NearStop on the tick
        // progress saturates (next != *progress); if we removed the marker
        // here on a non-simulated tick, the agent would be stuck at
        // progress=1.0 forever without ever transitioning state.
        if !chunk_is_simulated(pos, &simulated) {
            continue;
        }

        // Chunk is simulated — always remove the marker now so the next
        // tick doesn't revisit this agent, even if the body falls through
        // to the catch-all arm (e.g., empty plan).
        commands.entity(entity).remove::<NearStop>();

        if active_route.is_some() {
            continue;
        }

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
                if let Some(node_id) = graph.node_by_legacy(&stop_id) {
                    let already_waiting = waiting
                        .queue(node_id)
                        .map(|queue| queue.contains(&stable.0))
                        .unwrap_or(false);
                    if !already_waiting {
                        waiting.enqueue(node_id, stable.0.clone());
                    }
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
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
    simulated: Res<SimulatedChunks>,
    agent_index: Res<crate::mobility::resources::AgentIdIndex>,
    graph: Res<crate::routing::Graph>,
    transit_lines: Res<crate::routing::TransitLines>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    mut dirty_agents: ResMut<DirtyAgents>,
    mut dirty_vehicles: ResMut<DirtyVehicles>,
) {
    // ----- PHASE A: BOARDING -----

    // A.1 — collect (stop node, front agent, line/edge/progress) for each stop
    // that has at least one waiting agent. Defer the chunk-activity filter to
    // A.2 so we don't pre-pass over all 100k agents.
    let mut boarding_candidates: Vec<(
        crate::routing::NodeId,
        String,
        AgentId,
        crate::routing::LineId,
        usize,
        f32,
    )> = Vec::new();
    for (node_id, queue) in waiting.iter() {
        let Some(agent_id) = queue.front() else {
            continue;
        };
        let Some(stop_id) = graph.legacy_node_ids(*node_id).first().cloned() else {
            continue;
        };
        for (line_id, edge_index, progress) in
            line_edge_stops_for_node(&graph, &transit_lines, *node_id)
        {
            boarding_candidates.push((
                *node_id,
                stop_id.clone(),
                agent_id.clone(),
                line_id,
                edge_index,
                progress,
            ));
        }
    }

    // A.2 — find a matching vehicle for each candidate. Both the candidate
    // agent AND the matched vehicle must live in an Active/Hot chunk.
    // Two-phase: first lookup candidate agent positions (p0 borrow), then
    // match against vehicles (p1 borrow) — ParamSet only permits one inner
    // query borrow at a time.
    let mut candidates_with_pos: Vec<(
        crate::routing::NodeId,
        String,
        AgentId,
        crate::routing::LineId,
        usize,
        f32,
    )> = Vec::new();
    {
        let agents = sets.p0();
        for (node_id, stop_id, agent_id, line_id, edge_index, stop_progress) in boarding_candidates
        {
            let Some(agent_entity) = agent_index.0.get(&agent_id).copied() else {
                continue;
            };
            let Ok((_, pos, _, _, _)) = agents.get(agent_entity) else {
                continue;
            };
            if !chunk_is_simulated(pos, &simulated) {
                continue;
            }
            candidates_with_pos.push((
                node_id,
                stop_id,
                agent_id,
                line_id,
                edge_index,
                stop_progress,
            ));
        }
    }

    let mut boardings: Vec<(
        crate::routing::NodeId,
        String,
        AgentId,
        Entity,
        VehicleId,
        u16,
    )> = Vec::new();
    {
        let vehicles = sets.p1();
        for (node_id, stop_id, agent_id, line_id, edge_index, stop_progress) in candidates_with_pos
        {
            for (v_entity, v_pos_world, v_stable, v_occ, v_cap, v_pos) in vehicles.iter() {
                if !chunk_is_simulated(v_pos_world, &simulated) {
                    continue;
                }
                if v_pos.line_id == line_id
                    && v_pos.edge_index == edge_index
                    && (v_pos.progress - stop_progress).abs() < 1e-6
                    && v_occ.0.len() < v_cap.0 as usize
                {
                    let seat_index = v_occ.0.len() as u16;
                    boardings.push((
                        node_id,
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
        for (_node_id, _stop_id, agent_id, v_entity, _v_id, _seat) in &boardings {
            if let Ok((_, _, _, mut v_occ, _, _)) = vehicles.get_mut(*v_entity) {
                v_occ.0.push(agent_id.clone());
                dirty_vehicles.0.insert(*v_entity);
            }
        }
    }

    // A.4 — pop boarded agents from stop queues.
    for (node_id, _stop_id, agent_id, _, _, _) in &boardings {
        if waiting.queue(*node_id).and_then(|queue| queue.front()) == Some(agent_id) {
            waiting.dequeue(*node_id);
        }
    }

    // A.5 — agent-side mutations: state becomes InVehicle. O(1) lookup via index.
    {
        let mut agents = sets.p0();
        for (_node_id, _stop_id, agent_id, _v_entity, v_id, seat_index) in &boardings {
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

    // B.1 — collect (vehicle_entity, vehicle_id, end-of-edge stop node, occupants)
    // for every vehicle parked at an end-of-link stop in an Active/Hot chunk.
    let mut end_of_link_stops: std::collections::HashMap<
        (crate::routing::LineId, usize),
        crate::routing::NodeId,
    > = std::collections::HashMap::new();
    for line in transit_lines.iter() {
        for (edge_index, edge_id) in line.edges.iter().enumerate() {
            let to = graph.edge(*edge_id).to;
            if graph.node(to).kind == crate::routing::NodeKind::TransitStop {
                end_of_link_stops.insert((line.id, edge_index), to);
            }
        }
    }

    let mut alighting_candidates: Vec<(Entity, VehicleId, crate::routing::NodeId, Vec<AgentId>)> =
        Vec::new();
    {
        let vehicles = sets.p1();
        for (v_entity, v_pos_world, v_stable, v_occ, _cap, v_pos) in vehicles.iter() {
            if !chunk_is_simulated(v_pos_world, &simulated) {
                continue;
            }
            if (v_pos.progress - 1.0).abs() >= 1e-6 {
                continue;
            }
            if let Some(node_id) = end_of_link_stops.get(&(v_pos.line_id, v_pos.edge_index)) {
                alighting_candidates.push((
                    v_entity,
                    v_stable.0.clone(),
                    *node_id,
                    v_occ.0.clone(),
                ));
            }
        }
    }

    // B.2 — for each occupant, check plan stage + state. O(1) lookups via index.
    let mut to_alight: Vec<(Entity, VehicleId, String, AgentId)> = Vec::new();
    {
        let agents = sets.p0();
        for (v_entity, v_id, node_id, occupants) in &alighting_candidates {
            for agent_id in occupants {
                let Some(a_entity) = agent_index.0.get(agent_id).copied() else {
                    continue;
                };
                let Ok((_, a_pos, _, a_state, a_plan)) = agents.get(a_entity) else {
                    continue;
                };
                if !chunk_is_simulated(a_pos, &simulated) {
                    continue;
                }
                let stage = a_plan.stages.get(a_plan.cursor);
                let target_stop_id = match stage {
                    Some(PlanStage::RideToStop {
                        stop_id: target, ..
                    }) if graph.node_by_legacy(target) == Some(*node_id) => Some(target.clone()),
                    _ => None,
                };
                let in_this_vehicle = matches!(
                    &a_state.0,
                    AgentMobilityState::InVehicle { vehicle_id, .. } if vehicle_id == v_id
                );
                if let (Some(stop_id), true) = (target_stop_id, in_this_vehicle) {
                    to_alight.push((*v_entity, v_id.clone(), stop_id, agent_id.clone()));
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
        (&RoutePosition, &mut Position, Option<&CurrentLinkPolyline>),
        (With<VehicleMarker>, Without<AgentMarker>),
    >,
    simulated: Res<SimulatedChunks>,
    transit_lines: Res<crate::routing::TransitLines>,
    graph: Res<crate::routing::Graph>,
) {
    // Equality-guarded writes: bevy's `Mut<T>` marks the component changed
    // on every deref_mut, even if the new value is the same as the old one.
    // Without this guard, `Changed<Position>` fires for every entity every
    // tick and the incremental `track_chunk_populations_system` degenerates
    // into a full rebuild — destroying Task 6's win.
    //
    // NaN guard: the != comparison silently misbehaves if x or y is NaN
    // (NaN ≠ NaN is true → unconditional write every tick → Changed fires
    // → incremental rebucketing degenerates). Debug builds assert finite;
    // release builds skip non-finite values entirely.
    for (rp, mut pos, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(&pos, &simulated) {
            continue;
        }
        let new_xy = if let Some(c) = cached {
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                &c.points,
                rp.progress,
            ))
        } else {
            crate::mobility::vehicle_world_coord(rp, &transit_lines, &graph)
        };
        if let Some((x, y)) = new_xy
            && (pos.x != x || pos.y != y)
        {
            debug_assert!(
                x.is_finite() && y.is_finite(),
                "non-finite vehicle Position"
            );
            pos.x = x;
            pos.y = y;
        }
    }
    for (state, mut pos, cached) in agents.iter_mut() {
        if !chunk_is_simulated(&pos, &simulated) {
            continue;
        }
        let new_xy =
            if let (AgentMobilityState::Walking { progress, .. }, Some(c)) = (&state.0, cached) {
                Some(crate::mobility_geometry::world_coord_at_progress_slice(
                    &c.points, *progress,
                ))
            } else {
                crate::mobility::agent_world_coord(&state.0, &graph, &transit_lines)
            };
        if let Some((x, y)) = new_xy
            && (pos.x != x || pos.y != y)
        {
            debug_assert!(x.is_finite() && y.is_finite(), "non-finite agent Position");
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
    simulated: Res<SimulatedChunks>,
    transit_lines: Res<crate::routing::TransitLines>,
    graph: Res<crate::routing::Graph>,
) {
    for (pos, rp, mut dir, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(pos, &simulated) {
            continue;
        }
        if let Some(c) = cached {
            dir.0 = dir_at_progress(&c.points, rp.progress);
            continue;
        }
        let Some(line) = ((rp.line_id.0 as usize) < transit_lines.count())
            .then(|| transit_lines.line(rp.line_id))
        else {
            continue;
        };
        let Some(edge_id) = line.edges.get(rp.edge_index) else {
            continue;
        };
        dir.0 = dir_at_progress(&graph.edge(*edge_id).polyline, rp.progress);
    }
    for (pos, state, mut dir, cached) in agents.iter_mut() {
        if !chunk_is_simulated(pos, &simulated) {
            continue;
        }
        if let AgentMobilityState::Walking { link_id, progress } = &state.0 {
            if let Some(c) = cached {
                dir.0 = dir_at_progress(&c.points, *progress);
            } else if let Some(edge_id) =
                crate::mobility::api::edge_by_canonical_key(&graph, link_id)
            {
                dir.0 = dir_at_progress(&graph.edge(edge_id).polyline, *progress);
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
    moved_agents: Query<(Entity, &Position), (With<AgentMarker>, Changed<Position>)>,
    moved_vehicles: Query<(Entity, &Position), (With<VehicleMarker>, Changed<Position>)>,
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
            agents_by_chunk.0.entry(chunk).or_default().insert(entity);
            previous.0.insert(entity, chunk);
        }
        for (entity, pos) in all_vehicles.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            vehicles_by_chunk.0.entry(chunk).or_default().insert(entity);
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
                    bucket.remove(&entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            agents_by_chunk
                .0
                .entry(new_chunk)
                .or_default()
                .insert(entity);
            previous.0.insert(entity, new_chunk);
        }
        for (entity, pos) in moved_vehicles.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.remove(&entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            vehicles_by_chunk
                .0
                .entry(new_chunk)
                .or_default()
                .insert(entity);
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
                    bucket.remove(&entity);
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.remove(&entity);
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

// Phase 8a follow-ups removed the resource-only compat shim
// (`classify_activity_system` + `ChunkActivities` / `ChunkSubscribers`
// resources). Chunk LOD is now classified once, on the chunk entity, by
// `crate::world::systems::reclassify_chunk_lod_system` under
// `CoreSet::LodReclassify`. Mobility consumes the resulting
// `ChunkLodChanged` event stream via
// `consume_chunk_lod_transitions_system` (below), which fills the
// `ChunkLodTransitions` scratchpad that promote/demote drain.

/// Rebuild the `SimulatedChunks` + `WarmChunkCoords` derived views from the
/// chunk-entity LOD markers. Runs at the head of `MobilitySet::LOD` each
/// tick so all downstream systems see a consistent view of which chunks
/// are simulated this tick.
pub fn refresh_simulated_chunks_system(
    hot: Query<&ChunkCoordComp, With<HotChunk>>,
    active: Query<&ChunkCoordComp, With<ActiveChunk>>,
    warm: Query<&ChunkCoordComp, With<WarmChunk>>,
    mut simulated: ResMut<SimulatedChunks>,
    mut warm_view: ResMut<WarmChunkCoords>,
) {
    simulated.0.clear();
    for c in hot.iter().chain(active.iter()) {
        simulated.0.insert(c.0);
    }
    warm_view.0.clear();
    for c in warm.iter() {
        warm_view.0.insert(c.0);
    }
}

/// Drain `ChunkLodChanged` messages emitted by the foundation's
/// `reclassify_chunk_lod_system` and stash them in the
/// `ChunkLodTransitions` scratchpad for promote/demote to consume.
///
/// The `Local<MessageCursor<…>>` survives across ticks, so even if a tick
/// is delayed the consumer never misses a transition.
pub fn consume_chunk_lod_transitions_system(
    mut cursor: Local<MessageCursor<ChunkLodChanged>>,
    messages: Res<Messages<ChunkLodChanged>>,
    mut out: ResMut<ChunkLodTransitions>,
) {
    out.0.clear();
    for event in cursor.read(&messages) {
        out.0.push((event.coord, event.from, event.to));
    }
}

pub fn promote_warm_to_active_system(
    transitions: Res<ChunkLodTransitions>,
    mut flow_cells: ResMut<FlowCells>,
    graph: Res<crate::routing::Graph>,
    transit_lines: Res<crate::routing::TransitLines>,
    tick: Res<Tick>,
    mut commands: Commands,
) {
    for (chunk, prev, next) in &transitions.0 {
        if *prev != ChunkLod::Warm {
            continue;
        }
        if !matches!(next, ChunkLod::Active | ChunkLod::Hot) {
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
        let mut spawn_link: Option<String> = None;
        for edge in graph.edges() {
            if edge.kind == crate::routing::EdgeKind::Footway
                && edge
                    .polyline
                    .iter()
                    .any(|(x, y)| crate::mobility::chunk_of(*x, *y, 32) == *chunk)
                && let Some(legacy_id) = &edge.legacy_id
            {
                spawn_link = Some(legacy_id.clone());
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
            let (px, py) =
                crate::mobility::agent_world_coord(&spawned_state, &graph, &transit_lines)
                    .expect("LOD promoted walking agent must resolve through routing graph");
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
    transitions: Res<ChunkLodTransitions>,
    agents: Query<&AgentMobilityStateComponent, With<AgentMarker>>,
    agents_by_chunk: Res<AgentsByChunk>,
    vehicles_by_chunk: Res<VehiclesByChunk>,
    graph: Res<crate::routing::Graph>,
    transit_lines: Res<crate::routing::TransitLines>,
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
        if *next != ChunkLod::Warm {
            continue;
        }

        let Some(agent_entities) = agents_by_chunk.0.get(chunk) else {
            // No agents in this chunk — nothing to despawn. Vehicles might
            // still be present, fall through to vehicle handling.
            if !vehicles_by_chunk.0.contains_key(chunk) {
                continue;
            }
            let empty: HashSet<Entity> = HashSet::new();
            let vehicle_entities = vehicles_by_chunk.0.get(chunk).unwrap_or(&empty);
            despawn_vehicles_into_flow_cell(
                *chunk,
                vehicle_entities,
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
            let dest = agent_destination_chunk(state, &graph, &transit_lines).unwrap_or(*chunk);
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
    vehicle_entities: &HashSet<Entity>,
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
    graph: &crate::routing::Graph,
    transit_lines: &crate::routing::TransitLines,
) -> Option<crate::ids::ChunkCoord> {
    if let AgentMobilityState::Walking { link_id, .. } = &state.0 {
        return crate::mobility::api::edge_by_canonical_key(graph, link_id)
            .and_then(|edge_id| graph.edge(edge_id).polyline.last().copied())
            .map(|(x, y)| crate::mobility::chunk_of(x, y, 32));
    }
    crate::mobility::agent_world_coord(&state.0, graph, transit_lines)
        .map(|(x, y)| crate::mobility::chunk_of(x, y, 32))
}

pub fn warm_chunk_flow_system(
    tick: Res<Tick>,
    warm: Res<WarmChunkCoords>,
    mut flow_cells: ResMut<FlowCells>,
) {
    if !tick.0.is_multiple_of(10) {
        return;
    }

    let warm_chunks: Vec<crate::ids::ChunkCoord> = warm.0.iter().copied().collect();

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

    /// Returns a `SimulatedChunks` resource pre-populated so that every
    /// chunk in a generous range around the origin counts as simulated.
    /// Tests that exercise the LOD-filtered Advance/Output systems use
    /// this so the filter doesn't skip their fixtures.
    fn all_active() -> SimulatedChunks {
        let mut a = SimulatedChunks::default();
        for x in -10..=20 {
            for y in -10..=20 {
                a.0.insert(crate::ids::ChunkCoord { x, y });
            }
        }
        a
    }

    fn insert_test_routing(world: &mut World) -> crate::routing::LineId {
        use crate::routing::{Edge, EdgeId, EdgeKind, Graph, LineId, Node, NodeId, NodeKind};
        use crate::routing::{TransitLine, TransitLines};

        let nodes = vec![
            Node {
                id: NodeId(0),
                position: (0.0, 0.0),
                kind: NodeKind::TransitStop,
                legacy_id: Some("s:1".into()),
            },
            Node {
                id: NodeId(1),
                position: (10.0, 0.0),
                kind: NodeKind::TransitStop,
                legacy_id: Some("s:end".into()),
            },
            Node {
                id: NodeId(2),
                position: (0.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(3),
                position: (40.0, 5.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ];
        let edges = vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (10.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::TramTrack,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:vehicle".into()),
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (10.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:1".into()),
            },
            Edge {
                id: EdgeId(2),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (10.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("link:test".into()),
            },
            Edge {
                id: EdgeId(3),
                from: NodeId(0),
                to: NodeId(2),
                polyline: vec![(0.0, 0.0), (0.0, 10.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:b".into()),
            },
            Edge {
                id: EdgeId(4),
                from: NodeId(1),
                to: NodeId(2),
                polyline: vec![(10.0, 0.0), (0.0, 10.0)],
                length: 20.0_f32.sqrt() * 10.0_f32.sqrt(),
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:2".into()),
            },
            Edge {
                id: EdgeId(5),
                from: NodeId(0),
                to: NodeId(3),
                polyline: vec![(5.0, 5.0), (40.0, 5.0)],
                length: 35.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:end".into()),
            },
            Edge {
                id: EdgeId(6),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(10.0, 10.0), (20.0, 10.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:0".into()),
            },
            Edge {
                id: EdgeId(7),
                from: NodeId(0),
                to: NodeId(2),
                polyline: vec![(0.0, 0.0), (0.0, 10.0)],
                length: 10.0,
                kind: EdgeKind::TramTrack,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:b".into()),
            },
        ];
        let mut graph = Graph::new(nodes, edges);
        graph.add_legacy_node_alias("stop:old-town".into(), NodeId(0));
        graph.add_legacy_node_alias("stop:station".into(), NodeId(1));
        let line_id = LineId(0);
        let mut lines = TransitLines::new(vec![TransitLine {
            id: line_id,
            name: "r:1".into(),
            edges: vec![EdgeId(0), EdgeId(7)],
            stops: vec![NodeId(0), NodeId(1)],
            legacy_route_id: Some("r:1".into()),
        }]);
        lines.add_legacy_route_alias("route:old-town-loop".into(), line_id);
        world.insert_resource(graph);
        world.insert_resource(lines);
        if !world.contains_resource::<crate::routing::WaitingAgents>() {
            world.insert_resource(crate::routing::WaitingAgents::default());
        }
        line_id
    }

    #[test]
    fn tick_increment_system_advances_tick_by_one_per_schedule_run() {
        // Use the full plugin install path — the LOD set now depends on
        // foundation-owned `Messages<ChunkLodChanged>` and a configured
        // `CoreSet::LodReclassify`, so building the world by hand is brittle.
        let (mut world, mut schedule) = crate::mobility::api::empty_world_and_schedule();
        // Replace the freshly-installed (empty) SimulatedChunks with one
        // primed for the wide tile range any future seed could land in —
        // we're only asserting on Tick(), but the install order keeps the
        // gating systems happy.
        *world.resource_mut::<SimulatedChunks>() = all_active();
        schedule.run(&mut world);
        assert_eq!(world.resource::<Tick>().0, 1);
        schedule.run(&mut world);
        assert_eq!(world.resource::<Tick>().0, 2);
    }

    #[test]
    fn stop_arrival_transitions_walking_agent_to_waiting_at_stop() {
        use crate::ids::AgentId;
        use crate::mobility::records::{AgentMobilityState, PlanStage};

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:1".into(),
                    progress: 1.0,
                }),
                WalkPlan {
                    stages: vec![PlanStage::WalkToStop {
                        link_id: "l:1".into(),
                        stop_id: "s:1".into(),
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
                assert_eq!(stop_id.as_str(), "s:1");
            }
            other => panic!("expected WaitingAtStop, got {other:?}"),
        }
        let plan = world.get::<WalkPlan>(entity).unwrap();
        assert_eq!(plan.cursor, 1);
        let node_id = world
            .resource::<crate::routing::Graph>()
            .node_by_legacy("s:1")
            .unwrap();
        let waiting = world.resource::<crate::routing::WaitingAgents>();
        assert_eq!(
            waiting.queue(node_id).and_then(|queue| queue.front()),
            Some(&AgentId("a:1".into()))
        );
        assert!(world.resource::<DirtyAgents>().0.contains(&entity));
    }

    #[test]
    fn boarding_system_moves_waiting_agent_into_matching_vehicle() {
        use crate::ids::{AgentId, VehicleId};
        use crate::mobility::records::{AgentMobilityState, VehicleKind};

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(Tick(0));
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);
        let line_id = crate::routing::LineId(0);
        world.insert_resource(crate::mobility::resources::AgentIdIndex::default());
        let node_id = world
            .resource::<crate::routing::Graph>()
            .node_by_legacy("s:1")
            .unwrap();
        world
            .resource_mut::<crate::routing::WaitingAgents>()
            .enqueue(node_id, AgentId("a:1".into()));

        let agent_entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::WaitingAtStop {
                    stop_id: "s:1".into(),
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
                    line_id,
                    edge_index: 0,
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
        let node_id = world
            .resource::<crate::routing::Graph>()
            .node_by_legacy("s:1")
            .unwrap();
        let waiting = world.resource::<crate::routing::WaitingAgents>();
        assert!(
            waiting
                .queue(node_id)
                .map(|queue| queue.is_empty())
                .unwrap_or(true)
        );
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
        use crate::ids::{AgentId, VehicleId};
        use crate::mobility::records::{AgentMobilityState, PlanStage, VehicleKind};

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);
        let line_id = crate::routing::LineId(0);
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
                            route_id: "r:1".into(),
                            stop_id: "s:end".into(),
                        },
                        PlanStage::WalkToActivity {
                            link_id: "l:2".into(),
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
                    line_id,
                    edge_index: 0,
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
                assert_eq!(link_id.as_str(), "l:2");
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
        use crate::ids::AgentId;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(Tick(0));
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "link:test".into(),
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
        use crate::ids::AgentId;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:near".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "link:test".into(),
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
        use crate::ids::VehicleId;
        use crate::mobility::records::VehicleKind;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(DirtyVehicles::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);
        let line_id = crate::routing::LineId(0);

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    line_id,
                    edge_index: 0,
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
        use crate::ids::VehicleId;
        use crate::mobility::records::VehicleKind;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);
        world.insert_resource(DirtyVehicles::default());
        let line_id = crate::routing::LineId(0);

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    line_id,
                    edge_index: 0,
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
    fn vehicle_advance_requires_graph_transit_lines() {
        use crate::ids::VehicleId;
        use crate::mobility::records::VehicleKind;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);
        world.insert_resource(DirtyVehicles::default());
        world.insert_resource(crate::routing::TransitLines::default());

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:legacy".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    line_id: crate::routing::LineId(0),
                    edge_index: 0,
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
        assert!(
            (pos.progress - 0.4).abs() < 1e-6,
            "vehicles must not advance without graph TransitLines"
        );
        assert!(!world.resource::<DirtyVehicles>().0.contains(&entity));
    }

    #[test]
    fn compute_world_coord_system_writes_position_for_walking_agent() {
        use crate::ids::AgentId;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:1".into(),
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
        use crate::ids::AgentId;
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:1".into(),
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
        insert_test_routing(&mut world);
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
                    link_id: "l".into(),
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
                line_id: crate::routing::LineId(0),
                edge_index: 0,
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
        use crate::ids::VehicleId;
        use crate::mobility::records::VehicleKind;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);
        let line_id = crate::routing::LineId(0);

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Tram),
                RoutePosition {
                    line_id,
                    edge_index: 0,
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
            (pos.x - 2.5).abs() < 1e-3,
            "0.25 of the graph edge 0..10 = 2.5, got {}",
            pos.x
        );
        assert!(pos.y.abs() < 1e-3);
    }

    #[test]
    fn subscribe_drives_chunk_active_via_entity_classifier() {
        // End-to-end: apply_subscription_diff -> reclassify_chunk_lod_system
        // -> ChunkLodChanged -> consume_chunk_lod_transitions_system. Activity
        // for the subscribed chunk reaches Active after one schedule tick.
        use crate::ids::ChunkCoord;
        use crate::mobility::lod::MobilityActivity;
        let (mut world, mut schedule) = crate::mobility::api::empty_world_and_schedule();
        let chunk = ChunkCoord { x: 4, y: 4 };
        crate::mobility::api::apply_subscription_diff(&mut world, &[chunk], std::iter::empty());
        schedule.run(&mut world);
        assert_eq!(
            crate::mobility::api::activity_for_chunk(&world, chunk),
            Some(MobilityActivity::Active),
            "single subscriber → Active on first tick",
        );
    }

    #[test]
    fn consume_chunk_lod_transitions_publishes_event_to_scratchpad() {
        // Manually write a `ChunkLodChanged` event and assert it lands in
        // `ChunkLodTransitions` after running the consumer system.
        use crate::ids::ChunkCoord;
        let (mut world, _schedule) = crate::mobility::api::empty_world_and_schedule();
        let chunk = ChunkCoord { x: 1, y: 2 };
        world
            .resource_mut::<Messages<ChunkLodChanged>>()
            .write(ChunkLodChanged {
                entity: Entity::PLACEHOLDER,
                coord: chunk,
                from: ChunkLod::Asleep,
                to: ChunkLod::Active,
            });
        let mut sched = Schedule::default();
        sched.add_systems(consume_chunk_lod_transitions_system);
        sched.run(&mut world);
        let scratch = world.resource::<ChunkLodTransitions>();
        assert_eq!(scratch.0.len(), 1);
        assert_eq!(scratch.0[0].0, chunk);
        assert_eq!(scratch.0[0].1, ChunkLod::Asleep);
        assert_eq!(scratch.0[0].2, ChunkLod::Active);
    }

    #[test]
    fn promote_warm_spawns_floor_population_agents() {
        use crate::ids::*;
        use crate::mobility::lod::FlowCell;

        let mut world = World::new();
        insert_test_routing(&mut world);
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

        let mut transitions = ChunkLodTransitions::default();
        transitions
            .0
            .push((chunk, ChunkLod::Warm, ChunkLod::Active));
        world.insert_resource(transitions);

        world.insert_resource(Tick(100));

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
        use crate::mobility::records::AgentMobilityState;

        let mut world = World::new();
        insert_test_routing(&mut world);
        let chunk = ChunkCoord { x: 0, y: 0 };

        world.insert_resource(FlowCells::default());
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(AgentsByChunk::default());
        world.insert_resource(VehiclesByChunk::default());
        world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
        world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());

        let mut transitions = ChunkLodTransitions::default();
        transitions
            .0
            .push((chunk, ChunkLod::Active, ChunkLod::Warm));
        world.insert_resource(transitions);

        for n in 0..3 {
            world.spawn((
                AgentMarker,
                StableAgentId(AgentId(format!("a:{n}"))),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:end".into(),
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
        use crate::mobility::lod::FlowCell;

        let mut world = World::new();
        world.insert_resource(Tick(10));
        let mut warm = WarmChunkCoords::default();
        warm.0.insert(ChunkCoord { x: 0, y: 0 });
        world.insert_resource(warm);

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
        use crate::mobility::lod::FlowCell;

        let mut world = World::new();
        world.insert_resource(Tick(5));
        let mut warm = WarmChunkCoords::default();
        warm.0.insert(ChunkCoord { x: 0, y: 0 });
        world.insert_resource(warm);

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
        use crate::mobility::lod::FlowCell;

        fn run_promote() -> Vec<String> {
            let mut world = World::new();
            insert_test_routing(&mut world);
            let chunk = ChunkCoord { x: 0, y: 0 };
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
            let mut transitions = ChunkLodTransitions::default();
            transitions
                .0
                .push((chunk, ChunkLod::Warm, ChunkLod::Active));
            world.insert_resource(transitions);
            world.insert_resource(Tick(42));

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
        insert_test_routing(&mut world);
        world.insert_resource(SimulatedChunks::default()); // empty = none simulated
        world.insert_resource(DirtyAgents::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:0".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:0".into(),
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
        insert_test_routing(&mut world);
        let mut simulated = SimulatedChunks::default();
        // Position (100, 100) → chunk (3, 3) for chunk_size = 32.
        simulated.0.insert(ChunkCoord { x: 3, y: 3 });
        world.insert_resource(simulated);
        world.insert_resource(DirtyAgents::default());

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:0".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:0".into(),
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
        use crate::ids::AgentId;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:1".into(),
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
        use crate::ids::AgentId;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:1".into(),
                    progress: 1.0,
                }),
                WalkPlan {
                    stages: vec![PlanStage::WalkToStop {
                        link_id: "l:1".into(),
                        stop_id: "s:1".into(),
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
        use crate::ids::AgentId;
        use std::sync::Arc;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(AgentId("a:1".into())),
                AgentMobilityStateComponent(AgentMobilityState::Walking {
                    link_id: "l:a".into(),
                    progress: 0.0,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
                WalkSpeed(0.05),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(String::new()),
                CurrentLinkPolyline {
                    link_id: "l:a".into(),
                    points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(update_link_polyline_cache_system);

        // Tick 1: cache already matches → no change.
        schedule.run(&mut world);
        assert_eq!(
            world
                .get::<CurrentLinkPolyline>(entity)
                .unwrap()
                .link_id
                .as_str(),
            "l:a"
        );

        // Mutate the agent to a different link.
        if let Some(mut s) = world.get_mut::<AgentMobilityStateComponent>(entity) {
            s.0 = AgentMobilityState::Walking {
                link_id: "l:b".into(),
                progress: 0.0,
            };
        }
        schedule.run(&mut world);
        assert_eq!(
            world
                .get::<CurrentLinkPolyline>(entity)
                .unwrap()
                .link_id
                .as_str(),
            "l:b"
        );
        let cached = world.get::<CurrentLinkPolyline>(entity).unwrap();
        assert_eq!(cached.points.as_ref(), &vec![(0.0, 0.0), (0.0, 10.0)]);
    }

    #[test]
    fn current_link_polyline_invalidates_on_vehicle_link_change() {
        use crate::ids::VehicleId;
        use crate::mobility::records::VehicleKind;
        use std::sync::Arc;

        let mut world = World::new();
        insert_test_routing(&mut world);
        world.insert_resource(all_active());
        insert_test_routing(&mut world);

        let line_id = crate::routing::LineId(0);

        let entity = world
            .spawn((
                VehicleMarker,
                StableVehicleId(VehicleId("v:1".into())),
                VehicleKindComponent(VehicleKind::Car),
                RoutePosition {
                    line_id,
                    edge_index: 0,
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
                    link_id: "l:a".into(),
                    points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
                },
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(update_link_polyline_cache_system);

        if let Some(mut rp) = world.get_mut::<RoutePosition>(entity) {
            rp.edge_index = 1;
        }
        schedule.run(&mut world);
        assert_eq!(
            world
                .get::<CurrentLinkPolyline>(entity)
                .unwrap()
                .link_id
                .as_str(),
            "l:b"
        );
    }

    #[test]
    fn incremental_chunk_populations_matches_full_rebuild() {
        use crate::ids::AgentId;
        use crate::mobility::resources::{PreviousChunkByEntity, PreviousFlowCellContrib};

        let mut world = World::new();
        insert_test_routing(&mut world);
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
                    link_id: "l".into(),
                    progress: 0.0,
                }),
                WalkPlan {
                    stages: vec![],
                    cursor: 0,
                },
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
        let moved_entity = q
            .iter_mut(&mut world)
            .next()
            .map(|(e, mut p)| {
                p.x = 999.0;
                p.y = 999.0;
                e
            })
            .unwrap();

        // Tick 2: incremental path.
        schedule.run(&mut world);
        let after2_incremental: std::collections::HashMap<crate::ids::ChunkCoord, HashSet<Entity>> =
            world
                .resource::<AgentsByChunk>()
                .0
                .iter()
                .map(|(c, e)| (*c, e.clone()))
                .collect();

        // Compare against a fresh full rebuild from query state.
        let mut reference: std::collections::HashMap<crate::ids::ChunkCoord, HashSet<Entity>> =
            std::collections::HashMap::new();
        let mut q2 = world.query::<(Entity, &Position, &AgentMarker)>();
        for (entity, pos, _) in q2.iter(&world) {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            reference.entry(chunk).or_default().insert(entity);
        }
        assert_eq!(after2_incremental, reference);
        // Ensure the moved entity actually moved buckets.
        assert!(
            after2_incremental
                .values()
                .any(|v| v.contains(&moved_entity))
        );
    }
}

#[cfg(test)]
mod route_execution_tests {
    use super::*;
    use crate::ids::AgentId;
    use crate::mobility::records::{AgentMobilityState, AgentRecord, PlanStage};
    use crate::routing::{
        Edge, EdgeId, EdgeKind, FlowFieldCache, Graph, HpaConfig, HpaIndex, Node, NodeId, NodeKind,
        NodeSpatialIndex,
    };

    fn route_graph(activity_legacy_id: Option<&str>) -> Graph {
        route_graph_with_edge_legacy(activity_legacy_id, true)
    }

    fn graph_native_route_graph(activity_legacy_id: Option<&str>) -> Graph {
        route_graph_with_edge_legacy(activity_legacy_id, false)
    }

    fn route_graph_with_edge_legacy(activity_legacy_id: Option<&str>, edge_legacy: bool) -> Graph {
        Graph::new(
            vec![
                Node {
                    id: NodeId(0),
                    position: (0.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(1),
                    position: (1.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(2),
                    position: (2.0, 0.0),
                    kind: NodeKind::ActivityLocation,
                    legacy_id: activity_legacy_id.map(str::to_string),
                },
            ],
            vec![
                Edge {
                    id: EdgeId(0),
                    from: NodeId(0),
                    to: NodeId(1),
                    polyline: vec![(0.0, 0.0), (1.0, 0.0)],
                    length: 1.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: edge_legacy.then(|| "walk:a".into()),
                },
                Edge {
                    id: EdgeId(1),
                    from: NodeId(1),
                    to: NodeId(2),
                    polyline: vec![(1.0, 0.0), (2.0, 0.0)],
                    length: 1.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: edge_legacy.then(|| "walk:b".into()),
                },
            ],
        )
    }

    fn intermediate_cluster_route_graph() -> Graph {
        Graph::new(
            vec![
                Node {
                    id: NodeId(0),
                    position: (0.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(1),
                    position: (12.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(2),
                    position: (25.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(3),
                    position: (35.0, 0.0),
                    kind: NodeKind::ActivityLocation,
                    legacy_id: Some("activity:far".into()),
                },
            ],
            vec![
                Edge {
                    id: EdgeId(0),
                    from: NodeId(0),
                    to: NodeId(1),
                    polyline: vec![(0.0, 0.0), (12.0, 0.0)],
                    length: 12.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: Some("walk:0".into()),
                },
                Edge {
                    id: EdgeId(1),
                    from: NodeId(1),
                    to: NodeId(2),
                    polyline: vec![(12.0, 0.0), (25.0, 0.0)],
                    length: 13.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: Some("walk:1".into()),
                },
                Edge {
                    id: EdgeId(2),
                    from: NodeId(2),
                    to: NodeId(3),
                    polyline: vec![(25.0, 0.0), (35.0, 0.0)],
                    length: 10.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: Some("walk:2".into()),
                },
            ],
        )
    }

    fn world_schedule_and_agent() -> (World, Schedule, Entity) {
        world_schedule_and_agent_with_activity_legacy(Some("activity:work"))
    }

    fn world_schedule_and_agent_without_activity_legacy() -> (World, Schedule, Entity) {
        world_schedule_and_agent_with_activity_legacy(None)
    }

    fn world_schedule_and_agent_with_activity_legacy(
        activity_legacy_id: Option<&str>,
    ) -> (World, Schedule, Entity) {
        world_schedule_and_agent_with_graph(
            route_graph(activity_legacy_id),
            "walk:a",
            "activity:work",
            1.0,
        )
    }

    fn world_schedule_and_agent_with_graph(
        graph: Graph,
        initial_link_id: &str,
        activity_id: &str,
        walk_speed: f32,
    ) -> (World, Schedule, Entity) {
        let (mut world, schedule) = crate::mobility::api::empty_world_and_schedule();
        let hpa = HpaIndex::build(&graph, HpaConfig::default()).expect("HPA should build");
        let spatial = NodeSpatialIndex::from_nodes(graph.nodes());
        world.insert_resource(graph);
        world.insert_resource(hpa);
        world.insert_resource(spatial);
        world.insert_resource(FlowFieldCache::default());
        let active_coord = crate::ids::ChunkCoord { x: 0, y: 0 };
        let chunk_entity = world
            .spawn((
                ChunkCoordComp(active_coord),
                ActiveChunk,
                crate::world::components::ChunkSubscriberCount(1),
                crate::world::components::LodCooldown(0),
            ))
            .id();
        world
            .resource_mut::<crate::world::resources::ChunksByCoord>()
            .0
            .insert(active_coord, chunk_entity);

        let entity = crate::mobility::api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent:route".into()),
                AgentMobilityState::Walking {
                    link_id: initial_link_id.into(),
                    progress: 0.0,
                },
                vec![PlanStage::WalkToActivity {
                    link_id: initial_link_id.into(),
                    activity_id: activity_id.into(),
                }],
                walk_speed,
            ),
        );

        (world, schedule, entity)
    }

    fn world_schedule_and_agent_requiring_intermediate_corridor() -> (World, Schedule, Entity) {
        let graph = intermediate_cluster_route_graph();
        let (mut world, schedule) = crate::mobility::api::empty_world_and_schedule();
        let hpa = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 10,
                corridor_margin_clusters: 0,
            },
        )
        .expect("HPA should build");
        let spatial = NodeSpatialIndex::from_nodes(graph.nodes());
        world.insert_resource(graph);
        world.insert_resource(hpa);
        world.insert_resource(spatial);
        world.insert_resource(FlowFieldCache::default());
        let active_coord = crate::ids::ChunkCoord { x: 0, y: 0 };
        let chunk_entity = world
            .spawn((
                ChunkCoordComp(active_coord),
                ActiveChunk,
                crate::world::components::ChunkSubscriberCount(1),
                crate::world::components::LodCooldown(0),
            ))
            .id();
        world
            .resource_mut::<crate::world::resources::ChunksByCoord>()
            .0
            .insert(active_coord, chunk_entity);

        let entity = crate::mobility::api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent:route".into()),
                AgentMobilityState::Walking {
                    link_id: "walk:0".into(),
                    progress: 0.0,
                },
                vec![PlanStage::WalkToActivity {
                    link_id: "walk:0".into(),
                    activity_id: "activity:far".into(),
                }],
                0.0,
            ),
        );

        (world, schedule, entity)
    }

    #[test]
    fn route_assignment_inserts_active_route() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();

        schedule.run(&mut world);

        let route = world
            .get::<ActiveRoute>(entity)
            .expect("route assignment should insert ActiveRoute");
        assert_eq!(route.destination, NodeId(2));
        assert_eq!(route.steps.len(), 2);
        assert_eq!(route.steps[0].canonical_edge_key, "walk:a");
        assert_eq!(route.steps[1].canonical_edge_key, "walk:b");
        assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
    }

    #[test]
    fn route_assignment_uses_full_hpa_corridor() {
        let (mut world, mut schedule, entity) =
            world_schedule_and_agent_requiring_intermediate_corridor();

        schedule.run(&mut world);

        let route = world
            .get::<ActiveRoute>(entity)
            .expect("route assignment should include intermediate corridor clusters");
        assert_eq!(route.destination, NodeId(3));
        assert_eq!(route.steps.len(), 3);
        assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 0);
    }

    #[test]
    fn route_assignment_and_advance_accept_graph_native_edge_keys() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent_with_graph(
            graph_native_route_graph(Some("activity:work")),
            "edge:0",
            "activity:work",
            1.0,
        );

        schedule.run(&mut world);
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        schedule.run(&mut world);

        let route = world
            .get::<ActiveRoute>(entity)
            .expect("graph-native route should remain active on second edge");
        assert_eq!(route.cursor, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "edge:1" && *progress == 0.0
        ));
    }

    #[test]
    fn route_assignment_syncs_completed_initial_edge_to_first_step() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:a".into(),
            progress: 1.0,
        };

        schedule.run(&mut world);

        let route = world
            .get::<ActiveRoute>(entity)
            .expect("route assignment should insert ActiveRoute from completed edge endpoint");
        assert_eq!(route.cursor, 0);
        assert_eq!(route.steps.len(), 1);
        assert_eq!(route.steps[0].canonical_edge_key, "walk:b");
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "walk:b" && *progress == 0.0
        ));
    }

    #[test]
    fn route_assignment_completes_when_origin_is_destination() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:b".into(),
            progress: 1.0,
        };

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::AtActivity { activity_id } if activity_id == "activity:work"
        ));
    }

    #[test]
    fn route_assignment_resolves_activity_destination_through_spatial_fallback() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent_without_activity_legacy();

        schedule.run(&mut world);

        let route = world
            .get::<ActiveRoute>(entity)
            .expect("route assignment should use activity geometry and spatial fallback");
        assert_eq!(route.destination, NodeId(2));
        assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 0);
    }

    #[test]
    fn route_assignment_counts_unresolved_destination_as_failed() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent_without_activity_legacy();
        world.remove_resource::<NodeSpatialIndex>();

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        let stats = world.resource::<RouteAssignmentStats>();
        assert_eq!(stats.assigned, 0);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn route_advance_crosses_edges_before_finishing_plan() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();

        schedule.run(&mut world);
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        schedule.run(&mut world);

        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        match &state.0 {
            AgentMobilityState::Walking { link_id, progress } => {
                assert_eq!(link_id, "walk:b");
                assert_eq!(*progress, 0.0);
            }
            other => panic!("expected walking on second edge, got {other:?}"),
        }
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
        assert_eq!(world.get::<ActiveRoute>(entity).unwrap().cursor, 1);
    }

    #[test]
    fn route_advance_completes_final_activity_route() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:b".into(),
            progress: 1.0,
        };
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(2),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![RouteStep {
                edge_id: EdgeId(1),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:b".into(),
                length: 1.0,
            }],
            cursor: 0,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::AtActivity { activity_id } if activity_id == "activity:work"
        ));
    }

    #[test]
    fn route_advance_completes_final_stop_route() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world.get_mut::<WalkPlan>(entity).unwrap().stages = vec![PlanStage::WalkToStop {
            link_id: "walk:b".into(),
            stop_id: "activity:work".into(),
        }];
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:b".into(),
            progress: 1.0,
        };
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(2),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![RouteStep {
                edge_id: EdgeId(1),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:b".into(),
                length: 1.0,
            }],
            cursor: 0,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::WaitingAtStop { stop_id } if stop_id == "activity:work"
        ));
        assert!(
            world
                .resource::<crate::routing::WaitingAgents>()
                .queue(NodeId(2))
                .is_some_and(|queue| queue.contains(&AgentId("agent:route".into())))
        );
    }

    #[test]
    fn route_advance_invalidates_unexpected_current_stage() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world.get_mut::<WalkPlan>(entity).unwrap().stages = vec![PlanStage::Activity {
            activity_id: "activity:work".into(),
        }];
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:a".into(),
            progress: 1.0,
        };
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(2),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![RouteStep {
                edge_id: EdgeId(0),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:a".into(),
                length: 1.0,
            }],
            cursor: 0,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "walk:a" && *progress >= 1.0
        ));
    }

    #[test]
    fn route_advance_invalidates_stale_current_link() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        schedule.run(&mut world);
        world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:b".into(),
            progress: 1.0,
        };

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "walk:b" && *progress >= 1.0
        ));
    }

    #[test]
    fn route_advance_invalidates_disconnected_next_step() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:a".into(),
            progress: 1.0,
        };
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(2),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![
                RouteStep {
                    edge_id: EdgeId(0),
                    mode: crate::routing::ModeState::Walking,
                    canonical_edge_key: "walk:a".into(),
                    length: 1.0,
                },
                RouteStep {
                    edge_id: EdgeId(0),
                    mode: crate::routing::ModeState::Walking,
                    canonical_edge_key: "walk:a".into(),
                    length: 1.0,
                },
            ],
            cursor: 0,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "walk:a" && *progress >= 1.0
        ));
    }

    #[test]
    fn route_advance_invalidates_cursor_past_steps() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(2),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![RouteStep {
                edge_id: EdgeId(0),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:a".into(),
                length: 1.0,
            }],
            cursor: 1,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    }

    #[test]
    fn route_advance_invalidates_when_final_edge_misses_destination() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:a".into(),
            progress: 1.0,
        };
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(2),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![RouteStep {
                edge_id: EdgeId(0),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:a".into(),
                length: 1.0,
            }],
            cursor: 0,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "walk:a" && *progress >= 1.0
        ));
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
    }

    #[test]
    fn route_advance_invalidates_when_stage_destination_mismatches_route() {
        let (mut world, mut schedule, entity) = world_schedule_and_agent();
        world
            .get_mut::<AgentMobilityStateComponent>(entity)
            .unwrap()
            .0 = AgentMobilityState::Walking {
            link_id: "walk:a".into(),
            progress: 1.0,
        };
        world.entity_mut(entity).insert(ActiveRoute {
            destination: NodeId(1),
            profile: crate::routing::RoutingProfileKey::Walk,
            steps: vec![RouteStep {
                edge_id: EdgeId(0),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:a".into(),
                length: 1.0,
            }],
            cursor: 0,
        });

        schedule.run(&mut world);

        assert!(world.get::<ActiveRoute>(entity).is_none());
        assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
        let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
        assert!(matches!(
            &state.0,
            AgentMobilityState::Walking { link_id, progress }
                if link_id == "walk:a" && *progress >= 1.0
        ));
        assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
    }
}
