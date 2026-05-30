use super::common::chunk_is_simulated;
use super::*;
use std::collections::HashSet;

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

fn is_at_route_endpoint(
    graph: &crate::routing::Graph,
    canonical_edge_key: &str,
    progress: f32,
    node_id: crate::routing::NodeId,
) -> Option<bool> {
    let edge_id = crate::mobility::api::edge_by_canonical_key(graph, canonical_edge_key)?;
    let edge = graph.edge(edge_id);
    Some((progress <= 0.0 && edge.from == node_id) || (progress >= 1.0 && edge.to == node_id))
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

#[allow(clippy::too_many_arguments)]
fn complete_walk_stage_at_destination(
    entity: Entity,
    stable: &StableAgentId,
    state: &mut AgentMobilityStateComponent,
    plan: &mut WalkPlan,
    stage: PlanStage,
    arrival_link_id: &str,
    destination: crate::routing::NodeId,
    waiting: &mut crate::routing::WaitingAgents,
    dirty: &mut DirtyAgents,
    commands: &mut Commands,
) -> bool {
    match stage {
        PlanStage::WalkToStop { stop_id, .. } => {
            crate::mobility::systems::advance_cursor(plan);
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
            crate::mobility::systems::advance_cursor(plan);
            // Cyclic plans never settle at an activity: the agent must depart
            // again toward the next (wrapped) stage. Re-anchor it as Walking at
            // the arrival edge (progress saturated) so route_assignment routes
            // it onward next tick. Non-cyclic plans terminate at the activity.
            state.0 = if plan.cyclic {
                AgentMobilityState::Walking {
                    link_id: arrival_link_id.to_string(),
                    progress: 1.0,
                }
            } else {
                AgentMobilityState::AtActivity { activity_id }
            };
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

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
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
    tick: Res<Tick>,
    graph: Res<crate::routing::Graph>,
    hpa: Option<Res<crate::routing::HpaIndex>>,
    spatial: Option<Res<crate::routing::NodeSpatialIndex>>,
    mut cache: Option<ResMut<crate::routing::FlowFieldCache>>,
    mut stats: ResMut<RouteAssignmentStats>,
    mut waiting: ResMut<crate::routing::WaitingAgents>,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
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
        if matches!(stage, PlanStage::Activity { .. }) {
            if progress >= 1.0
                && let Some(next_link) =
                    next_wander_footway_link(&graph, &link_id, progress, &stable.0, tick.0)
            {
                state.0 = AgentMobilityState::Walking {
                    link_id: next_link,
                    progress: 0.0,
                };
                dirty.0.insert(entity);
            }
            stats.skipped += 1;
            continue;
        }
        let Some(hpa) = hpa.as_deref() else {
            stats.skipped += 1;
            continue;
        };
        let Some(cache) = cache.as_deref_mut() else {
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
        let profile_key = crate::routing::RoutingProfileKey::Walk;
        if origin == destination {
            if is_at_route_endpoint(&graph, &link_id, progress, destination) == Some(true)
                && complete_walk_stage_at_destination(
                    entity,
                    stable,
                    &mut state,
                    &mut plan,
                    stage,
                    &link_id,
                    destination,
                    &mut waiting,
                    &mut dirty,
                    &mut commands,
                )
            {
                stats.skipped += 1;
            } else {
                stats.failed += 1;
            }
            continue;
        }
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

fn next_wander_footway_link(
    graph: &crate::routing::Graph,
    current_link_id: &str,
    progress: f32,
    agent_id: &crate::ids::AgentId,
    tick: u64,
) -> Option<String> {
    let current_edge_id = crate::mobility::api::edge_by_canonical_key(graph, current_link_id)?;
    let current_edge = graph.edge(current_edge_id);
    let node = if progress >= 1.0 {
        current_edge.to
    } else {
        current_edge.from
    };
    let mut candidates: Vec<_> = graph
        .outgoing(node)
        .iter()
        .copied()
        .filter(|edge_id| {
            let edge = graph.edge(*edge_id);
            edge.kind == crate::routing::EdgeKind::Footway && *edge_id != current_edge_id
        })
        .map(|edge_id| crate::mobility::api::canonical_edge_key(graph, edge_id))
        .collect();
    if candidates.is_empty() {
        candidates = graph
            .outgoing(node)
            .iter()
            .copied()
            .filter(|edge_id| graph.edge(*edge_id).kind == crate::routing::EdgeKind::Footway)
            .map(|edge_id| crate::mobility::api::canonical_edge_key(graph, edge_id))
            .collect();
    }
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return None;
    }
    let index = (wander_seed(&agent_id.0, current_link_id, tick) as usize) % candidates.len();
    Some(candidates[index].clone())
}

fn wander_seed(agent_id: &str, current_link_id: &str, tick: u64) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in agent_id
        .as_bytes()
        .iter()
        .chain(current_link_id.as_bytes())
        .chain(tick.to_le_bytes().iter())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
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
            &current_step.canonical_edge_key,
            route.destination,
            &mut waiting,
            &mut dirty,
            &mut commands,
        ) {
            invalidate_active_route(entity, &mut stats, &mut dirty, &mut commands);
        }
    }
}
