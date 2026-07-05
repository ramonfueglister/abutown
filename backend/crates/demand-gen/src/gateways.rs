//! Commune → gateway mapping: each external commune is assigned a boundary
//! gateway by **bearing with motorway preference** (spec §4.2): if the
//! commune centroid is > 8 km from the Gemeinde centroid and a motorway
//! gateway (with a lane in the needed direction) lies within ±60° of the
//! bearing, take the angularly nearest such motorway gateway; otherwise the
//! bearing-nearest gateway of any class that has the needed direction.
//! Gateways lacking the needed direction (one-way stubs) are never picked —
//! that IS the deterministic "bearing-next" fallback.

use traffic_net::TrafficNet;

/// Edges at/above this free-flow speed are motorways (100 km/h = 27.78 m/s).
pub const MOTORWAY_SPEED_MS: f32 = 27.0;
/// Communes farther than this get the motorway preference.
pub const MOTORWAY_PREF_KM: f64 = 8.0;
/// Half-angle of the motorway-preference cone (±60°).
pub const MOTORWAY_PREF_HALF_ANGLE: f64 = std::f64::consts::FRAC_PI_3;

/// Which lane direction a demand trip needs at the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Spawning INTO the net: a lane on an edge leaving the gateway node.
    Spawn,
    /// Leaving the net: a lane on an edge ending at the gateway node.
    Sink,
}

/// Per-gateway lookup info, precomputed from the net.
#[derive(Debug, Clone)]
pub struct GwInfo {
    pub node: u32,
    pub x: f32,
    pub z: f32,
    /// Bearing from the world origin (= Gemeinde centroid anchor) in radians,
    /// `atan2(east, north)`, range `(-π, π]`.
    pub bearing: f64,
    /// True if any adjacent edge is motorway-speed.
    pub motorway: bool,
    /// Lowest lane id of edges leaving this gateway (spawn direction), if any.
    pub spawn_lane: Option<u32>,
    /// Lowest lane id of edges ending at this gateway (sink direction), if any.
    pub sink_lane: Option<u32>,
}

/// Bearing of a world-space point from the origin: `atan2(east, north)` with
/// north = -z (world frame is x=east, z=south).
pub fn bearing_of(x: f64, z: f64) -> f64 {
    x.atan2(-z)
}

/// Wrap-aware angular distance in `[0, π]`.
pub fn ang_dist(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    d.min(std::f64::consts::TAU - d)
}

/// Extract [`GwInfo`] for every gateway node, sorted by node id.
pub fn gateway_infos(net: &TrafficNet) -> Vec<GwInfo> {
    net.gateways()
        .iter()
        .map(|&node_id| {
            let node = net
                .nodes
                .iter()
                .find(|n| n.id == node_id)
                .expect("gateways() ids come from nodes");
            let mut motorway = false;
            let mut spawn_lane: Option<u32> = None;
            let mut sink_lane: Option<u32> = None;
            for e in &net.edges {
                if e.from != node_id && e.to != node_id {
                    continue;
                }
                motorway |= e.speed_ms >= MOTORWAY_SPEED_MS;
                let min_lane = e.lanes.iter().copied().min();
                if e.from == node_id {
                    spawn_lane = min_opt(spawn_lane, min_lane);
                }
                if e.to == node_id {
                    sink_lane = min_opt(sink_lane, min_lane);
                }
            }
            GwInfo {
                node: node_id,
                x: node.x,
                z: node.z,
                bearing: bearing_of(node.x as f64, node.z as f64),
                motorway,
                spawn_lane,
                sink_lane,
            }
        })
        .collect()
}

fn min_opt(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (x, None) | (None, x) => x,
    }
}

