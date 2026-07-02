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
    waypoints: &crate::mobility::resources::ActivityWaypoints,
) -> Option<crate::routing::NodeId> {
    match stage {
        PlanStage::WalkToStop { stop_id, .. } => graph.node_by_legacy(stop_id),
        PlanStage::WalkToActivity { activity_id, .. } => {
            if let Some(node) = graph.node_by_legacy(activity_id) {
                return Some(node);
            }
            let coord = waypoints.0.get(activity_id).copied().or_else(|| {
                crate::mobility_geometry::activity_geometry(activity_id).map(|g| g.coord)
            })?;
            spatial?.nearest(coord)
        }
        _ => None,
    }
}

fn is_economic_destination_activity(activity_id: &str) -> bool {
    activity_id == "activity:destination"
        || (activity_id.starts_with("activity:") && activity_id.ends_with(":destination"))
}

/// Economic destination override: an attributed citizen's destination leg routes
/// to its bound market node (from `CitizenEconomicTargets`) instead of the
/// geometric corridor endpoint. The `home` leg and unattributed citizens keep
/// `geometric`.
pub(crate) fn economic_destination(
    stage: &PlanStage,
    agent: &crate::ids::AgentId,
    geometric: crate::routing::NodeId,
    targets: Option<&crate::mobility::resources::CitizenEconomicTargets>,
) -> crate::routing::NodeId {
    // "activity:*:destination" is the economic away-from-home leg. Older snapshots
    // may still carry the global "activity:destination" id.
    if let PlanStage::WalkToActivity { activity_id, .. } = stage
        && is_economic_destination_activity(activity_id)
        && let Some(t) = targets
        && let Some(node) = t.0.get(agent)
    {
        return *node;
    }
    geometric
}

