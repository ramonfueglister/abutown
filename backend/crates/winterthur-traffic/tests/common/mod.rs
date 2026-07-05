//! Shared fixtures for the winterthur-traffic integration tests: the REAL
//! baked Gemeinde net + census trips.bin, bound to a pinned workday clock so
//! results never depend on when the suite runs.

// Each integration-test binary compiles its own copy of this module and none
// uses every helper — allow the per-binary leftovers.
#![allow(dead_code)]

use std::path::PathBuf;

use bevy_ecs::prelude::{Schedule, World};
use chrono::{TimeZone, Utc};
use traffic_net::TrafficNet;
use winterthur_traffic::clock::{WallClock, parse_hhmm};
use winterthur_traffic::demand::TripSchedule;
use winterthur_traffic::shell;
use winterthur_traffic::spawner::SpawnerCfg;

pub fn data_path(file: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crate dir is backend/crates/winterthur-traffic; repo root is three up.
    p.pop();
    p.pop();
    p.pop();
    p.push("data/winterthur");
    p.push(file);
    p
}

/// The raw bytes of the real baked net (needed for the trips.bin net-hash).
pub fn load_real_net_json() -> String {
    let p = data_path("trafficnet.json");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

pub fn load_real_net(json: &str) -> TrafficNet {
    traffic_net::load(json).expect("real Winterthur bake must validate")
}

/// The real census trip table, hash-bound to `net_json`.
pub fn load_real_trips(net_json: &str) -> TripSchedule {
    let p = data_path("trips.bin");
    TripSchedule::load(&p, net_json.as_bytes())
        .unwrap_or_else(|e| panic!("load {}: {e}", p.display()))
}

/// A wall clock pinned to Friday 2026-07-03 (a plain workday) at `at`
/// (`HH:MM`), so tests are independent of the real time they run at.
pub fn workday_clock(at: &str) -> WallClock {
    let now = Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap();
    WallClock::new(now, Some(parse_hhmm(at).expect("test HH:MM literal")))
}

/// Build the full sim on the REAL net + REAL trips at a pinned workday
/// time-of-day, default demand scale.
pub fn build_real_sim(seed: u64, at: &str) -> (World, Schedule) {
    let json = load_real_net_json();
    let net = load_real_net(&json);
    let trips = load_real_trips(&json);
    shell::build_sim(net, seed, trips, workday_clock(at), SpawnerCfg::default())
}
