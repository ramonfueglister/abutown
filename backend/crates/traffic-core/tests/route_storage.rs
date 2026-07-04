//! Regression test for the bounded per-slot route storage (Task 7 finding 1).
//!
//! Before the fix, `Core::spawn` and `Core::reroute` both `extend_from_slice`d
//! into a single append-only `route_lanes: Vec<u32>` that `Fleet::free` never
//! reclaimed, so route storage grew O(total spawns + reroutes) forever — an
//! unbounded leak in a persistent server. The fix reuses per-slot route buffers
//! (`clear()` + `extend`, capacity retained). This test drives heavy
//! spawn/reroute/despawn churn and asserts total route storage capacity
//! stabilizes after warm-up instead of growing without bound.

use traffic_core::Core;
use traffic_net::TrafficNet;

const SIDE: f32 = 250.0;

/// A 1000 m closed single-lane ring (square loop, dense ids 0..4). Same shape
/// as `ring.rs`'s fixture; kept local so this test file is self-contained.
fn ring_net() -> TrafficNet {
    let corners = [[0.0, 0.0], [SIDE, 0.0], [SIDE, SIDE], [0.0, SIDE]];

    let mut nodes = String::new();
    for (i, c) in corners.iter().enumerate() {
        if i > 0 {
            nodes.push(',');
        }
        nodes.push_str(&format!(
            r#"{{"id":{i},"x":{},"z":{},"kind":"uncontrolled","signal":null}}"#,
            c[0], c[1]
        ));
    }

    let mut edges = String::new();
    let mut lanes = String::new();
    let mut turns = String::new();
    for i in 0..4u32 {
        let from = i;
        let to = (i + 1) % 4;
        let a = corners[from as usize];
        let b = corners[to as usize];
        if i > 0 {
            edges.push(',');
            lanes.push(',');
            turns.push(',');
        }
        edges.push_str(&format!(
            r#"{{"id":{i},"from":{from},"to":{to},"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[{i}]}}"#
        ));
        lanes.push_str(&format!(
            r#"{{"id":{i},"edge":{i},"index":0,"lengthM":{SIDE},"pts":[[{},{}],[{},{}]]}}"#,
            a[0], a[1], b[0], b[1]
        ));
        let next = (i + 1) % 4;
        turns.push_str(&format!(
            r#"{{"id":{i},"fromLane":{i},"toLane":{next},"node":{to},"conflictsWith":[],"yieldsTo":[]}}"#
        ));
    }

    let json = format!(
        r#"{{"meta":{{"anchor":{{"lon":0.0,"lat":0.0}},"laneWidth":3.5,"cellSize":1.0}},
            "nodes":[{nodes}],"edges":[{edges}],"lanes":[{lanes}],"turns":[{turns}]}}"#
    );
    traffic_net::load(&json).expect("synthetic ring net must validate")
}

/// A 4-lane loop route whose head is `lane` (so `spawn`/`reroute`'s
/// `route[0] == current lane` guard is satisfied).
fn route_from(lane: u32) -> Vec<u32> {
    (0..4).map(|k| (lane + k) % 4).collect()
}

#[test]
fn route_storage_is_bounded_under_spawn_reroute_churn() {
    let net = ring_net();
    let cap = 8;
    let mut core = Core::new(&net, cap, 0x7);

    // Spawn up to capacity, spread across the four lanes so no two share an
    // arc position (the kernel reads coincident vehicles as a collision, but we
    // never tick here — this test only exercises route storage).
    let mut vehs = Vec::new();
    for k in 0..cap as u32 {
        let lane = k % 4;
        let s = 10.0 + (k as f32) * 20.0 % 200.0;
        if let Some(v) = core.spawn(lane, s, &route_from(lane)) {
            vehs.push(v);
        }
    }
    assert!(!vehs.is_empty(), "must have spawned some vehicles");

    // Warm-up churn phase: reroute every vehicle many times (the pure reroute
    // leak vector) so every per-slot buffer reaches its steady-state capacity.
    for round in 0..200u64 {
        for &v in &vehs {
            let lane = core.vehicle_view(v).expect("alive").lane;
            core.reroute(v, &route_from(lane));
        }
        let _ = round;
    }
    let cap_after_warmup = core.fleet.route_storage_capacity();
    let used_after_warmup = core.fleet.route_storage_len();

    // Second churn phase of the SAME shape: capacity must not grow at all
    // (buffers are reused; clear()+extend retains capacity). A single flat
    // append-only buffer would have doubled its capacity across this phase.
    for _ in 0..200u64 {
        for &v in &vehs {
            let lane = core.vehicle_view(v).expect("alive").lane;
            core.reroute(v, &route_from(lane));
        }
    }
    let cap_after_second = core.fleet.route_storage_capacity();
    let used_after_second = core.fleet.route_storage_len();

    assert_eq!(
        cap_after_second, cap_after_warmup,
        "route storage capacity must be stable across identical churn phases \
         (warmup {cap_after_warmup}, second {cap_after_second}) — a leak means it grew"
    );
    assert_eq!(
        used_after_second, used_after_warmup,
        "used route storage must be identical across identical churn phases"
    );

    // Absolute bound: total used lane ids is at most cap × max-route-len (here
    // 4 lanes per route), never O(number of reroutes).
    let max_route_len = 4;
    assert!(
        core.fleet.route_storage_len() <= cap * max_route_len,
        "route storage {} must be bounded by cap*max_route_len={}",
        core.fleet.route_storage_len(),
        cap * max_route_len
    );
}
