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
const TICKS_PER_WORLD_DAY: u64 =
    world_core::clock::WORLD_SECONDS_PER_DAY * world_core::TICKS_PER_SECOND / world_core::WORLD_TIME_SCALE;

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

fn env_or(key: &str, dflt: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| dflt.to_string())
}

fn main() -> anyhow::Result<()> {
    let net_path = env_or("TRAFFICNET_JSON", "data/winterthur/trafficnet.json");
    let trips_path = env_or("TRIPS_BIN", "data/winterthur/trips.bin");
    let stations_path = env_or("COUNT_STATIONS", "data/winterthur/count-stations.json");
    let out_path = env_or("CALIBRATION_OUT", "scratch/calibration/simulated-profiles.json");
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
    let state = Arc::new(Mutex::new((
        vec![NONE; 0] as Vec<u32>,               // prev edge per slot
        vec![[[0u64; 3]; 24]; stations.len()] as Counts,
    )));
    let hook_state = Arc::clone(&state);
    let watchers = Arc::new(watchers);
    world.insert_resource(SnapshotHook::new(move |snap| {
        let mut guard = hook_state.lock().expect("hook state poisoned");
        let (prev, counts) = &mut *guard;
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
    for t in 0..TICKS_PER_WORLD_DAY {
        schedule.run(&mut world);
        if t % (TICKS_PER_WORLD_DAY / 24) == 0 {
            eprintln!(
                "  world {:02}:00  ({:.0}s elapsed)",
                world_hour(t),
                started.elapsed().as_secs_f32()
            );
        }
    }

    let guard = state.lock().expect("hook state poisoned");
    let (_, counts) = &*guard;
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
