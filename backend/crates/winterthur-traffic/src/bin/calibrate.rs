//! `calibrate`: headless full-world-day run that counts vehicles ENTERING
//! the mapped count-station edges per world hour and vehicle class — the sim
//! side of the S2 calibration (plan: docs/superpowers/plans/
//! 2026-07-06-traffic-sota-s2-calibration.md, Task 3).
//!
//! Drives the SAME deterministic `build_sim` chain as the server (traffic-
//! only mode, no gateway, no timing): `WallClock::anchored` at a pinned
//! workday midnight, then 144'000 ticks = one full world day (0.6 world-s
//! per tick). Crossing detection runs in the [`SnapshotHook`] seam — the
//! wire must not feed back into the sim, and neither does the calibration
//! counter: it is a pure read of each tick's snapshot.
//!
//! Env (matches the server binary's conventions):
//!   TRAFFICNET_JSON   (default data/winterthur/trafficnet.json)
//!   TRIPS_BIN         (default data/winterthur/trips.bin)
//!   COUNT_STATIONS    (default data/winterthur/count-stations.json)
//!   CALIBRATION_OUT   (default scratch/calibration/simulated-profiles.json)
//!   TRAFFIC_SEED      (default 0)
//!   DEMAND_SCALE      (default 1.0)
//!   CALIBRATE_DATE    (default 2026-07-07, a Tuesday → workday block)
//!
//! Runtime is dominated by the kernel (~2-4k alive vehicles at rush hour);
//! expect minutes in release, far too slow for CI — run locally:
//!   scripts/cargo-serial.sh run --manifest-path backend/Cargo.toml \
//!     --release -p winterthur-traffic --bin calibrate

use chrono::{NaiveDate, NaiveTime};
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use winterthur_traffic::clock::WallClock;
use winterthur_traffic::demand::TripSchedule;
use winterthur_traffic::shell::{SnapshotHook, build_sim};
use winterthur_traffic::spawner::SpawnerCfg;

/// One full world day in sim ticks: 86_400 world-s / (DT · WORLD_TIME_SCALE).
const TICKS_PER_WORLD_DAY: u64 = world_core::clock::WORLD_SECONDS_PER_DAY
    * world_core::TICKS_PER_SECOND
    / world_core::WORLD_TIME_SCALE;

/// World hour of a tick (0..24), mirroring `WorldClock::s_of_world_day`.
fn world_hour(tick: u64) -> usize {
    let world_s = tick * world_core::WORLD_TIME_SCALE / world_core::TICKS_PER_SECOND;
    ((world_s % world_core::clock::WORLD_SECONDS_PER_DAY) / 3600) as usize
}

/// A monitored directed cross-section from count-stations.json.
struct Station {
    anlage_name: String,
    richtung_name: String,
    edge: u32,
}

/// Per-station hourly entering counts, one bucket per kernel class.
type Counts = Vec<[[u64; 3]; 24]>;

fn kind_str(n: &traffic_net::Node) -> &'static str {
    use traffic_net::NodeKind::*;
    match n.kind {
        Signal => "signal",
        Roundabout => "roundabout",
        Priority => "priority",
        Uncontrolled => "uncontrolled",
        Gateway => "gateway",
        DeadEnd => "dead_end",
    }
}

fn env_or(key: &str, dflt: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| dflt.to_string())
}

