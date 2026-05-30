use super::common::{chunk_is_simulated, traffic_route_edge, traffic_route_edge_cache_link_id};
use super::*;

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
    traffic_routes: Res<crate::routing::TrafficRoutes>,
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

    // Vehicles: resolve the current edge via TrafficRoutes + Graph.
    // Synthesize a stable link id from the edge's legacy id (or its numeric
    // EdgeId if the edge has no legacy ancestry) so cache comparison still
    // works against the existing component shape.
    for (entity, rp, cached) in vehicles.iter_mut() {
        let resolved: Option<(String, Vec<(f32, f32)>)> =
            traffic_route_edge(&graph, &traffic_routes, rp).map(|edge| {
                (
                    traffic_route_edge_cache_link_id(edge),
                    edge.polyline.clone(),
                )
            });
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
                crate::mobility::systems::advance_cursor(&mut plan);
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
                // Preserve the arrival link before mutating state so a cyclic
                // agent can be re-anchored there and depart again.
                let arrival_link_id = match &state.0 {
                    AgentMobilityState::Walking { link_id, .. } => Some(link_id.clone()),
                    _ => None,
                };
                crate::mobility::systems::advance_cursor(&mut plan);
                // Cyclic plans never settle at an activity: resume Walking at the
                // arrival edge so route_assignment routes the agent onward toward
                // the next (wrapped) stage. Non-cyclic plans terminate here.
                state.0 = match (plan.cyclic, arrival_link_id) {
                    (true, Some(link_id)) => AgentMobilityState::Walking {
                        link_id,
                        progress: 1.0,
                    },
                    _ => AgentMobilityState::AtActivity { activity_id },
                };
                dirty.0.insert(entity);
            }
            _ => {}
        }
    }
}
