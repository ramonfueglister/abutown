//! Integration tests for MOBIL lane changes (Task 4).
//!
//! A synthetic **two-lane** closed ring (a 250 m × 4-side square, each side a
//! 2-lane edge) exercises the lane-change machinery end-to-end:
//!  1. **Overtake + return** — a fast car stuck behind a slow truck in the
//!     right lane changes left, passes, and (keep-right bias) returns to the
//!     right within a bounded number of ticks, with no collisions.
//!  2. **Safety veto** — a car boxed in by a fast vehicle in the adjacent lane
//!     does not change into it.
//!  3. **Determinism** — same seed → identical hash; identical across 1 vs 4
//!     rayon threads, with lane changes active.
//!
//! Direction convention (see `traffic_core::mobil`): lane **index 0 = right**,
//! **index 1 = left**. Overtaking moves right→left; the keep-right bias pulls a
//! vehicle back to index 0 once the left lane is clear.

use rayon::ThreadPoolBuilder;
use traffic_core::{Core, IdmParams, MobilParams};
use traffic_net::TrafficNet;

const SIDE: f32 = 250.0;
const RING_LEN: f32 = 1000.0;
const N_LANES_PER_EDGE: u32 = 2;

/// Build a 1000 m closed **two-lane** ring. Node `i` at a square corner; edge
/// `i` runs corner i -> i+1 and carries two lanes (index 0 and 1). Lane ids are
/// dense `0..8`: edge `e` owns lanes `2e` (index 0, right) and `2e+1` (index 1,
/// left). Each corner node has covering turns lane->lane onto the next edge for
/// both indices (satisfies validate's turn-coverage rule; Task 5 will make turns
/// meaningful — here they only exist to validate).
fn two_lane_ring_net() -> TrafficNet {
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
    let mut turn_id = 0u32;
    for e in 0..4u32 {
        let from = e;
        let to = (e + 1) % 4;
        let a = corners[from as usize];
        let b = corners[to as usize];
        let l0 = 2 * e; // index 0 (right)
        let l1 = 2 * e + 1; // index 1 (left)
        if e > 0 {
            edges.push(',');
            lanes.push(',');
        }
        edges.push_str(&format!(
            r#"{{"id":{e},"from":{from},"to":{to},"speedMs":13.9,"laneCount":2,"priorityRoad":false,"lanes":[{l0},{l1}]}}"#
        ));
        // Two parallel lanes with identical polyline (same arc-length param).
        lanes.push_str(&format!(
            r#"{{"id":{l0},"edge":{e},"index":0,"lengthM":{SIDE},"pts":[[{},{}],[{},{}]]}},"#,
            a[0], a[1], b[0], b[1]
        ));
        lanes.push_str(&format!(
            r#"{{"id":{l1},"edge":{e},"index":1,"lengthM":{SIDE},"pts":[[{},{}],[{},{}]]}}"#,
            a[0], a[1], b[0], b[1]
        ));
        // Turns onto the next edge's matching-index lane, for both indices.
        let next = (e + 1) % 4;
        let n0 = 2 * next;
        let n1 = 2 * next + 1;
        for (fl, tl) in [(l0, n0), (l1, n1)] {
            if turn_id > 0 {
                turns.push(',');
            }
            turns.push_str(&format!(
                r#"{{"id":{turn_id},"fromLane":{fl},"toLane":{tl},"node":{to},"conflictsWith":[],"yieldsTo":[]}}"#
            ));
            turn_id += 1;
        }
    }

    let json = format!(
        r#"{{"meta":{{"anchor":{{"lon":0.0,"lat":0.0}},"laneWidth":3.5,"cellSize":1.0}},
            "nodes":[{nodes}],"edges":[{edges}],"lanes":[{lanes}],"turns":[{turns}]}}"#
    );

    traffic_net::load(&json).expect("synthetic two-lane ring net must validate")
}

/// The lane id for edge `e`, index `idx` (0=right, 1=left).
fn lane_id(e: u32, idx: u32) -> u32 {
    N_LANES_PER_EDGE * e + idx
}

/// A route around the ring on a fixed lane index, starting at edge `start_e`.
fn route_on_index(start_e: u32, idx: u32) -> Vec<u32> {
    (0..4).map(|k| lane_id((start_e + k) % 4, idx)).collect()
}

/// Global arc position of a vehicle (edge order along the ring), for gap /
/// occupancy checks independent of the kernel's internal lookups.
fn global_s(core: &Core, i: usize) -> f32 {
    let lane = core.fleet.lane[i];
    let edge = lane / N_LANES_PER_EDGE;
    edge as f32 * SIDE + core.fleet.s[i]
}

fn lane_index(core: &Core, i: usize) -> u32 {
    core.fleet.lane[i] % N_LANES_PER_EDGE
}

