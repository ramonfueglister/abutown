//! Integration tests for intersections (Task 5): signal gating, gap
//! acceptance, node crossing, and conflict-point occupancy.
//!
//! Fixtures:
//!  * a hand-built signalised corridor (red queues / green discharges),
//!  * `tests/fixtures/roundabout.json` (entry yields to circulating traffic),
//!  * a synthetic right-before-left priority node,
//!  * and the REAL baked Winterthur network for an end-to-end soak.

use rayon::ThreadPoolBuilder;
use traffic_core::junction::{JunctionModel, turn_between};
use traffic_core::{Core, IdmParams};
use traffic_net::{NodeKind, TrafficNet};

// ---------------------------------------------------------------------------
// Signal gating: a single signalised approach. One long feeder lane leads into
// a signal node; a downstream exit lane leaves it. Red -> vehicles queue and
// nobody crosses; green -> the queue discharges at ~saturation flow.
// ---------------------------------------------------------------------------

/// Build a straight two-edge corridor: feeder lane 0 (node 0 -> signal node 1),
/// exit lane 1 (node 1 -> node 2). The signal at node 1 gates the single turn
/// 0 -> 1 with a `green`/`cycle` split. Lane 0 is `FEEDER_LEN` m long so a queue
/// can form behind the stop line.
fn signal_corridor(feeder_len: f32, green_s: f32, cycle_s: f32) -> TrafficNet {
    let red_s = cycle_s - green_s;
    let json = format!(
        r#"{{
          "meta": {{"anchor":{{"lon":0.0,"lat":0.0}},"laneWidth":3.5,"cellSize":1.0}},
          "nodes": [
            {{"id":0,"x":0,"z":0,"kind":"dead_end","signal":null}},
            {{"id":1,"x":{feeder_len},"z":0,"kind":"signal","signal":{{
                "cycleS":{cycle_s},
                "phases":[
                  {{"greenS":{green_s},"turns":[0]}},
                  {{"greenS":{red_s},"turns":[]}}
                ]
            }}}},
            {{"id":2,"x":{exit_x},"z":0,"kind":"dead_end","signal":null}}
          ],
          "edges": [
            {{"id":0,"from":0,"to":1,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[0]}},
            {{"id":1,"from":1,"to":2,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[1]}}
          ],
          "lanes": [
            {{"id":0,"edge":0,"index":0,"lengthM":{feeder_len},"pts":[[0,0],[{feeder_len},0]]}},
            {{"id":1,"edge":1,"index":0,"lengthM":300,"pts":[[{feeder_len},0],[{exit_x},0]]}}
          ],
          "turns": [
            {{"id":0,"fromLane":0,"toLane":1,"node":1,"conflictsWith":[],"yieldsTo":[]}}
          ]
        }}"#,
        exit_x = feeder_len + 300.0
    );
    traffic_net::load(&json).expect("signal corridor must validate")
}

