//! Integration tests for intersections (Task 5): signal gating, gap
//! acceptance, node crossing, and conflict-point occupancy.
//!
//! Fixtures:
//!  * a hand-built signalised corridor (red queues / green discharges),
//!  * `tests/fixtures/roundabout.json` (entry yields to circulating traffic),
//!  * a synthetic right-before-left priority node,
//!  * and the REAL baked Winterthur network for an end-to-end soak.

use rayon::ThreadPoolBuilder;
use traffic_core::junction::turn_between;
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

/// Actuated semantics (S3): a lone approach's signal IDLES GREEN. The
/// authored all-red filler phase is dropped at build, leaving one demanded
/// phase; with no competing demand the controller never switches away, so
/// the sole turn reads green at (essentially) every tick — the opposite of
/// the fixed-time bake, which held it red for the off-phase share and
/// starved the movement. We assert green >= 99% of ticks over 5 nominal
/// cycles and that a saturated queue keeps discharging (crossings > 0 in
/// every cycle-length window, i.e. never gated to zero).
#[test]
fn actuated_single_approach_idles_green() {
    let net = signal_corridor(200.0, 30.0, 60.0);
    let mut core = Core::new(&net, 512, 0xA11CE);
    core.set_params(IdmParams {
        v0: 13.9,
        t_headway: 1.2,
        a_max: 1.5,
        b_comf: 2.0,
        s0: 2.0,
    });
    let route = [0u32, 1u32];
    for k in 0..8u32 {
        core.spawn(0, 5.0 + k as f32 * 7.0, 0, &route)
            .expect("spawn feeder seed");
    }
    core.reindex();

    const TICKS: u64 = 3000; // 5 * 60 s
    const WINDOW: u64 = 600; // one nominal cycle
    let mut green_ticks = 0u64;
    let mut prev_exit: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut crossed_in_window = 0u64;

    for t in 0..TICKS {
        // Keep the approach saturated.
        loop {
            let queued = (0..core.fleet.slots())
                .filter(|&i| core.fleet.alive[i] && core.fleet.lane[i] == 0)
                .count();
            if queued >= 20 {
                break;
            }
            let clear = (0..core.fleet.slots())
                .all(|i| !core.fleet.alive[i] || core.fleet.lane[i] != 0 || core.fleet.s[i] > 8.0);
            if !clear || core.spawn(0, 0.0, 0, &route).is_none() {
                break;
            }
        }
        core.tick(t);
        if core.signal_green(0, t) {
            green_ticks += 1;
        }
        let exit: std::collections::HashSet<u32> = (0..core.fleet.slots() as u32)
            .filter(|&v| core.fleet.alive[v as usize] && core.fleet.lane[v as usize] == 1)
            .collect();
        crossed_in_window += exit.difference(&prev_exit).count() as u64;
        prev_exit = exit;
        if (t + 1) % WINDOW == 0 {
            assert!(
                crossed_in_window > 0,
                "discharge stalled in cycle window ending at t={t}"
            );
            crossed_in_window = 0;
        }
    }
    let green_share = green_ticks as f32 / TICKS as f32;
    assert!(
        green_share > 0.99,
        "lone approach must idle green (>=99% of ticks), got {:.1}%",
        green_share * 100.0
    );
}

