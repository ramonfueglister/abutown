//! Vehicle-conservation audit (Task 7, spec §8): at every tick
//! `spawned == arrived + alive`, where *arrived* counts every end-of-route
//! despawn — **including gateway sinks** (routes ending on a boundary stub's
//! in-lane despawn via the kernel's normal end-of-route path, Task 6).
//!
//! The fast test runs the full ECS shell on the diamond-gateway fixture with
//! steady gateway→gateway through-demand; the `#[ignore]`d one replays 30
//! sim-minutes of the REAL census morning peak on the REAL Gemeinde net
//! (run locally: `cargo test -p winterthur-traffic --test conservation --
//! --ignored --nocapture`).

mod common;

use std::io::Write as _;
use std::path::PathBuf;

use demand_gen::output::{self, TripRecord};
use winterthur_traffic::audit::Conservation;
use winterthur_traffic::demand::TripSchedule;
use winterthur_traffic::shell::{self, CoreRes};
use winterthur_traffic::spawner::SpawnerCfg;

fn fixture_json() -> String {
    let p = format!(
        "{}/tests/fixtures/diamond-gateway.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

/// Write a trips.bin for `net_json` to a unique temp file and load it
/// (mirrors the spawner unit tests' helper — hash-bound to the net).
fn make_schedule(name: &str, net_json: &str, weekday: &[TripRecord]) -> TripSchedule {
    let net_hash = *blake3::hash(net_json.as_bytes()).as_bytes();
    let mut bytes = Vec::new();
    output::write_trips(&mut bytes, &net_hash, weekday, &[]).unwrap();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "winterthur-traffic-conservation-test-{}-{name}.bin",
        std::process::id()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&bytes).unwrap();
    TripSchedule::load(&path, net_json.as_bytes()).unwrap()
}

/// 5 000 ticks (500 sim-seconds) at a 07:30 workday boot on the diamond
/// fixture with one gateway→gateway trip per second: the invariant must hold
/// at every 500-tick checkpoint and gateway sinks must actually fire
/// (`arrived > 0`).
#[test]
fn conservation_holds_with_gateway_sinks_on_fixture() {
    let json = fixture_json();
    let net = traffic_net::load(&json).expect("diamond-gateway fixture must validate");
    // Steady demand over the whole run: departures every sim-second from
    // 07:30:00 (= 27 000 s) for 450 s, all through-traffic gateway→gateway
    // (origin lane 0 leaves gateway node 0; dest lane 5 ends at gateway 5).
    let trips: Vec<TripRecord> = (0..450)
        .map(|k| TripRecord {
            departure_s: 27_000 + k,
            origin_lane: 0,
            dest_lane: 5,
            segment: output::SEGMENT_THROUGH,
            vehicle_class: 0,
        })
        .collect();
    let schedule = make_schedule("fixture", &json, &trips);
    let (mut world, mut ecs) = shell::build_sim(
        net,
        0x00C0_FFEE,
        schedule,
        common::workday_clock("07:30"),
        SpawnerCfg::default(),
    );

    for block in 1..=10u32 {
        for _ in 0..500 {
            ecs.run(&mut world);
        }
        let alive = world.resource::<CoreRes>().0.fleet.alive_count();
        let cons = *world.resource::<Conservation>();
        assert!(
            cons.holds(alive),
            "conservation violated at tick {}: {cons:?} alive={alive}",
            block * 500
        );
    }

    let cons = *world.resource::<Conservation>();
    assert!(
        cons.spawned > 100,
        "demand must spawn steadily over 5000 ticks, got {cons:?}"
    );
    assert!(
        cons.arrived > 0,
        "gateway sinks must fire (arrivals counted), got {cons:?}"
    );
}

/// 30 sim-minutes (18 000 ticks) of the REAL census morning peak on the REAL
/// Gemeinde net: invariant at every 1 000-tick checkpoint, the fleet climbs
/// past 200, and nothing panics. Ignored in CI (needs the baked
/// `data/winterthur/{trafficnet.json,trips.bin}` artifacts + ~minutes of CPU).
#[test]
#[ignore = "real net + real trips.bin, 18k ticks — run locally with -- --ignored"]
fn conservation_holds_on_real_net_30_sim_minutes() {
    let (mut world, mut ecs) = common::build_real_sim(0x0AB7_07A1, "07:30");

    let mut peak_alive = 0usize;
    for block in 1..=18u32 {
        for _ in 0..1000 {
            ecs.run(&mut world);
        }
        let alive = world.resource::<CoreRes>().0.fleet.alive_count();
        peak_alive = peak_alive.max(alive);
        let cons = *world.resource::<Conservation>();
        assert!(
            cons.holds(alive),
            "conservation violated at tick {}: {cons:?} alive={alive}",
            block * 1000
        );
    }

    let cons = *world.resource::<Conservation>();
    assert!(
        peak_alive > 200,
        "morning-peak fleet must climb above 200, peak={peak_alive}"
    );
    assert!(
        cons.arrived > 0,
        "30 sim-minutes must complete some trips, got {cons:?}"
    );
    println!("real-net conservation: 18000 ticks, peak_alive={peak_alive}, {cons:?}");
}