fn main() -> anyhow::Result<()> {
    let net_path = env_or("TRAFFICNET_JSON", "data/winterthur/trafficnet.json");
    let trips_path = env_or("TRIPS_BIN", "data/winterthur/trips.bin");
    let stations_path = env_or("COUNT_STATIONS", "data/winterthur/count-stations.json");
    let out_path = env_or(
        "CALIBRATION_OUT",
        "scratch/calibration/simulated-profiles.json",
    );
    let seed: u64 = env_or("TRAFFIC_SEED", "0").parse()?;
    let demand_scale: f32 = env_or("DEMAND_SCALE", "1.0").parse()?;
    let date = NaiveDate::parse_from_str(&env_or("CALIBRATE_DATE", "2026-07-07"), "%Y-%m-%d")?;

    let net_json = std::fs::read_to_string(&net_path)?;
    let net = traffic_net::load(&net_json).map_err(|e| anyhow::anyhow!("net load: {e}"))?;
    let trips = TripSchedule::load(std::path::Path::new(&trips_path), net_json.as_bytes())?;

    let stations_doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&stations_path)?)?;
    let stations: Vec<Station> = stations_doc["stations"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("{stations_path}: no stations array"))?
        .iter()
        .map(|s| Station {
            anlage_name: s["anlageName"].as_str().unwrap_or("?").to_string(),
            richtung_name: s["richtungName"].as_str().unwrap_or("?").to_string(),
            edge: s["edge"].as_u64().expect("station edge id") as u32,
        })
        .collect();
    anyhow::ensure!(!stations.is_empty(), "no stations to calibrate against");

    // edge id → station indices watching it (dense LUT over edge ids).
    let max_edge = net.edges.iter().map(|e| e.id).max().unwrap_or(0) as usize;
    let mut watchers: Vec<Vec<usize>> = vec![Vec::new(); max_edge + 1];
    for (i, st) in stations.iter().enumerate() {
        watchers[st.edge as usize].push(i);
    }

    // Midnight anchor on a pinned real date: world second 0, day_kind of
    // `date` (the whole 4 h wall run stays inside that date).
    let clock = WallClock::anchored(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    let (mut world, mut schedule) = build_sim(
        net.clone(),
        seed,
        trips,
        clock,
        SpawnerCfg { demand_scale },
        None,
    );

    // Crossing counter in the snapshot seam: a vehicle is COUNTED when the
    // edge of its current lane differs from its previous tick's edge and the
    // new edge is monitored (slot-reuse safe: freed slots reset to NONE).
    const NONE: u32 = u32::MAX;
    let max_node = net.nodes.iter().map(|n| n.id).max().unwrap_or(0) as usize;
    let edge_from: Arc<Vec<u32>> = {
        let mut v = vec![0u32; max_edge + 1];
        for e in &net.edges {
            v[e.id as usize] = e.from;
        }
        Arc::new(v)
    };
    let state = Arc::new(Mutex::new((
        vec![NONE; 0] as Vec<u32>, // prev edge per slot
        vec![[[0u64; 3]; 24]; stations.len()] as Counts,
        vec![0u64; max_node + 1], // node crossings (vehicle entered an edge FROM this node)
    )));
    let hook_state = Arc::clone(&state);
    let watchers = Arc::new(watchers);
    world.insert_resource(SnapshotHook::new(move |snap| {
        let mut guard = hook_state.lock().expect("hook state poisoned");
        let (prev, counts, node_cross) = &mut *guard;
        let slots = snap.core.fleet.slots();
        prev.resize(slots, NONE);
        let hour = world_hour(snap.tick);
        for slot in 0..slots {
            let cur = match snap.core.vehicle_view(slot as u32) {
                Some(view) => view.edge,
                None => {
                    prev[slot] = NONE;
                    continue;
                }
            };
            if cur != prev[slot] {
                if prev[slot] != NONE {
                    node_cross[edge_from[cur as usize] as usize] += 1;
                }
                if let Some(watching) = watchers.get(cur as usize) {
                    let class = snap.core.fleet.class[slot].min(2) as usize;
                    for &si in watching {
                        counts[si][hour][class] += 1;
                    }
                }
                prev[slot] = cur;
            }
        }
    }));

    eprintln!(
        "calibrate: {} stations, seed={seed}, demand_scale={demand_scale}, date={date} \
         ({TICKS_PER_WORLD_DAY} ticks = 1 world day)",
        stations.len()
    );
    let started = std::time::Instant::now();
    let max_node_id = net.nodes.iter().map(|n| n.id).max().unwrap_or(0) as usize;
    let mut hold_heads_by_node = vec![0u32; max_node_id + 1];

    // Optional single-node micro watch (CALIBRATE_WATCH_NODE): every 100
    // ticks during world 08:00-09:00, log each approach lane's queue, its
    // head's state, and the SPACE on the head's next route lane — separates
    // signal, gap, occupancy and spillback at one junction.
    let watch_node: Option<u32> = std::env::var("CALIBRATE_WATCH_NODE")
        .ok()
        .and_then(|v| v.parse().ok());
    let watch_lanes: Vec<u32> = watch_node
        .map(|node| {
            net.turns
                .iter()
                .filter(|tn| tn.node == node)
                .map(|tn| tn.from_lane)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();

    for t in 0..TICKS_PER_WORLD_DAY {
        schedule.run(&mut world);

        if watch_node.is_some() && (48_000..54_000).contains(&t) && t % 100 == 0 {
            use traffic_core::junction::{JunctionModel, turn_between};
            let core = &world.resource::<winterthur_traffic::shell::CoreRes>().0;
            let jm = JunctionModel::build(&net);
            let mut line = format!("watch t={t}");
            for &lane in &watch_lanes {
                let occ = core.index.on_lane(lane);
                let q = occ
                    .iter()
                    .filter(|&&v| core.fleet.v[v as usize] < 2.0)
                    .count();
                let head = occ.first().map(|&v| v as usize);
                let head_info = match head {
                    None => "-".to_string(),
                    Some(h) => {
                        let dist = net.lanes[lane as usize].length_m - core.fleet.s[h];
                        let cursor = core.fleet.route[h].cursor as usize;
                        let nxt = core.fleet.route_slice(h).get(cursor + 1).copied();
                        let (sig, next_space) =
                            match nxt.and_then(|nl| turn_between(&net, lane, nl)) {
                                Some(turn) => {
                                    let g = jm.signal_green(turn, t, traffic_core::DT);
                                    let nl = nxt.unwrap();
                                    // Space before the rear-most vehicle on next lane.
                                    let space = core
                                        .index
                                        .on_lane(nl)
                                        .last()
                                        .map(|&r| core.fleet.s[r as usize])
                                        .unwrap_or(f32::INFINITY);
                                    (if g { "G" } else { "R" }, space)
                                }
                                None => ("?", f32::NAN),
                            };
                        format!(
                            "d{dist:.1} v{:.1} {sig} nsp{next_space:.0}",
                            core.fleet.v[h]
                        )
                    }
                };
                line.push_str(&format!(" | L{lane}: q{q} [{head_info}]"));
            }
            eprintln!("{line}");
        }
        if t % (TICKS_PER_WORLD_DAY / 24) == 0 {
            let core = &world.resource::<winterthur_traffic::shell::CoreRes>().0;
            let mut alive = 0u32;
            let mut stopped = 0u32;
            let mut v_sum = 0.0f32;
            for slot in 0..core.fleet.slots() {
                if let Some(view) = core.vehicle_view(slot as u32) {
                    alive += 1;
                    v_sum += view.v;
                    if view.v < 0.5 {
                        stopped += 1;
                    }
                }
            }
            eprintln!(
                "  world {:02}:00  alive={alive} stopped={stopped} mean_v={:.1}  ({:.0}s)",
                world_hour(t),
                if alive > 0 { v_sum / alive as f32 } else { 0.0 },
                started.elapsed().as_secs_f32()
            );
            // Chokepoint sampling (daytime hours): queue HEADS — stopped, road
            // clear ahead, pressed against a lane end — attributed to the node
            // of their next turn. Accumulated across hourly snapshots.
            let h = world_hour(t);
            if (7..=20).contains(&h) {
                use traffic_core::junction::turn_between;
                for slot in 0..core.fleet.slots() {
                    let Some(view) = core.vehicle_view(slot as u32) else {
                        continue;
                    };
                    if view.v >= 0.5 {
                        continue;
                    }
                    let lane_len = net.lanes[view.lane as usize].length_m;
                    if lane_len - view.s > 5.0 {
                        continue;
                    }
                    let mut clear = true;
                    for &other in core.index.on_lane(view.lane) {
                        if other != slot as u32 {
                            let so = core.fleet.s[other as usize];
                            if so > view.s && so - view.s < 15.0 {
                                clear = false;
                                break;
                            }
                        }
                    }
                    if !clear {
                        continue;
                    }
                    let cursor = core.fleet.route[slot].cursor as usize;
                    if let Some(&next_lane) = core.fleet.route_slice(slot).get(cursor + 1)
                        && let Some(turn) = turn_between(&net, view.lane, next_lane)
                    {
                        hold_heads_by_node[net.turns[turn as usize].node as usize] += 1;
                    }
                }
            }
        }
    }

    // Night-stall classification at world 03:00 equivalent — but since the
    // loop above already ran to completion we classify at END state instead:
    // for each stopped vehicle, is it (a) queued behind a leader, (b) held at
    // a lane end whose next turn's signal is red, (c) held at a lane end with
    // green/no signal (gap or conflict-point hold, or a NoTurn wall), or
    // (d) stopped mid-lane with clear road (anomaly)?
    {
        use traffic_core::junction::{JunctionModel, turn_between};
        let core = &world.resource::<winterthur_traffic::shell::CoreRes>().0;
        let jm = JunctionModel::build(&net);
        let (mut queued, mut red, mut hold, mut anomaly, mut route_end) =
            (0u32, 0u32, 0u32, 0u32, 0u32);
        let mut hold_samples: Vec<String> = Vec::new();
        for slot in 0..core.fleet.slots() {
            let Some(view) = core.vehicle_view(slot as u32) else {
                continue;
            };
            if view.v >= 0.5 {
                continue;
            }
            // Leader within 15 m ahead on the same lane?
            let mut has_leader = false;
            let occ = core.index.on_lane(view.lane);
            for &other in occ {
                if other == slot as u32 {
                    continue;
                }
                let so = core.fleet.s[other as usize];
                if so > view.s && so - view.s < 15.0 {
                    has_leader = true;
                    break;
                }
            }
            if has_leader {
                queued += 1;
                continue;
            }
            let lane_len = net.lanes[view.lane as usize].length_m;
            let dist_to_end = lane_len - view.s;
            if dist_to_end > 5.0 {
                anomaly += 1;
                continue;
            }
            // At a lane end with clear road: what governs the boundary?
            let cursor = core.fleet.route[slot].cursor as usize;
            let route = core.fleet.route_slice(slot);
            match route.get(cursor + 1) {
                None => route_end += 1,
                Some(&next_lane) => match turn_between(&net, view.lane, next_lane) {
                    None => {
                        hold += 1;
                        if hold_samples.len() < 8 {
                            hold_samples.push(format!(
                                "slot {slot}: NO-TURN wall lane {} -> {next_lane} (edge {})",
                                view.lane, view.edge
                            ));
                        }
                    }
                    Some(turn) => {
                        if !jm.signal_green(turn, TICKS_PER_WORLD_DAY, traffic_core::DT) {
                            red += 1;
                        } else {
                            hold += 1;
                            if hold_samples.len() < 8 {
                                hold_samples.push(format!(
                                    "slot {slot}: HOLD at turn {turn} (lane {} edge {} node {}, yields_to={:?})",
                                    view.lane,
                                    view.edge,
                                    net.turns[turn as usize].node,
                                    net.turns[turn as usize].yields_to
                                ));
                            }
                        }
                    }
                },
            }
        }
        eprintln!(
            "calibrate: stopped classification at end — queued_behind_leader={queued} \
             signal_red={red} boundary_hold={hold} route_end_wait={route_end} mid_lane_anomaly={anomaly}"
        );
        for s in &hold_samples {
            eprintln!("  {s}");
        }
    }

    // Chokepoint ranking: nodes by queue-head holds (hourly 07-20 samples),
    // with their kind and total crossings/day for capacity context.
    {
        let guard = state.lock().expect("hook state poisoned");
        let node_cross = &guard.2;
        let kind_of: std::collections::BTreeMap<u32, &str> =
            net.nodes.iter().map(|n| (n.id, kind_str(n))).collect();
        let mut ranked: Vec<(u32, u32)> = hold_heads_by_node
            .iter()
            .enumerate()
            .filter(|&(_, &c)| c > 0)
            .map(|(n, &c)| (n as u32, c))
            .collect();
        ranked.sort_by_key(|&(n, c)| (std::cmp::Reverse(c), n));
        eprintln!("calibrate: top chokepoint nodes (queue-head holds across 14 hourly samples):");
        for &(node, holds) in ranked.iter().take(15) {
            eprintln!(
                "  node {node} [{}]: holds={holds} crossings/day={}",
                kind_of.get(&node).unwrap_or(&"?"),
                node_cross[node as usize]
            );
        }
    }

    // Gridlock forensics: where do stuck vehicles sit at world midnight?
    {
        let core = &world.resource::<winterthur_traffic::shell::CoreRes>().0;
        let mut by_edge: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        for slot in 0..core.fleet.slots() {
            if let Some(view) = core.vehicle_view(slot as u32)
                && view.v < 0.5
            {
                *by_edge.entry(view.edge).or_insert(0) += 1;
            }
        }
        let mut top: Vec<(u32, u32)> = by_edge.into_iter().collect();
        top.sort_by_key(|&(e, n)| (std::cmp::Reverse(n), e));
        eprintln!("calibrate: top stuck edges at end (edge: stopped vehicles):");
        for (e, n) in top.iter().take(20) {
            eprintln!("  edge {e}: {n}");
        }
    }

    // Spawner outcome ledger: without this, a level gap in the report is
    // ambiguous between "demand too low" and "trips failed to spawn".
    {
        use winterthur_traffic::shell::SpawnerRes;
        let stranded = *world.resource::<winterthur_traffic::shell::StrandedLedger>();
        eprintln!("calibrate: stranded ledger {stranded:?}");
        let counters = world.resource::<SpawnerRes>().0.counters();
        let alive = world
            .resource::<winterthur_traffic::shell::CoreRes>()
            .0
            .fleet
            .alive_count();
        eprintln!(
            "calibrate: spawned={} skipped_no_route={} suppressed={} blocked_entry={} alive_at_end={}",
            counters.spawned,
            counters.skipped_no_route,
            counters.suppressed,
            counters.blocked_entry,
            alive
        );
    }

    let guard = state.lock().expect("hook state poisoned");
    let (_, counts, _) = &*guard;
    let out = serde_json::json!({
        "seed": seed,
        "demandScale": demand_scale,
        "date": date.to_string(),
        "ticks": TICKS_PER_WORLD_DAY,
        "stations": stations.iter().enumerate().map(|(i, st)| serde_json::json!({
            "anlageName": st.anlage_name,
            "richtungName": st.richtung_name,
            "edge": st.edge,
            // Vehicles ENTERING the edge per world hour, by class bucket —
            // the same unit as the observed profiles (vehicles/hour).
            "hours": {
                "car":      (0..24).map(|h| counts[i][h][0]).collect::<Vec<_>>(),
                "delivery": (0..24).map(|h| counts[i][h][1]).collect::<Vec<_>>(),
                "truck":    (0..24).map(|h| counts[i][h][2]).collect::<Vec<_>>(),
            },
        })).collect::<Vec<_>>(),
    });
    if let Some(dir) = std::path::Path::new(&out_path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::File::create(&out_path)?;
    writeln!(f, "{}", serde_json::to_string_pretty(&out)?)?;
    eprintln!(
        "calibrate: wrote {out_path} after {:.0}s",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}