/// Per-lane-index minimum bumper-to-bumper gap (vehicles only conflict within
/// the same lane index, since the two indices are physically separate lanes).
fn min_gap_within_lanes(core: &Core) -> f32 {
    let mut min = f32::INFINITY;
    for idx in 0..N_LANES_PER_EDGE {
        let mut pos: Vec<f32> = Vec::new();
        for i in 0..core.fleet.slots() {
            if core.fleet.alive[i] && lane_index(core, i) == idx {
                pos.push(global_s(core, i));
            }
        }
        pos.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = pos.len();
        for k in 0..n {
            let cur = pos[k];
            let next = if k + 1 < n {
                pos[k + 1]
            } else {
                pos[0] + RING_LEN
            };
            min = min.min((next - cur) - 4.5);
        }
    }
    min
}

#[test]
fn fast_car_overtakes_slow_truck_and_returns_right() {
    let net = two_lane_ring_net();
    let mut core = Core::new(&net, 8, 0xBEEF);
    // Highway-stable IDM so the only dynamics are car-following + the lane
    // change (no spurious stop-and-go). A brisk v0 gives the car room to pass.
    core.set_params(IdmParams {
        v0: 20.0,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });
    core.set_mobil(MobilParams::default());

    // A slow "truck": crawling, in the RIGHT lane (index 0) of edge 0 at s=120.
    let truck = core
        .spawn(lane_id(0, 0), 120.0, 0, &route_on_index(0, 0))
        .expect("spawn truck");
    // A fast car behind it in the same right lane at s=40, initialised at speed.
    let car = core
        .spawn(lane_id(0, 0), 40.0, 0, &route_on_index(0, 0))
        .expect("spawn car");
    // Pin the truck slow by giving the car a head-start in speed; both use the
    // same IDM, but the car approaches and must decide to overtake.
    core.fleet.v[truck as usize] = 2.0;
    core.fleet.v[car as usize] = 18.0;
    core.reindex();

    let mut car_changed_left = false;
    let mut car_passed_truck = false;
    let mut car_returned_right = false;

    // Run up to N ticks; assert no collision every tick.
    for t in 0..1500u64 {
        core.tick(t);
        let g = min_gap_within_lanes(&core);
        assert!(g > -0.01, "collision at tick {t}: min within-lane gap {g}");

        let ci = car as usize;
        let ti = truck as usize;
        if lane_index(&core, ci) == 1 {
            car_changed_left = true;
        }
        // "Passed" = car's global position is ahead of the truck's (allowing
        // wrap) once it has moved left at some point.
        if car_changed_left {
            let dc = global_s(&core, ci);
            let dt = global_s(&core, ti);
            // ahead within half a ring (accounts for wrap)
            let ahead = (dc - dt).rem_euclid(RING_LEN);
            if (1.0..RING_LEN / 2.0).contains(&ahead) {
                car_passed_truck = true;
            }
        }
        if car_passed_truck && lane_index(&core, ci) == 0 {
            car_returned_right = true;
        }
        if car_returned_right {
            break;
        }
    }

    assert!(
        car_changed_left,
        "car never changed to the left lane to overtake"
    );
    assert!(car_passed_truck, "car never got ahead of the truck");
    assert!(
        car_returned_right,
        "car never returned to the right lane (keep-right bias)"
    );
}

#[test]
fn safety_veto_blocks_change_into_fast_adjacent_traffic() {
    let net = two_lane_ring_net();
    let mut core = Core::new(&net, 8, 0x5AFE);
    core.set_params(IdmParams {
        v0: 20.0,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });
    core.set_mobil(MobilParams::default());

    // Slow car in the RIGHT lane behind a slow leader -> it WOULD want to move
    // left, but the LEFT lane has a fast car right beside/behind it, so the
    // safety criterion must veto the change.
    let leader = core
        .spawn(lane_id(0, 0), 60.0, 0, &route_on_index(0, 0))
        .expect("leader");
    let boxed = core
        .spawn(lane_id(0, 0), 40.0, 0, &route_on_index(0, 0))
        .expect("boxed");
    // Fast car in the left lane just behind `boxed`'s position (small gap, high
    // speed) — cutting in front of it would force a hard brake.
    let fast_left = core
        .spawn(lane_id(0, 1), 38.0, 0, &route_on_index(0, 1))
        .expect("fast_left");
    core.fleet.v[leader as usize] = 3.0;
    core.fleet.v[boxed as usize] = 4.0;
    core.fleet.v[fast_left as usize] = 20.0;
    core.reindex();

    // For the first few ticks (while the fast car is still dangerously close
    // behind in the left lane), `boxed` must not be sitting in the left lane.
    let bi = boxed as usize;
    for t in 0..8u64 {
        core.tick(t);
        // No collision.
        assert!(min_gap_within_lanes(&core) > -0.01, "collision at tick {t}");
        // The boxed car must not have cut in unsafely at the very first ticks
        // while the fast car was still right behind it.
        if t < 3 {
            assert_eq!(
                lane_index(&core, bi),
                0,
                "boxed car unsafely cut into the left lane at tick {t}"
            );
        }
    }
}