/// Actuated cross demand (S3): a two-phase signal with the MAIN road kept
/// continuously saturated must still serve a side-road platoon within the
/// min-green + max-green bound once it arrives, then hand green back to the
/// main road (which resumes discharging). Proves demand-adaptive switching
/// in both directions.
#[test]
fn actuated_cross_demand_is_served_and_main_resumes() {
    let json = r#"{
      "meta": {"anchor":{"lon":0.0,"lat":0.0},"laneWidth":3.5,"cellSize":1.0},
      "nodes": [
        {"id":0,"x":0,"z":0,"kind":"dead_end","signal":null},
        {"id":1,"x":400,"z":0,"kind":"dead_end","signal":null},
        {"id":2,"x":200,"z":-200,"kind":"dead_end","signal":null},
        {"id":3,"x":200,"z":200,"kind":"dead_end","signal":null},
        {"id":4,"x":200,"z":0,"kind":"signal","signal":{
            "cycleS":60,
            "phases":[
              {"greenS":30,"turns":[0]},
              {"greenS":30,"turns":[1]}
            ]
        }}
      ],
      "edges": [
        {"id":0,"from":0,"to":4,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[0]},
        {"id":1,"from":4,"to":1,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[1]},
        {"id":2,"from":2,"to":4,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[2]},
        {"id":3,"from":4,"to":3,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[3]}
      ],
      "lanes": [
        {"id":0,"edge":0,"index":0,"lengthM":200,"pts":[[0,0],[200,0]]},
        {"id":1,"edge":1,"index":0,"lengthM":200,"pts":[[200,0],[400,0]]},
        {"id":2,"edge":2,"index":0,"lengthM":200,"pts":[[200,-200],[200,0]]},
        {"id":3,"edge":3,"index":0,"lengthM":200,"pts":[[200,0],[200,200]]}
      ],
      "turns": [
        {"id":0,"fromLane":0,"toLane":1,"node":4,"conflictsWith":[1],"yieldsTo":[]},
        {"id":1,"fromLane":2,"toLane":3,"node":4,"conflictsWith":[0],"yieldsTo":[]}
      ]
    }"#;
    let net = traffic_net::load(json).expect("actuated X fixture must validate");
    let mut core = Core::new(&net, 64, 0x516);
    let main_route = [0u32, 1u32];
    for k in 0..10u32 {
        core.spawn(0, 190.0 - k as f32 * 8.0, 0, &main_route)
            .expect("main seed");
    }
    core.reindex();

    const ARRIVE: u64 = 200;
    let mut side: Vec<u32> = Vec::new();
    let mut side_served_at: Option<u64> = None;
    let mut main_after_side = 0u64;
    let mut prev_main_exit: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for t in 0..4000 {
        // Keep the MAIN road continuously fed so it always has demand.
        loop {
            let queued = (0..core.fleet.slots())
                .filter(|&i| core.fleet.alive[i] && core.fleet.lane[i] == 0)
                .count();
            if queued >= 12 {
                break;
            }
            let clear = (0..core.fleet.slots())
                .all(|i| !core.fleet.alive[i] || core.fleet.lane[i] != 0 || core.fleet.s[i] > 8.0);
            if !clear || core.spawn(0, 0.0, 0, &main_route).is_none() {
                break;
            }
        }
        if t == ARRIVE {
            for k in 0..3u32 {
                side.push(
                    core.spawn(2, 150.0 - k as f32 * 10.0, 0, &[2u32, 3])
                        .expect("side spawn"),
                );
            }
            core.reindex();
        }
        core.tick(t);

        if t > ARRIVE && side_served_at.is_none() {
            let all_over = side
                .iter()
                .all(|&v| !core.fleet.alive[v as usize] || core.fleet.lane[v as usize] == 3);
            if all_over {
                side_served_at = Some(t);
            }
        }
        let main_exit: std::collections::HashSet<u32> = (0..core.fleet.slots() as u32)
            .filter(|&v| core.fleet.alive[v as usize] && core.fleet.lane[v as usize] == 1)
            .collect();
        if side_served_at.is_some_and(|served| t > served) {
            main_after_side += main_exit.difference(&prev_main_exit).count() as u64;
        }
        prev_main_exit = main_exit;
    }

    let served = side_served_at.expect("side road never got green");
    assert!(
        served < ARRIVE + 900,
        "side demand served too late: tick {served} (arrival {ARRIVE})"
    );
    assert!(
        main_after_side >= 3,
        "main road never resumed after the side phase ({main_after_side} crossings)"
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

// ---------------------------------------------------------------------------
// Mutual-yield deadlock regression (S2 calibration finding): at an
// UNCONTROLLED crossing whose two turns yield to EACH OTHER (right-before-
// left is inherently cyclic), two vehicles arriving together must not wait
// on each other forever. Gap acceptance is defined against the APPROACHING
// priority stream; a vehicle standing at its own stop line must not veto —
// the physical conflict point is arbitrated by phase-2 occupancy (claims +
// clearance). Before the fix this scenario gridlocked permanently and, on
// the real net, snowballed into 26.6k of 26.7k vehicles stopped by world
// midnight.
// ---------------------------------------------------------------------------

/// An uncontrolled X: west feeder (lane 0) -> east exit (lane 1), north
/// feeder (lane 2) -> south exit (lane 3), crossing at node 4. The two turns
/// mutually conflict AND mutually yield.
fn mutual_yield_cross() -> TrafficNet {
    let json = r#"{
      "meta": {"anchor":{"lon":0.0,"lat":0.0},"laneWidth":3.5,"cellSize":1.0},
      "nodes": [
        {"id":0,"x":0,"z":0,"kind":"dead_end","signal":null},
        {"id":1,"x":200,"z":0,"kind":"dead_end","signal":null},
        {"id":2,"x":100,"z":-100,"kind":"dead_end","signal":null},
        {"id":3,"x":100,"z":100,"kind":"dead_end","signal":null},
        {"id":4,"x":100,"z":0,"kind":"uncontrolled","signal":null}
      ],
      "edges": [
        {"id":0,"from":0,"to":4,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[0]},
        {"id":1,"from":4,"to":1,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[1]},
        {"id":2,"from":2,"to":4,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[2]},
        {"id":3,"from":4,"to":3,"speedMs":13.9,"laneCount":1,"priorityRoad":false,"lanes":[3]}
      ],
      "lanes": [
        {"id":0,"edge":0,"index":0,"lengthM":100,"pts":[[0,0],[100,0]]},
        {"id":1,"edge":1,"index":0,"lengthM":100,"pts":[[100,0],[200,0]]},
        {"id":2,"edge":2,"index":0,"lengthM":100,"pts":[[100,-100],[100,0]]},
        {"id":3,"edge":3,"index":0,"lengthM":100,"pts":[[100,0],[100,100]]}
      ],
      "turns": [
        {"id":0,"fromLane":0,"toLane":1,"node":4,"conflictsWith":[1],"yieldsTo":[1]},
        {"id":1,"fromLane":2,"toLane":3,"node":4,"conflictsWith":[0],"yieldsTo":[0]}
      ]
    }"#;
    traffic_net::load(json).expect("mutual-yield cross must validate")
}

