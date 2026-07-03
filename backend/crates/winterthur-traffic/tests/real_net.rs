//! Integration test on the REAL baked Winterthur network: build a `Router`,
//! compute a route between two far-apart edges, assert the lane path is
//! fully connected via real turns, then spawn it into a `traffic_core::Core`
//! and tick a handful of times without panicking.

use std::path::PathBuf;
use traffic_core::Core;
use traffic_net::TrafficNet;
use winterthur_traffic::Router;

fn load_real_net() -> TrafficNet {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crate dir is backend/crates/winterthur-traffic; repo root is three up.
    p.pop();
    p.pop();
    p.pop();
    p.push("data/winterthur/trafficnet.json");
    let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    traffic_net::load(&json).expect("real Winterthur bake must validate")
}

#[test]
fn route_on_real_net_is_drivable_and_ticks_without_panicking() {
    let net = load_real_net();
    let router = Router::new(&net);

    // Two edges ~1.2km apart (picked via BFS over the edge/turn graph for a
    // pair that is actually reachable — this real net has one-way streets,
    // so not every geographically-far pair is connected in both directions),
    // both with at least one outgoing turn so they're valid route endpoints.
    let from_edge = 1083u32;
    let to_edge = 498u32;

    let route = router
        .route(&net, from_edge, to_edge)
        .expect("route between two far-apart real-net edges must exist");
    assert!(
        route.len() >= 2,
        "expected a multi-hop route, got {route:?}"
    );

    // Every consecutive lane pair must be connected by a real turn.
    for w in route.windows(2) {
        let turns = net.turns_from(w[0]);
        assert!(
            turns
                .iter()
                .any(|&tid| net.turns[tid as usize].to_lane == w[1]),
            "no turn from lane {} to lane {} in route {:?}",
            w[0],
            w[1],
            route
        );
    }

    // Spawn the computed route into a minimal Core and tick a handful of
    // times without panicking (safety-net smoke, not a behavioral assertion).
    let mut core = Core::new(&net, 8, 0xF00D);
    let start_lane = route[0];
    let s0 = (net.lane_len(start_lane) * 0.2).clamp(1.0, 5.0);
    let veh = core
        .spawn(start_lane, s0, &route)
        .expect("spawn on the computed route must succeed");
    let _ = veh;

    for t in 0..50 {
        core.tick(t);
    }
}
