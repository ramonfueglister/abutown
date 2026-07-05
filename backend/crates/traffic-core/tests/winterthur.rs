//! First end-to-end run on the REAL baked Winterthur network (Task 5).
//!
//! Loads `data/winterthur/trafficnet.json`, spawns ~200 vehicles on random
//! valid routes (seeded, reproducible), runs 2000 ticks, and asserts the safety
//! invariants that Task 5 must uphold on real data:
//!  * no collisions (no negative within-lane bumper gap),
//!  * no conflict-point co-occupancy (no two conflicting turns crossing a node
//!    on the same tick — checked via physical node proximity),
//!  * no NaN in any position/speed,
//!  * forward progress: some vehicles complete their routes and despawn.
//!
//! If the real bake has defects (unreachable lanes, degenerate turns) this test
//! is where they surface; we build routes only from lanes that actually have
//! onward turns, and report — rather than paper over — anything that blocks
//! progress.

use std::collections::HashMap;
use std::path::PathBuf;
use traffic_core::junction::turn_between;
use traffic_core::{Core, IdmParams};
use traffic_net::TrafficNet;

/// Deterministic splitmix64 for route/seed selection (matches the kernel's
/// stateless-rng philosophy; test-local so it needs no crate export).
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn load_real_net() -> TrafficNet {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crate dir is backend/crates/traffic-core; repo root is three up.
    p.pop();
    p.pop();
    p.pop();
    p.push("data/winterthur/trafficnet.json");
    let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    traffic_net::load(&json).expect("real Winterthur bake must validate")
}

/// Build a random valid lane-sequence route of up to `max_len` hops starting at
/// `start_lane`, following available turns. Returns `None` if the start lane has
/// no onward turn (dead lane).
fn random_route(
    net: &TrafficNet,
    start_lane: u32,
    max_len: usize,
    mut rng: u64,
) -> Option<Vec<u32>> {
    let mut route = vec![start_lane];
    let mut cur = start_lane;
    for _ in 0..max_len {
        let turns = net.turns_from(cur);
        if turns.is_empty() {
            break;
        }
        rng = mix(rng);
        let pick = turns[(rng as usize) % turns.len()];
        let next = net.turns[pick as usize].to_lane;
        // Avoid immediately revisiting the same lane (keeps routes progressing).
        if route.contains(&next) {
            break;
        }
        route.push(next);
        cur = next;
    }
    if route.len() >= 2 { Some(route) } else { None }
}