/// Same-seed + across-thread-count determinism, with lane changes active. Uses
/// a denser two-lane ring so lane changes actually fire during the run.
fn crowded_core(seed: u64) -> Core {
    let net = two_lane_ring_net();
    let mut core = Core::new(&net, 20, seed);
    core.set_params(IdmParams {
        v0: 18.0,
        t_headway: 1.2,
        a_max: 1.2,
        b_comf: 2.0,
        s0: 2.0,
    });
    core.set_mobil(MobilParams::default());
    // 16 vehicles: alternate lanes and speeds so MOBIL has work to do.
    for k in 0..16u32 {
        let e = k % 4;
        let idx = (k / 4) % 2;
        let s = 20.0 + (k as f32) * 11.0 % (SIDE - 20.0);
        let id = core
            .spawn(lane_id(e, idx), s, 0, &route_on_index(e, idx))
            .expect("spawn crowded");
        core.fleet.v[id as usize] = 5.0 + (k as f32 * 1.7) % 12.0;
    }
    core.reindex();
    core
}

#[test]
fn deterministic_same_seed_with_lane_changes() {
    let mut a = crowded_core(0x1111);
    let mut b = crowded_core(0x1111);
    for t in 0..2000u64 {
        a.tick(t);
        b.tick(t);
    }
    assert_eq!(a.state_hash(), b.state_hash(), "same seed must match");
}

#[test]
fn deterministic_across_thread_counts_with_lane_changes() {
    let run = |threads: usize| -> u64 {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut core = crowded_core(0x2222);
            for t in 0..2000u64 {
                core.tick(t);
            }
            core.state_hash()
        })
    };
    assert_eq!(run(1), run(4), "thread count must not affect state hash");
}

/// Physical-fit regression (S2 collision on the real net): a MANDATORY
/// lane change (waived comfort threshold) must never place a vehicle
/// overlapping a leader already on the target lane. Two-lane edge where only
/// lane 0 serves the route; the wrong-lane vehicle is forced to merge, but
/// lane 0 is packed with a standing queue right at its merge position — the
/// merge must be REFUSED (no overlap), not forced on top of the queue.
#[test]
fn mandatory_merge_never_overlaps_target_leader() {
    let json = r#"{
      "meta": {"anchor":{"lon":0.0,"lat":0.0},"laneWidth":3.5,"cellSize":1.0},
      "nodes": [
        {"id":0,"x":0,"z":0,"kind":"dead_end","signal":null},
        {"id":1,"x":200,"z":0,"kind":"uncontrolled","signal":null},
        {"id":2,"x":300,"z":0,"kind":"dead_end","signal":null}
      ],
      "edges": [
        {"id":0,"from":0,"to":1,"speedMs":13.9,"laneCount":2,"priorityRoad":false,"lanes":[0,1]},
        {"id":1,"from":1,"to":2,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[2]}
      ],
      "lanes": [
        {"id":0,"edge":0,"index":0,"lengthM":200,"pts":[[0,0],[200,0]]},
        {"id":1,"edge":0,"index":1,"lengthM":200,"pts":[[0,-3.5],[200,-3.5]]},
        {"id":2,"edge":1,"index":0,"lengthM":100,"pts":[[200,0],[300,0]]}
      ],
      "turns": [
        {"id":0,"fromLane":0,"toLane":2,"node":1,"conflictsWith":[],"yieldsTo":[]}
      ]
    }"#;
    let net = traffic_net::load(json).expect("merge fixture must validate");
    let mut core = Core::new(&net, 32, 0xC0DE);
    // Pack lane 0 with a standing queue occupying the whole merge zone.
    for k in 0..20u32 {
        core.spawn(0, 5.0 + k as f32 * 6.0, 0, &[0u32, 2])
            .expect("queue on serving lane");
    }
    // The wrong-lane vehicle: on lane 1, which has no turn to the exit.
    core.spawn(1, 60.0, 0, &[1u32, 2])
        .expect("wrong-lane vehicle");
    core.reindex();

    // Run; every tick the minimum bumper gap must stay positive (no overlap).
    for t in 0..1500 {
        core.tick(t);
        // Min gap per lane across the fleet.
        for lane in 0..3u32 {
            let occ = core.index.on_lane(lane);
            for w in occ.windows(2) {
                let a = w[0] as usize; // higher s (leader)
                let b = w[1] as usize; // lower s (follower)
                let gap = core.fleet.s[a] - core.fleet.s[b] - core.fleet.len_m[a];
                assert!(
                    gap > -0.01,
                    "overlap on lane {lane} at tick {t}: gap {gap:.2}"
                );
            }
        }
        if core.fleet.alive_count() == 0 {
            break;
        }
    }
}