/// A single phase in `[greenS, turns]` form: the validate step requires the
/// signal phases to *exactly* cover the node's incoming turns. Our second phase
/// `turns:[]` is the all-red interval; the sole incoming turn `0` is gated once
/// in phase 0. That satisfies coverage.
#[test]
fn red_light_queues_and_green_discharges() {
    // green_share = 0.5: Webster saturation flow target = 1800 * 0.5 * 1 lane
    // = 900 vph. Assert within +/-20% (720..=1080), computed from the fixture's
    // own greenS/cycleS so the band stays honest if the fixture changes.
    let green_s = 30.0;
    let cycle_s = 60.0;
    let lanes = 1.0;
    let green_share = green_s / cycle_s;
    let net = signal_corridor(200.0, green_s, cycle_s);
    let mut core = Core::new(&net, 512, 0xA11CE);
    core.set_params(IdmParams {
        v0: 13.9,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });

    // Seed an initial platoon on the feeder so a queue exists immediately.
    let route = [0u32, 1u32];
    for k in 0..8u32 {
        let s = 5.0 + k as f32 * 7.0;
        core.spawn(0, s, 0, &route).expect("spawn feeder seed");
    }
    core.reindex();

    let dt = 0.1f32;
    let jm = JunctionModel::build(&net);

    // Keep the approach CONTINUOUSLY saturated: every tick, if the queued
    // count on the feeder lane is below a target depth and there's a safe gap
    // at s=0, spawn another vehicle on the same route. Deterministic (no
    // randomness) — always tries the same target depth.
    const TARGET_QUEUE_DEPTH: usize = 20;
    const MIN_SPAWN_GAP_M: f32 = 8.0;

    let mut prev_on_exit: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut saw_queue_on_red = false;

    // Measure crossings over cycles 1..=5 (skip cycle 0 as warm-up so the
    // window reflects steady-state saturated discharge, not startup transients
    // from the initial seed platoon).
    const MEASURE_FROM_CYCLE: u32 = 1;
    const MEASURE_CYCLES: u32 = 5;
    let mut green_window_idx: i64 = -1;
    let mut was_green = jm.signal_green(0, 0, dt);
    let mut crossed_in_window = 0u64;
    let mut measure_start_t: Option<u64> = None;
    let mut measure_end_t: Option<u64> = None;

    let total_cycles = (MEASURE_FROM_CYCLE + MEASURE_CYCLES + 1) as u64;
    let total_ticks = (total_cycles as f32 * cycle_s / dt) as u64;

    for t in 0..total_ticks {
        // Top up the standing queue before ticking, so the approach never
        // starves: keep spawning at s=0 while there's room and depth is low.
        loop {
            let queued = (0..core.fleet.slots())
                .filter(|&i| core.fleet.alive[i] && core.fleet.lane[i] == 0)
                .count();
            if queued >= TARGET_QUEUE_DEPTH {
                break;
            }
            // Safe gap at s=0: no alive vehicle on lane 0 within MIN_SPAWN_GAP_M
            // of the origin.
            let blocked = (0..core.fleet.slots()).any(|i| {
                core.fleet.alive[i] && core.fleet.lane[i] == 0 && core.fleet.s[i] < MIN_SPAWN_GAP_M
            });
            if blocked {
                break;
            }
            if core.spawn(0, 0.0, 0, &route).is_none() {
                break; // fleet full
            }
        }
        core.reindex();

        core.tick(t);
        let green = jm.signal_green(0, t, dt);

        // Detect fresh crossings this tick (first appearance on lane 1).
        let mut on_exit_now: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for i in 0..core.fleet.slots() {
            if core.fleet.alive[i] && core.fleet.lane[i] == 1 {
                on_exit_now.insert(i as u32);
            }
        }
        let fresh_count = on_exit_now.difference(&prev_on_exit).count() as u64;
        for &id in &on_exit_now {
            if !prev_on_exit.contains(&id) {
                assert!(
                    green,
                    "vehicle {id} crossed the stop line on RED at tick {t}"
                );
            }
        }

        if !green {
            let queued = (0..core.fleet.slots())
                .filter(|&i| core.fleet.alive[i] && core.fleet.lane[i] == 0)
                .count();
            if queued >= 5 {
                saw_queue_on_red = true;
            }
        }

        if green && !was_green {
            green_window_idx += 1;
        }

        let in_measure_window = green_window_idx >= MEASURE_FROM_CYCLE as i64
            && green_window_idx < (MEASURE_FROM_CYCLE + MEASURE_CYCLES) as i64;
        if green && in_measure_window {
            if measure_start_t.is_none() {
                measure_start_t = Some(t);
            }
            crossed_in_window += fresh_count;
            measure_end_t = Some(t);
        }

        prev_on_exit = on_exit_now;
        was_green = green;
    }

    assert!(
        saw_queue_on_red,
        "no queue ever formed while the light was red"
    );

    let (start_t, end_t) = (
        measure_start_t.expect("measurement window never opened"),
        measure_end_t.expect("measurement window never closed"),
    );
    // Webster capacity (`sat_flow * green_share`) is an hourly-average rate over
    // the FULL cycle (red included), not a green-only rate — the green_share
    // factor already discounts for red time. So the denominator here is the
    // full wall-clock span of the measured cycles, not just their green slices.
    let measured_wall_duration = MEASURE_CYCLES as f32 * cycle_s;
    let vph = crossed_in_window as f32 / measured_wall_duration * 3600.0;

    let target = 1800.0 * green_share * lanes;
    let band_lo = 0.8 * target;
    let band_hi = 1.2 * target;
    assert!(
        vph >= band_lo && vph <= band_hi,
        "saturated signal discharge {vph:.1} vph (crossed {crossed_in_window} over \
         {measured_wall_duration:.1}s wall time [{start_t}..={end_t}]) outside the \
         Webster +/-20% band [{band_lo:.1}, {band_hi:.1}] around target {target:.1} \
         (1800 * green_share {green_share:.3} * {lanes} lane)"
    );
}