#[test]
fn mutual_yield_arrivals_do_not_deadlock() {
    let net = mutual_yield_cross();
    let mut core = Core::new(&net, 8, 0xDEAD);
    // Same distance, same class: both hit the stop line on the same tick —
    // the worst-case symmetric arrival.
    core.spawn(0, 50.0, 0, &[0u32, 1]).expect("spawn west");
    core.spawn(2, 50.0, 0, &[2u32, 3]).expect("spawn north");
    core.reindex();

    // 100 s of sim time is orders of magnitude beyond one crossing +
    // clearance; both open routes must have completed (despawned).
    for t in 0..1000 {
        core.tick(t);
        if core.fleet.alive_count() == 0 {
            return;
        }
    }
    panic!(
        "mutual-yield crossing deadlocked: {} vehicle(s) still alive after 100 s",
        core.fleet.alive_count()
    );
}

/// Missed-turn stranding seam: a route whose next lane is NOT reachable from
/// the current lane (no turn 0 -> 3 exists in the mutual-yield cross) drives
/// the vehicle into the no-turn wall, and the kernel reports it via
/// `stranded_last_tick` — alive, stopped, waiting for a shell rescue.
#[test]
fn no_turn_wall_reports_stranded() {
    let net = mutual_yield_cross();
    let mut core = Core::new(&net, 4, 0x57A);
    core.spawn(0, 50.0, 0, &[0u32, 3])
        .expect("spawn walled route");
    core.reindex();

    let mut reported = false;
    for t in 0..600 {
        core.tick(t);
        if core.stranded_last_tick().contains(&0) {
            reported = true;
            break;
        }
    }
    assert!(reported, "walled vehicle never reported stranded");
    assert_eq!(
        core.fleet.alive_count(),
        1,
        "kernel must NOT despawn it itself"
    );
}

/// Strategic mandatory merge (missed-turn fix): on a two-lane feeder where
/// only lane 0 has a turn onto the exit, a vehicle routed on lane 1 must
/// merge to lane 0 inside the urgent zone and complete — before the fix it
/// drove into the no-turn wall and waited forever.
#[test]
fn wrong_lane_merges_before_the_wall_and_completes() {
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
    let net = traffic_net::load(json).expect("two-lane merge fixture must validate");
    let mut core = Core::new(&net, 4, 0x3E);
    core.spawn(1, 20.0, 0, &[1u32, 2])
        .expect("spawn on wrong lane");
    core.reindex();

    for t in 0..800 {
        core.tick(t);
        if core.fleet.alive_count() == 0 {
            return; // merged, crossed, completed
        }
    }
    panic!("vehicle never completed: stuck on the turnless lane");
}