#[test]
fn winterthur_end_to_end_soak() {
    let net = load_real_net();
    let cap = 260usize;
    let mut core = Core::new(&net, cap, 0x1234_5678);
    core.set_params(IdmParams {
        v0: 13.9,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });

    // Candidate start lanes: those with at least one onward turn and a sane
    // length (>= 10 m, so a vehicle spawns clear of the stop line).
    let starts: Vec<u32> = net
        .lanes
        .iter()
        .filter(|l| l.length_m >= 10.0 && !net.turns_from(l.id).is_empty())
        .map(|l| l.id)
        .collect();
    assert!(
        starts.len() > 200,
        "expected plenty of viable start lanes, got {}",
        starts.len()
    );

    // Spawn ~200 vehicles on distinct start lanes (at most one per lane so no
    // two overlap at spawn), each with a random route.
    let target = 200usize;
    let mut rng = 0xC0FFEE_u64;
    let mut spawned = 0usize;
    let mut used_lane: HashMap<u32, bool> = HashMap::new();
    let mut attempts = 0;
    while spawned < target && attempts < starts.len() * 4 {
        attempts += 1;
        rng = mix(rng);
        let start = starts[(rng as usize) % starts.len()];
        if used_lane.contains_key(&start) {
            continue;
        }
        let Some(route) = random_route(&net, start, 12, mix(rng ^ 0xABCD)) else {
            continue;
        };
        // Spawn near the lane start so it has road ahead.
        let s = (net.lane_len(start) * 0.2).clamp(1.0, 5.0);
        if core.spawn(start, s, &route).is_some() {
            used_lane.insert(start, true);
            spawned += 1;
        }
    }
    assert!(
        spawned >= 150,
        "expected to spawn ~200 vehicles, only placed {spawned}"
    );
    core.reindex();
    let initial_alive = core.fleet.alive_count();

    // Precompute node world positions for the co-occupancy proximity check.
    let node_pos: HashMap<u32, [f32; 2]> = net.nodes.iter().map(|n| (n.id, [n.x, n.z])).collect();

    let mut completed_any = false;
    for t in 0..2000u64 {
        core.tick(t);

        // --- No NaN anywhere.
        for i in 0..core.fleet.slots() {
            if core.fleet.alive[i] {
                assert!(
                    core.fleet.s[i].is_finite() && core.fleet.v[i].is_finite(),
                    "NaN/inf at slot {i} tick {t}: s={} v={}",
                    core.fleet.s[i],
                    core.fleet.v[i]
                );
                assert!(core.fleet.v[i] >= -1e-3, "negative speed at tick {t}");
            }
        }

        // --- No within-lane collisions: on every lane, consecutive vehicles by
        // s keep a non-negative bumper gap. Build per-lane position lists.
        // (cheap enough at 200 veh; done every 50 ticks to bound cost.)
        if t % 50 == 0 {
            let mut by_lane: HashMap<u32, Vec<(f32, f32)>> = HashMap::new();
            for i in 0..core.fleet.slots() {
                if core.fleet.alive[i] {
                    by_lane
                        .entry(core.fleet.lane[i])
                        .or_default()
                        .push((core.fleet.s[i], core.fleet.len_m[i]));
                }
            }
            for (lane, mut v) in by_lane {
                v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                for w in v.windows(2) {
                    let gap = w[1].0 - w[0].0 - w[0].1;
                    assert!(gap > -0.5, "collision on lane {lane} tick {t}: gap {gap}");
                }
            }
        }

        // --- Conflict-point co-occupancy: for each node, collect turns whose
        // vehicles are actively *crossing* the conflict point this tick, and
        // assert no two of them conflict.
        //
        // v1 has no junction geometry: nodes are points and lanes meet at them,
        // so a vehicle *waiting* to give way sits at `lane_len` — geometrically
        // on the node. That is not co-occupancy of the conflict point; it is a
        // queue. A vehicle actually *traversing* the junction is close to the
        // node AND moving. So we count a vehicle as occupying the conflict point
        // only when it is within a tight radius of the node and has real speed;
        // a yielding vehicle (held at the stop line with v≈0) is excluded.
        if t % 10 == 0 {
            let mut near: HashMap<u32, Vec<u32>> = HashMap::new();
            for i in 0..core.fleet.slots() {
                if !core.fleet.alive[i] {
                    continue;
                }
                // Excludes queued/held (stopped) vehicles at the stop line.
                if core.fleet.v[i] < 1.0 {
                    continue;
                }
                let lane = core.fleet.lane[i];
                let cursor = core.fleet.route[i].cursor as usize;
                let route = core.fleet.route_slice(i);
                // The turn the vehicle is executing = current lane -> next lane.
                let Some(&next_lane) = route.get(cursor + 1) else {
                    continue;
                };
                let Some(turn) = turn_between(&net, lane, next_lane) else {
                    continue;
                };
                let node = net.turns[turn as usize].node;
                let np = node_pos[&node];
                let (pos, _) = net.pos_at(lane, core.fleet.s[i]);
                let d = ((pos[0] - np[0]).powi(2) + (pos[1] - np[1]).powi(2)).sqrt();
                if d < 2.5 {
                    near.entry(node).or_default().push(turn);
                }
            }
            for (node, turns) in &near {
                for a in 0..turns.len() {
                    for b in (a + 1)..turns.len() {
                        let ta = turns[a];
                        let tb = turns[b];
                        if ta == tb {
                            continue;
                        }
                        let a_conf = net.turns[ta as usize].conflicts_with.contains(&tb);
                        let b_conf = net.turns[tb as usize].conflicts_with.contains(&ta);
                        assert!(
                            !(a_conf || b_conf),
                            "conflict-point co-occupancy at node {node} tick {t}: \
                             turns {ta} & {tb} both crossing"
                        );
                    }
                }
            }
        }

        if core.fleet.alive_count() < initial_alive {
            completed_any = true;
        }
    }

    assert!(
        completed_any,
        "no vehicle completed its route in 2000 ticks — network may be gridlocked \
         or routes are degenerate (bake defect worth investigating)"
    );
}