pub(crate) fn materialize_route_steps(
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
    waypoints: Res<crate::mobility::resources::ActivityWaypoints>,
    targets: Option<Res<crate::mobility::resources::CitizenEconomicTargets>>,
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
        let Some(geometric_destination) =
            destination_for_stage(&graph, &stage, spatial.as_deref(), &waypoints)
        else {
            stats.failed += 1;
            continue;
        };
        // Deterministic one-tick lag: EconomySet::Attribution and MobilitySet::Advance
        // have no explicit ordering edge in the schedule (both only constrain themselves
        // before tick_increment_system / MobilitySet::Bookkeeping), so this may read the
        // prior tick's CitizenEconomicTargets. Only EconomySet::Attribution writes it
        // (a zero-order hold refreshed on macro-flow delivery ticks), so the lag is
        // deterministic and bounded to one tick — acceptable for Slice 1.
        let destination =
            economic_destination(&stage, &stable.0, geometric_destination, targets.as_deref());
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
    waypoints: Res<crate::mobility::resources::ActivityWaypoints>,
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
        if destination_for_stage(&graph, &stage, spatial.as_deref(), &waypoints)
            != Some(route.destination)
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind, NodeSpatialIndex};

    fn minimal_graph_with_node_at_5_5() -> (Graph, NodeSpatialIndex) {
        let graph = Graph::new(
            vec![
                Node {
                    id: NodeId(0),
                    position: (0.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(1),
                    position: (5.0, 5.0),
                    kind: NodeKind::ActivityLocation,
                    legacy_id: None,
                },
            ],
            vec![Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (5.0, 5.0)],
                length: 7.07,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:test".into()),
            }],
        );
        let spatial = NodeSpatialIndex::from_nodes(graph.nodes());
        (graph, spatial)
    }

    #[test]
    fn destination_for_activity_prefers_waypoints_over_static_geometry() {
        let (graph, spatial) = minimal_graph_with_node_at_5_5();
        let mut wp = crate::mobility::resources::ActivityWaypoints::default();
        wp.0.insert("activity:home".to_string(), (5.0, 5.0));
        let stage = PlanStage::WalkToActivity {
            link_id: "walk:test".into(),
            activity_id: "activity:home".into(),
        };
        assert_eq!(
            destination_for_stage(&graph, &stage, Some(&spatial), &wp),
            spatial.nearest((5.0, 5.0)),
        );
    }

    #[test]
    fn destination_for_activity_without_declared_geometry_returns_none() {
        let (graph, spatial) = minimal_graph_with_node_at_5_5();
        let wp = crate::mobility::resources::ActivityWaypoints::default();
        let stage = PlanStage::WalkToActivity {
            link_id: "walk:test".into(),
            activity_id: "activity:home".into(),
        };
        assert_eq!(
            destination_for_stage(&graph, &stage, Some(&spatial), &wp),
            None
        );
    }

    #[test]
    fn home_leg_is_never_overridden() {
        // A `WalkToActivity{activity_id:"activity:home"}` stage must NOT be overridden
        // even when `CitizenEconomicTargets` maps the agent to a market node.
        let agent = crate::ids::AgentId("agent-1".into());
        let market_node = crate::routing::NodeId(99);
        let geometric = crate::routing::NodeId(1);
        let stage = PlanStage::WalkToActivity {
            link_id: "walk:test".into(),
            activity_id: "activity:home".into(),
        };
        let mut targets = crate::mobility::resources::CitizenEconomicTargets::default();
        targets.0.insert(agent.clone(), market_node);
        let result = economic_destination(&stage, &agent, geometric, Some(&targets));
        assert_eq!(result, geometric, "home leg must not be overridden");
    }

    #[test]
    fn destination_leg_with_target_is_overridden() {
        // A `WalkToActivity{activity_id:"activity:destination"}` stage for an attributed
        // citizen must route to its bound market node.
        let agent = crate::ids::AgentId("agent-2".into());
        let market_node = crate::routing::NodeId(42);
        let geometric = crate::routing::NodeId(7);
        let stage = PlanStage::WalkToActivity {
            link_id: "walk:test".into(),
            activity_id: "activity:destination".into(),
        };
        let mut targets = crate::mobility::resources::CitizenEconomicTargets::default();
        targets.0.insert(agent.clone(), market_node);
        let result = economic_destination(&stage, &agent, geometric, Some(&targets));
        assert_eq!(
            result, market_node,
            "destination leg must be overridden to market node"
        );
    }

    #[test]
    fn grouped_destination_leg_with_target_is_overridden() {
        let agent = crate::ids::AgentId("agent-2".into());
        let market_node = crate::routing::NodeId(42);
        let geometric = crate::routing::NodeId(7);
        let stage = PlanStage::WalkToActivity {
            link_id: "walk:test".into(),
            activity_id: "activity:spawn:ped:north:destination".into(),
        };
        let mut targets = crate::mobility::resources::CitizenEconomicTargets::default();
        targets.0.insert(agent.clone(), market_node);
        let result = economic_destination(&stage, &agent, geometric, Some(&targets));
        assert_eq!(
            result, market_node,
            "grouped destination leg must be overridden to market node"
        );
    }

    #[test]
    fn destination_leg_without_target_is_geometric() {
        // A `WalkToActivity{activity_id:"activity:destination"}` stage for an unattributed
        // citizen (not in CitizenEconomicTargets) must fall back to the geometric endpoint.
        let agent = crate::ids::AgentId("agent-3".into());
        let geometric = crate::routing::NodeId(5);
        let stage = PlanStage::WalkToActivity {
            link_id: "walk:test".into(),
            activity_id: "activity:destination".into(),
        };
        // Empty targets: agent is not attributed.
        let targets = crate::mobility::resources::CitizenEconomicTargets::default();
        let result = economic_destination(&stage, &agent, geometric, Some(&targets));
        assert_eq!(
            result, geometric,
            "unattributed citizen must use geometric destination"
        );

        // Also test with None targets (economy plugin absent).
        let result_no_targets = economic_destination(&stage, &agent, geometric, None);
        assert_eq!(
            result_no_targets, geometric,
            "None targets must use geometric destination"
        );
    }
}
