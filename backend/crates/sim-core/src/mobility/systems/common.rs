use super::*;

pub(super) fn dir_at_progress(
    points: &[(f32, f32)],
    progress: f32,
) -> abutown_protocol::DirectionDto {
    crate::mobility_geometry::direction_at_progress_slice(points, progress)
}

pub(super) fn traffic_route_edge<'a>(
    graph: &'a crate::routing::Graph,
    traffic_routes: &crate::routing::TrafficRoutes,
    route_position: &RoutePosition,
) -> Option<&'a crate::routing::Edge> {
    if (route_position.route_id.0 as usize) >= traffic_routes.count() {
        return None;
    }
    let route = traffic_routes.route(route_position.route_id);
    let edge_id = *route.edges.get(route_position.edge_index)?;
    Some(graph.edge(edge_id))
}

pub(super) fn traffic_route_edge_cache_link_id(edge: &crate::routing::Edge) -> String {
    edge.legacy_id
        .clone()
        .unwrap_or_else(|| format!("edge:{}", edge.id.0))
}

pub(super) fn traffic_route_edge_cache_link_id_matches(
    edge: &crate::routing::Edge,
    link_id: &str,
) -> bool {
    match edge.legacy_id.as_deref() {
        Some(legacy_id) => legacy_id == link_id,
        None => link_id == format!("edge:{}", edge.id.0),
    }
}

pub(super) fn current_vehicle_cached_polyline<'a>(
    graph: &crate::routing::Graph,
    traffic_routes: &crate::routing::TrafficRoutes,
    route_position: &RoutePosition,
    cached: Option<&'a CurrentLinkPolyline>,
) -> Option<&'a [(f32, f32)]> {
    let cached = cached?;
    let edge = traffic_route_edge(graph, traffic_routes, route_position)?;
    if traffic_route_edge_cache_link_id_matches(edge, &cached.link_id) {
        Some(cached.points.as_slice())
    } else {
        None
    }
}

/// Returns true if the chunk containing the entity is Active or Hot.
/// Asleep/Warm chunks are skipped by the Advance/Output systems so only
/// hot entities tick at full fidelity. Source of truth: chunk-entity LOD
/// markers; this lookup goes through the `SimulatedChunks` derived view
/// refreshed each tick by `refresh_simulated_chunks_system`.
pub(super) fn chunk_is_simulated(pos: &Position, simulated: &SimulatedChunks) -> bool {
    let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
    simulated.0.contains(&chunk)
}