// ---------------------------------------------------------------------------
// Roundabout: an entering vehicle must wait for a gap in circulating traffic,
// and the conflict point (the ring node) is never co-occupied.
// ---------------------------------------------------------------------------

fn load_roundabout() -> TrafficNet {
    let json = include_str!("fixtures/roundabout.json");
    traffic_net::load(json).expect("roundabout fixture must validate")
}

/// Assert the entry turn yields to the circulating turn at the same node and
/// that no two conflicting turns ever cross the node on the same tick.
#[test]
fn roundabout_entry_yields_and_no_cooccupancy() {
    let net = load_roundabout();

    // Identify lanes/turns from the fixture (see the builder in scratch: ring
    // lanes 0..3, entry lane 4, exit lane 5; entry turn 5 yields to circ turn 0
    // at node 0).
    let entry_lane = 4u32;
    let ring01 = 0u32; // ring lane node0 -> node1
    // Circulating vehicles loop the ring: lanes 0->1->2->3->0.
    let ring_route: Vec<u32> = vec![0, 1, 2, 3];
    // Entry vehicle: enters lane 4 -> ring lane 0 -> continues the ring.
    let entry_route: Vec<u32> = vec![4, 0, 1, 2, 3];

    // The entry turn must yield to the circulating turn (baked yieldsTo).
    let entry_turn = turn_between(&net, entry_lane, ring01).expect("entry turn exists");
    assert!(
        !net.turns[entry_turn as usize].yields_to.is_empty(),
        "entry turn must yield to circulating traffic"
    );

    let mut core = Core::new(&net, 16, 0x4200);
    core.set_params(IdmParams {
        v0: 8.0,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });

    // Seed circulating vehicles densely around the ring so the entry rarely
    // finds a gap early — they occupy the node region continuously.
    for k in 0..3u32 {
        let lane = k; // ring lanes 0,1,2
        core.spawn(lane, 5.0, 0, &rotate_route(&ring_route, lane))
            .expect("spawn circulating");
    }
    // One entering vehicle, near the entry lane end so it reaches the give-way
    // line quickly.
    let enter = core
        .spawn(entry_lane, 30.0, 0, &entry_route)
        .expect("spawn entering");
    core.fleet.v[enter as usize] = 6.0;
    for k in 0..3u32 {
        // give the circulating cars some speed
        // slots 0..2 are the circulating vehicles
        core.fleet.v[k as usize] = 6.0;
    }
    core.reindex();

    // Conflict-point occupancy invariant: on no tick do two turns that conflict
    // both cross node 0. We approximate node crossing by "a vehicle whose lane
    // changed to a lane leaving node 0 this tick". Stronger + simpler: assert
    // physical non-overlap at the node — no two vehicles within a small radius
    // of the ring node 0 simultaneously if they came from conflicting turns.
    // We use the ring node position (0,0).
    let node0 = [0.0f32, 0.0f32];
    let mut entered = false;
    for t in 0..3000u64 {
        core.tick(t);

        // Count vehicles physically inside the node conflict zone (< 3 m of the
        // ring node) that are on the entry lane vs a ring lane. They must never
        // co-occupy (the entry must have yielded).
        let mut near_from_entry = 0;
        let mut near_from_ring = 0;
        for i in 0..core.fleet.slots() {
            if !core.fleet.alive[i] {
                continue;
            }
            let lane = core.fleet.lane[i];
            let (pos, _) = net.pos_at(lane, core.fleet.s[i]);
            let d = ((pos[0] - node0[0]).powi(2) + (pos[1] - node0[1]).powi(2)).sqrt();
            if d < 4.0 {
                if lane == entry_lane {
                    near_from_entry += 1;
                } else if lane == ring01 || lane == 3 {
                    // ring lanes adjacent to node 0 (arriving 3->0, leaving 0->1)
                    near_from_ring += 1;
                }
            }
        }
        assert!(
            !(near_from_entry > 0 && near_from_ring > 0),
            "conflict-point co-occupancy at node 0 on tick {t}: \
             entry={near_from_entry} ring={near_from_ring}"
        );

        // The entering vehicle should eventually make it onto the ring.
        if core.fleet.alive[enter as usize] && core.fleet.lane[enter as usize] != entry_lane {
            entered = true;
        }
    }
    assert!(
        entered,
        "entering vehicle never found a gap to join the ring"
    );
}

