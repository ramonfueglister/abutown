use super::common::{
    chunk_is_simulated, current_vehicle_cached_polyline, dir_at_progress, traffic_route_edge,
};
use super::*;

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
    traffic_routes: Res<crate::routing::TrafficRoutes>,
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
        let new_xy = if let Some(points) =
            current_vehicle_cached_polyline(&graph, &traffic_routes, rp, cached)
        {
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                points,
                rp.progress,
            ))
        } else {
            crate::mobility::vehicle_world_coord(rp, &traffic_routes, &graph)
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
                crate::mobility::agent_world_coord(&state.0, &graph)
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
    traffic_routes: Res<crate::routing::TrafficRoutes>,
    graph: Res<crate::routing::Graph>,
) {
    for (pos, rp, mut dir, cached) in vehicles.iter_mut() {
        if !chunk_is_simulated(pos, &simulated) {
            continue;
        }
        if let Some(points) = current_vehicle_cached_polyline(&graph, &traffic_routes, rp, cached) {
            dir.0 = dir_at_progress(points, rp.progress);
            continue;
        }
        if let Some(edge) = traffic_route_edge(&graph, &traffic_routes, rp) {
            dir.0 = dir_at_progress(&edge.polyline, rp.progress);
        }
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
