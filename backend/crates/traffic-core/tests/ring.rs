//! Closed-ring integration test for the `traffic-core` kernel.
//!
//! A synthetic 1000 m single-lane loop (a 250 m × 4-side square) with 40
//! vehicles seeded uniformly. Over 3000 ticks we assert the three headline
//! properties of a correct IDM ring:
//!  1. **No collisions** — the minimum bumper-to-bumper gap stays positive on
//!     every tick.
//!  2. **Stop-and-go emergence** — homogeneous traffic on a ring is linearly
//!     unstable above a critical density; congestion waves form. We assert the
//!     speed field develops real variance (stddev > 1 m/s) while the mean speed
//!     is well below free-flow (< 0.8·v0).
//!  3. **Determinism** — same seed twice → identical `state_hash`, and
//!     identical across rayon thread counts (1 vs 4).

use rayon::ThreadPoolBuilder;
use traffic_core::{Core, IdmParams};
use traffic_net::TrafficNet;

const RING_LEN: f32 = 1000.0;
const SIDE: f32 = 250.0;
const N_VEH: usize = 40;
const TICKS: u64 = 3000;

/// Build a 1000 m closed single-lane ring as a validated `TrafficNet`.
/// Square loop: node 0 (0,0) -> 1 (250,0) -> 2 (250,250) -> 3 (0,250) -> 0.
/// Edge/lane/turn ids are dense 0..4. Each corner is an `uncontrolled` node
/// with exactly one covering turn (satisfies validate's turn-coverage rule).
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
        // Turn at the arrival node `to`: from this lane onto the next lane.
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

/// Map a global arc position `s_global` in `[0, RING_LEN)` to `(lane, s_local)`.
fn global_to_lane(s_global: f32) -> (u32, f32) {
    let lane = (s_global / SIDE) as u32 % 4;
    let s_local = s_global - lane as f32 * SIDE;
    (lane, s_local)
}

/// A route that starts the cursor at `lane` (rotation of RING_ROUTE) so
/// `route[0] == lane`, as `spawn` requires.
fn route_from(lane: u32) -> Vec<u32> {
    (0..4).map(|k| (lane + k) % 4).collect()
}

/// Build a ring Core with `N_VEH` vehicles seeded uniformly around the loop.
fn seeded_core(seed: u64) -> Core {
    let net = ring_net();
    let mut core = Core::new(&net, N_VEH + 4, seed);
    // Stop-and-go calibration (Treiber & Kesting 2013, congested-branch
    // parameters): a sluggish acceleration `a` with a desired speed `v0` well
    // above the density-limited equilibrium speed pushes the 40-veh/1000 m flow
    // deep into the string-unstable regime, where a perturbation grows into a
    // travelling congestion wave with vehicles fully stopping and re-launching.
    // The default (highway) params are string-stable at this density and would
    // just relax to uniform flow.
    core.set_params(IdmParams {
        v0: 20.0,
        t_headway: 1.5,
        a_max: 0.3,
        b_comf: 3.0,
        s0: 2.0,
    });
    for i in 0..N_VEH {
        // Uniform base spacing plus a small sinusoidal position perturbation to
        // nucleate a density wave immediately (as in ring experiments where
        // cars start slightly bunched); the instability then amplifies it.
        let base = (i as f32) * (RING_LEN / N_VEH as f32);
        let perturbed = (base + 4.0 * (i as f32 * std::f32::consts::TAU / N_VEH as f32).sin())
            .rem_euclid(RING_LEN);
        let (lane, s_local) = global_to_lane(perturbed);
        core.spawn(lane, s_local, 0, &route_from(lane))
            .expect("spawn within capacity");
    }
    core.reindex();
    core
}

/// Minimum bumper-to-bumper gap across all vehicles on the ring, computed from
/// global arc positions (independent of the kernel's internal leader lookup).
fn min_gap(core: &Core) -> f32 {
    let mut positions: Vec<f32> = Vec::new();
    for i in 0..core.fleet.slots() {
        if core.fleet.alive[i] {
            let g = core.fleet.lane[i] as f32 * SIDE + core.fleet.s[i];
            positions.push(g);
        }
    }
    positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = positions.len();
    let mut min = f32::INFINITY;
    for k in 0..n {
        let cur = positions[k];
        let next = if k + 1 < n {
            positions[k + 1]
        } else {
            positions[0] + RING_LEN // wrap
        };
        let gap = (next - cur) - 4.5; // minus vehicle length
        min = min.min(gap);
    }
    min
}

fn mean_speed(core: &Core) -> f32 {
    let mut sum = 0.0;
    let mut n = 0;
    for i in 0..core.fleet.slots() {
        if core.fleet.alive[i] {
            sum += core.fleet.v[i];
            n += 1;
        }
    }
    sum / n as f32
}