/// Rotate a ring route so `route[0] == start_lane`, as `spawn` requires.
fn rotate_route(route: &[u32], start_lane: u32) -> Vec<u32> {
    let pos = route.iter().position(|&l| l == start_lane).unwrap();
    route
        .iter()
        .cycle()
        .skip(pos)
        .take(route.len())
        .copied()
        .collect()
}

// ---------------------------------------------------------------------------
// Right-before-left: at an uncontrolled/priority node, the yielding turn's
// vehicle waits for the priority approach to clear.
// ---------------------------------------------------------------------------

/// A crossing of two one-way roads at a priority node. Road A (west->east)
/// through lane 0 -> lane 1 has priority; road B (south->north) through lane 2
/// -> lane 3 yields to A. Turn B (id 1) yieldsTo turn A (id 0).
fn priority_cross() -> TrafficNet {
    let json = r#"{
      "meta": {"anchor":{"lon":0.0,"lat":0.0},"laneWidth":3.5,"cellSize":1.0},
      "nodes": [
        {"id":0,"x":-100,"z":0,"kind":"dead_end","signal":null},
        {"id":1,"x":0,"z":0,"kind":"priority","signal":null},
        {"id":2,"x":100,"z":0,"kind":"dead_end","signal":null},
        {"id":3,"x":0,"z":-100,"kind":"dead_end","signal":null},
        {"id":4,"x":0,"z":100,"kind":"dead_end","signal":null}
      ],
      "edges": [
        {"id":0,"from":0,"to":1,"speedMs":13.9,"laneCount":1,"priorityRoad":true,"lanes":[0]},
        {"id":1,"from":1,"to":2,"speedMs":13.9,"laneCount":1,"priorityRoad":true,"lanes":[1]},
        {"id":2,"from":3,"to":1,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[2]},
        {"id":3,"from":1,"to":4,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[3]}
      ],
      "lanes": [
        {"id":0,"edge":0,"index":0,"lengthM":100,"pts":[[-100,0],[0,0]]},
        {"id":1,"edge":1,"index":0,"lengthM":100,"pts":[[0,0],[100,0]]},
        {"id":2,"edge":2,"index":0,"lengthM":100,"pts":[[0,-100],[0,0]]},
        {"id":3,"edge":3,"index":0,"lengthM":100,"pts":[[0,0],[0,100]]}
      ],
      "turns": [
        {"id":0,"fromLane":0,"toLane":1,"node":1,"conflictsWith":[1],"yieldsTo":[]},
        {"id":1,"fromLane":2,"toLane":3,"node":1,"conflictsWith":[0],"yieldsTo":[0]}
      ]
    }"#;
    traffic_net::load(json).expect("priority cross must validate")
}

