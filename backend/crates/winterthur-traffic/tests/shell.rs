//! Headless integration test for the `winterthur-traffic` bevy_ecs shell.
//!
//! Boots the sim on the REAL baked Winterthur network, drives the ECS
//! `Schedule` directly (no tokio interval — as fast as the CPU allows), and
//! asserts the population / collision-free / determinism invariants from the
//! Task 7 brief.

use std::path::PathBuf;
use traffic_net::TrafficNet;
use winterthur_traffic::shell::{self, CoreRes, SimClock};

fn data_path(file: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crate dir is backend/crates/winterthur-traffic; repo root is three up.
    p.pop();
    p.pop();
    p.pop();
    p.push("data/winterthur");
    p.push(file);
    p
}

fn load_real_net() -> TrafficNet {
    let p = data_path("trafficnet.json");
    let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    traffic_net::load(&json).expect("real Winterthur bake must validate")
}

fn load_buildings() -> String {
    let p = data_path("buildings.json");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Recompute the minimum bumper-to-bumper gap across every lane over the whole
/// fleet. A non-positive gap between two distinct vehicles on the same lane is
/// a collision. Returns `f32::INFINITY` when no lane holds two or more cars.
///
/// Independent of `Core` internals: buckets alive vehicles by lane from the
/// public `fleet` SoA, sorts each bucket by arc-position, and checks adjacent
/// pairs (the only possible same-lane collisions).
fn min_positive_gap(core: &traffic_core::Core) -> f32 {
    use std::collections::BTreeMap;
    let fleet = &core.fleet;
    let mut by_lane: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for i in 0..fleet.slots() {
        if fleet.alive[i] {
            by_lane.entry(fleet.lane[i]).or_default().push(i);
        }
    }
    let mut min_gap = f32::INFINITY;
    for slots in by_lane.values_mut() {
        // Descending s: leader first.
        slots.sort_by(|&a, &b| fleet.s[b].partial_cmp(&fleet.s[a]).unwrap());
        for w in slots.windows(2) {
            let lead = w[0]; // higher s (leader)
            let follow = w[1]; // lower s
            let gap = fleet.s[lead] - fleet.s[follow] - fleet.len_m[lead];
            if gap < min_gap {
                min_gap = gap;
            }
        }
    }
    min_gap
}

#[test]
fn boots_ticks_populates_and_is_collision_free() {
    let net = load_real_net();
    let buildings = load_buildings();
    let (mut world, mut schedule) = shell::build_sim_with_buildings(net, 0xABCD, &buildings);

    for _ in 0..1000 {
        schedule.run(&mut world);
    }

    let core = &world.resource::<CoreRes>().0;
    let pop = core.fleet.alive_count();
    assert!(
        pop > 200,
        "fleet population must exceed 200 after 1000 ticks, got {pop}"
    );

    let min_gap = min_positive_gap(core);
    assert!(
        min_gap > 0.0,
        "collision detected: min bumper gap {min_gap} <= 0"
    );

    let clock = world.resource::<SimClock>();
    assert_eq!(clock.tick, 1000, "clock must have advanced 1000 ticks");
}

#[test]
fn deterministic_same_seed_same_hash() {
    let net = load_real_net();

    let buildings = load_buildings();
    let run = |seed: u64| -> u64 {
        let (mut world, mut schedule) =
            shell::build_sim_with_buildings(net.clone(), seed, &buildings);
        for _ in 0..1000 {
            schedule.run(&mut world);
        }
        world.resource::<CoreRes>().0.state_hash()
    };

    let h1 = run(0x1234);
    let h2 = run(0x1234);
    assert_eq!(h1, h2, "same seed must yield identical Core state_hash");

    let h3 = run(0x9999);
    assert_ne!(
        h1, h3,
        "different seeds should (almost surely) diverge — sanity check"
    );
}

/// The `/healthz` endpoint must stay responsive while the real 10 Hz tick loop
/// is running (the #91 outage lesson: a busy tick must not starve HTTP). Boots
/// the real loop on an ephemeral port in a background task and probes health.
#[tokio::test]
async fn healthz_stays_responsive_under_live_loop() {
    let net = load_real_net();
    let buildings = load_buildings();
    let (world, schedule) = shell::build_sim_with_buildings(net, 0x5EED, &buildings);

    // Pick a high, unlikely-contended port for the test.
    let port = 8791u16;
    tokio::spawn(async move {
        let _ = shell::run_loop(world, schedule, port).await;
    });

    // Give the health server a moment to bind, then probe several times while
    // the loop ticks — every probe must return 200 quickly.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/healthz");
    for _ in 0..5 {
        let resp = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.get(&url).send(),
        )
        .await
        .expect("healthz must respond within 500 ms while the loop ticks")
        .expect("healthz request must succeed");
        assert!(
            resp.status().is_success(),
            "healthz status {}",
            resp.status()
        );
        let body = resp.text().await.expect("healthz body");
        assert_eq!(body, "ok");
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
}