#[test]
fn ring_no_collisions_and_stop_and_go() {
    let mut core = seeded_core(0xABCD);

    let mut speed_samples: Vec<f32> = Vec::new(); // per-vehicle speeds, last 500 ticks
    let mut mean_samples: Vec<f32> = Vec::new();

    for t in 0..TICKS {
        core.tick(t);

        let g = min_gap(&core);
        assert!(g > 0.0, "collision at tick {t}: min gap {g}");

        if t >= TICKS - 500 {
            for i in 0..core.fleet.slots() {
                if core.fleet.alive[i] {
                    speed_samples.push(core.fleet.v[i]);
                }
            }
            mean_samples.push(mean_speed(&core));
        }
    }

    // Stop-and-go: speed stddev over the window > 1 m/s.
    let n = speed_samples.len() as f32;
    let mean = speed_samples.iter().sum::<f32>() / n;
    let var = speed_samples
        .iter()
        .map(|s| (s - mean).powi(2))
        .sum::<f32>()
        / n;
    let stddev = var.sqrt();
    assert!(
        stddev > 1.0,
        "expected stop-and-go stddev > 1.0 m/s, got {stddev}"
    );

    // Congestion: mean speed well below free-flow.
    let overall_mean = mean_samples.iter().sum::<f32>() / mean_samples.len() as f32;
    assert!(
        overall_mean < core.v0() * 0.8,
        "expected mean speed < 0.8*v0 ({}), got {overall_mean}",
        core.v0() * 0.8
    );
}

/// Build a ring Core with a CLASS-MIXED fleet (car/van/truck round-robin) on
/// the per-class default calibrations (no uniform override).
fn seeded_mixed_core(seed: u64) -> Core {
    const N_MIXED: usize = 30;
    let net = ring_net();
    let mut core = Core::new(&net, N_MIXED + 4, seed);
    for i in 0..N_MIXED {
        let base = (i as f32) * (RING_LEN / N_MIXED as f32);
        let (lane, s_local) = global_to_lane(base);
        core.spawn(lane, s_local, (i % 3) as u8, &route_from(lane))
            .expect("spawn within capacity");
    }
    core.reindex();
    core
}

/// Class-aware minimum bumper-to-bumper gap: each pair's gap subtracts the
/// LEADER's real length (12 m truck ≠ 4.5 m car).
fn min_gap_mixed(core: &Core) -> f32 {
    let mut vehicles: Vec<(f32, f32)> = Vec::new(); // (global pos, len)
    for i in 0..core.fleet.slots() {
        if core.fleet.alive[i] {
            let g = core.fleet.lane[i] as f32 * SIDE + core.fleet.s[i];
            vehicles.push((g, core.fleet.len_m[i]));
        }
    }
    vehicles.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let n = vehicles.len();
    let mut min = f32::INFINITY;
    for k in 0..n {
        let (cur, _) = vehicles[k];
        let (lead_pos, lead_len) = if k + 1 < n {
            vehicles[k + 1]
        } else {
            (vehicles[0].0 + RING_LEN, vehicles[0].1)
        };
        min = min.min(lead_pos - cur - lead_len);
    }
    min
}

/// Heterogeneous fleet: no collisions, thread-count-invariant hash, and the
/// IDM class calibration is really wired through — a truck's mean equilibrium
/// gap to its leader exceeds a car's (larger `s0` + longer `T` headway).
#[test]
fn ring_mixed_classes_no_collisions_and_truck_headway() {
    let mut core = seeded_mixed_core(0xC1A55);

    // Per-follower-class gap accumulators over the settled window.
    let mut gap_sum = [0.0f64; 3];
    let mut gap_n = [0u64; 3];

    for t in 0..TICKS {
        core.tick(t);
        let g = min_gap_mixed(&core);
        assert!(g > 0.0, "collision at tick {t}: min gap {g}");

        if t >= TICKS - 300 {
            // Ascending global order; each vehicle's leader is the next one.
            let mut vehicles: Vec<(f32, f32, u8)> = Vec::new();
            for i in 0..core.fleet.slots() {
                if core.fleet.alive[i] {
                    vehicles.push((
                        core.fleet.lane[i] as f32 * SIDE + core.fleet.s[i],
                        core.fleet.len_m[i],
                        core.fleet.class[i],
                    ));
                }
            }
            vehicles.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let n = vehicles.len();
            for k in 0..n {
                let (pos, _, class) = vehicles[k];
                let (lead_pos, lead_len, _) = if k + 1 < n {
                    vehicles[k + 1]
                } else {
                    (vehicles[0].0 + RING_LEN, vehicles[0].1, 0)
                };
                gap_sum[class as usize] += f64::from(lead_pos - pos - lead_len);
                gap_n[class as usize] += 1;
            }
        }
    }

    let mean_gap = |c: usize| (gap_sum[c] / gap_n[c] as f64) as f32;
    let car = mean_gap(0);
    let truck = mean_gap(2);
    assert!(
        truck > car + 1.0,
        "truck equilibrium gap must exceed car gap (truck {truck}, car {car})"
    );

    // Determinism with a mixed fleet across rayon thread counts.
    let run = |threads: usize| -> u64 {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut core = seeded_mixed_core(0xC1A55);
            for t in 0..500 {
                core.tick(t);
            }
            core.state_hash()
        })
    };
    assert_eq!(
        run(1),
        run(4),
        "mixed-class fleet must stay thread-count invariant"
    );
}

#[test]
fn ring_deterministic_same_seed() {
    let mut a = seeded_core(0x1234);
    let mut b = seeded_core(0x1234);
    for t in 0..TICKS {
        a.tick(t);
        b.tick(t);
    }
    assert_eq!(a.state_hash(), b.state_hash(), "same seed must match");
}

#[test]
fn ring_deterministic_across_thread_counts() {
    let run = |threads: usize| -> u64 {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut core = seeded_core(0x5678);
            for t in 0..TICKS {
                core.tick(t);
            }
            core.state_hash()
        })
    };
    let h1 = run(1);
    let h4 = run(4);
    assert_eq!(h1, h4, "thread count must not affect state hash");
}