/// Pick the gateway for a commune at `bearing` / `dist_km` needing `need`,
/// per the bearing-with-motorway-preference rule. Ties break on lower node
/// id. Returns `None` only if NO gateway has the needed direction.
pub fn pick<'a>(infos: &'a [GwInfo], bearing: f64, dist_km: f64, need: Need) -> Option<&'a GwInfo> {
    let has_lane = |g: &GwInfo| match need {
        Need::Spawn => g.spawn_lane.is_some(),
        Need::Sink => g.sink_lane.is_some(),
    };
    // total order on (angular distance, node id) — f64 ang distance is finite
    let best_of = |it: &mut dyn Iterator<Item = &'a GwInfo>| -> Option<&'a GwInfo> {
        it.min_by(|a, b| {
            let da = ang_dist(a.bearing, bearing);
            let db = ang_dist(b.bearing, bearing);
            da.partial_cmp(&db)
                .expect("angular distances are finite")
                .then(a.node.cmp(&b.node))
        })
    };
    if dist_km > MOTORWAY_PREF_KM {
        let mut cone = infos.iter().filter(|g| {
            g.motorway && has_lane(g) && ang_dist(g.bearing, bearing) <= MOTORWAY_PREF_HALF_ANGLE
        });
        if let Some(g) = best_of(&mut cone) {
            return Some(g);
        }
    }
    let mut any = infos.iter().filter(|g| has_lane(g));
    best_of(&mut any)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gw(node: u32, bearing_deg: f64, motorway: bool, spawn: bool, sink: bool) -> GwInfo {
        GwInfo {
            node,
            x: 0.0,
            z: 0.0,
            bearing: bearing_deg.to_radians(),
            motorway,
            spawn_lane: spawn.then_some(node * 10),
            sink_lane: sink.then_some(node * 10 + 1),
        }
    }

    #[test]
    fn far_commune_prefers_motorway_gateway_within_cone() {
        let infos = vec![
            gw(1, 0.0, true, true, true),   // motorway, dead ahead
            gw(2, 5.0, false, true, true),  // local road, angularly closer
            gw(3, 170.0, true, true, true), // motorway, far outside cone
        ];
        // 20 km away at bearing 5° → motorway within ±60° wins over the
        // angularly-nearest local gateway
        let g = pick(&infos, 5.0f64.to_radians(), 20.0, Need::Spawn).unwrap();
        assert_eq!(g.node, 1);
    }

    #[test]
    fn near_commune_takes_bearing_nearest_any_class() {
        let infos = vec![gw(1, 0.0, true, true, true), gw(2, 5.0, false, true, true)];
        // 3 km away → no motorway preference, local gateway at 5° is nearest
        let g = pick(&infos, 6.0f64.to_radians(), 3.0, Need::Spawn).unwrap();
        assert_eq!(g.node, 2);
    }

    #[test]
    fn far_commune_without_cone_motorway_falls_back_to_bearing_nearest() {
        let infos = vec![
            gw(2, 5.0, false, true, true),
            gw(3, 170.0, true, true, true),
        ];
        let g = pick(&infos, 0.0, 20.0, Need::Spawn).unwrap();
        assert_eq!(g.node, 2);
    }

    #[test]
    fn one_way_stub_falls_back_to_next_gateway_with_direction() {
        let infos = vec![
            gw(1, 0.0, false, true, false), // spawn-only stub
            gw(2, 10.0, false, false, true),
        ];
        // needs a sink → gateway 1 (bearing-nearest) lacks it → gateway 2
        let g = pick(&infos, 0.0, 3.0, Need::Sink).unwrap();
        assert_eq!(g.node, 2);
        // needs a spawn → gateway 1
        let g = pick(&infos, 0.0, 3.0, Need::Spawn).unwrap();
        assert_eq!(g.node, 1);
    }

    #[test]
    fn bearing_wraps_across_pi() {
        let infos = vec![
            gw(1, 175.0, false, true, true),
            gw(2, -90.0, false, true, true),
        ];
        // bearing -175° is 10° from +175° across the wrap, 85° from -90°
        let g = pick(&infos, (-175.0f64).to_radians(), 3.0, Need::Spawn).unwrap();
        assert_eq!(g.node, 1);
    }

    #[test]
    fn bearing_of_world_frame() {
        // x=east, z=south; north = -z → a point due north has bearing 0
        assert!(bearing_of(0.0, -100.0).abs() < 1e-9);
        // due east → +90°
        assert!((bearing_of(100.0, 0.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        // due south → ±180°
        assert!((bearing_of(0.0, 100.0).abs() - std::f64::consts::PI).abs() < 1e-9);
    }
}
