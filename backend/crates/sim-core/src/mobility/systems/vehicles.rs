use super::common::chunk_is_simulated;
use super::*;

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
    traffic_routes: Res<crate::routing::TrafficRoutes>,
    mut dirty: ResMut<DirtyVehicles>,
) {
    for (entity, world_pos, mut pos, mut dwell) in query.iter_mut() {
        if !chunk_is_simulated(world_pos, &simulated) {
            continue;
        }
        if (pos.route_id.0 as usize) >= traffic_routes.count() {
            continue;
        }
        let route = traffic_routes.route(pos.route_id);
        if route.edges.is_empty() || pos.edge_index >= route.edges.len() {
            continue;
        }
        if dwell.0 > 0 {
            dwell.0 -= 1;
            dirty.0.insert(entity);
            continue;
        }
        if pos.progress >= 1.0 {
            pos.edge_index = (pos.edge_index + 1) % route.edges.len();
            pos.progress = 0.0;
            dirty.0.insert(entity);
            continue;
        }
        let next = (pos.progress + pos.speed).min(1.0);
        if next != pos.progress {
            pos.progress = next;
            dirty.0.insert(entity);
        }
    }
}