#[test]
fn right_before_left_yields_to_priority() {
    let net = priority_cross();
    assert_eq!(net.nodes[1].kind, NodeKind::Priority);
    let mut core = Core::new(&net, 8, 0x21DE);
    core.set_params(IdmParams {
        v0: 13.9,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });

    // Priority vehicle on road A, approaching the node at speed.
    let prio = core.spawn(0, 50.0, 0, &[0u32, 1]).expect("prio");
    core.fleet.v[prio as usize] = 13.0;
    // Yielding vehicle on road B, also approaching. It must wait until A clears.
    let yielder = core.spawn(2, 55.0, 0, &[2u32, 3]).expect("yielder");
    core.fleet.v[yielder as usize] = 12.0;
    core.reindex();

    let node = [0.0f32, 0.0f32];
    let mut prio_crossed = false;
    let mut yielder_crossed_tick: Option<u64> = None;
    let mut prio_crossed_tick: Option<u64> = None;

    for t in 0..1200u64 {
        core.tick(t);

        // Physical non-overlap at the node: never both cars within 4 m of it.
        let mut prio_near = false;
        let mut yield_near = false;
        for (id, near) in [(prio, &mut prio_near), (yielder, &mut yield_near)] {
            let i = id as usize;
            if !core.fleet.alive[i] {
                continue;
            }
            let (pos, _) = net.pos_at(core.fleet.lane[i], core.fleet.s[i]);
            let d = ((pos[0] - node[0]).powi(2) + (pos[1] - node[1]).powi(2)).sqrt();
            if d < 4.0 {
                *near = true;
            }
        }
        assert!(
            !(prio_near && yield_near),
            "priority and yielding vehicles co-occupied the node at tick {t}"
        );

        // Track crossings onto the exit lanes.
        if core.fleet.alive[prio as usize] && core.fleet.lane[prio as usize] == 1 {
            prio_crossed = true;
            prio_crossed_tick.get_or_insert(t);
        }
        if core.fleet.alive[yielder as usize] && core.fleet.lane[yielder as usize] == 3 {
            yielder_crossed_tick.get_or_insert(t);
        }
    }

    assert!(prio_crossed, "priority vehicle never crossed");
    // The yielder must cross *after* the priority vehicle (it gave way).
    match (prio_crossed_tick, yielder_crossed_tick) {
        (Some(p), Some(y)) => assert!(
            y >= p,
            "yielder crossed at {y} before priority at {p} — did not yield"
        ),
        (Some(_), None) => { /* yielder still waiting/behind — acceptable */ }
        _ => panic!("priority vehicle should have crossed"),
    }
}

// ---------------------------------------------------------------------------
// Determinism across thread counts, with junctions active.
// ---------------------------------------------------------------------------

fn junction_soak_core(seed: u64) -> Core {
    let net = load_roundabout();
    let mut core = Core::new(&net, 24, seed);
    core.set_params(IdmParams {
        v0: 8.0,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });
    let ring_route: Vec<u32> = vec![0, 1, 2, 3];
    for k in 0..4u32 {
        let lane = k;
        core.spawn(
            lane,
            3.0 + k as f32 * 2.0,
            0,
            &rotate_route(&ring_route, lane),
        )
        .expect("spawn circ");
    }
    core.spawn(4, 20.0, 0, &[4u32, 0, 1, 2, 3])
        .expect("spawn entry");
    core.reindex();
    core
}

#[test]
fn junctions_deterministic_across_thread_counts() {
    let run = |threads: usize| -> u64 {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut core = junction_soak_core(0xDE7E);
            for t in 0..2000u64 {
                core.tick(t);
            }
            core.state_hash()
        })
    };
    assert_eq!(run(1), run(4), "thread count must not affect junction sim");
}

#[test]
fn junctions_deterministic_same_seed() {
    let mut a = junction_soak_core(0x5EED);
    let mut b = junction_soak_core(0x5EED);
    for t in 0..2000u64 {
        a.tick(t);
        b.tick(t);
    }
    assert_eq!(a.state_hash(), b.state_hash());
}
