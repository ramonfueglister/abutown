//! Headless integration test for the `winterthur-traffic` bevy_ecs shell.
//!
//! Boots the sim on the REAL baked Winterthur Gemeinde network with the REAL
//! census trips.bin at a pinned workday 07:30 boot, drives the ECS `Schedule`
//! directly (no tokio interval — as fast as the CPU allows), and asserts the
//! population / collision-free / determinism invariants.

mod common;

use common::{build_real_sim, load_real_net, load_real_net_json};
use winterthur_traffic::measure::EdgeMeasure;
use winterthur_traffic::shell::{self, CoreRes, MeasureRes, RouterRes, SimClock};

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
    // 07:30 workday boot: warm start (07:15–07:30 departures) + live morning
    // peak must populate the world.
    let (mut world, mut schedule) = build_real_sim(0xABCD, "07:30");

    for _ in 0..1000 {
        schedule.run(&mut world);
    }

    let core = &world.resource::<CoreRes>().0;
    let pop = core.fleet.alive_count();
    assert!(
        pop > 200,
        "fleet population must exceed 200 after 1000 ticks at a 07:30 boot, got {pop}"
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
fn deterministic_same_seed_and_anchor_same_hash() {
    let run = |seed: u64, at: &str| -> u64 {
        let (mut world, mut schedule) = build_real_sim(seed, at);
        for _ in 0..1000 {
            schedule.run(&mut world);
        }
        world.resource::<CoreRes>().0.state_hash()
    };

    let h1 = run(0x1234, "07:30");
    let h2 = run(0x1234, "07:30");
    assert_eq!(
        h1, h2,
        "same (seed, trips, boot anchor) must yield identical Core state_hash"
    );

    let h3 = run(0x9999, "07:30");
    assert_ne!(
        h1, h3,
        "different seeds should (almost surely) diverge — sanity check"
    );

    let h4 = run(0x1234, "03:00");
    assert_ne!(
        h1, h4,
        "a different boot anchor must produce a different spawn pattern"
    );
}

/// End-to-end wiring test for the measurement → router → CH-rebuild seam.
/// Drives the real ECS schedule past one measurement window close on the real
/// net with an injected short window, then asserts the flush actually reached
/// the router: at least one edge's smoothed travel time in the `Router` moved
/// off its free-flow baseline (proof `EdgeMeasure::flush ->
/// Router::update_weights -> rebuild` fired), and the CH still answers a route.
#[test]
fn measure_window_flush_updates_router_weights() {
    let json = load_real_net_json();
    let net = load_real_net(&json);
    let (mut world, mut schedule) = build_real_sim(0xF00D, "07:30");

    // Inject a short measurement window so a flush happens quickly instead of at
    // 3000 ticks. Overwriting the resource is enough: the `measure_edges` system
    // reads whatever `MeasureRes` holds each tick.
    const SHORT_WINDOW: u64 = 400;
    world.insert_resource(MeasureRes(EdgeMeasure::with_window(&net, SHORT_WINDOW)));

    // Baseline: the router's free-flow travel time per edge before any flush.
    let baseline: Vec<f32> = (0..net.edges.len() as u32)
        .map(|e| {
            world
                .resource::<RouterRes>()
                .0
                .edge_time_s(e)
                .expect("edge in range")
        })
        .collect();

    // Run past exactly one window close (needs the population to build up so
    // congested edges yield measured times below free-flow).
    for _ in 0..(SHORT_WINDOW + 1) {
        schedule.run(&mut world);
    }

    // A flush must have landed: some edge weight moved off its baseline.
    let router = &world.resource::<RouterRes>().0;
    let changed = (0..net.edges.len() as u32).filter(|&e| {
        let now = router.edge_time_s(e).expect("edge in range");
        (now - baseline[e as usize]).abs() > 1e-4
    });
    let n_changed = changed.count();
    assert!(
        n_changed > 0,
        "measure→router→CH-rebuild wiring did not fire: no edge weight changed \
         after a window flush at window={SHORT_WINDOW}"
    );

    // The rebuilt CH must still be queryable end-to-end: some vehicle's current
    // edge routes to some other edge (sanity that `rebuild` produced a usable
    // graph, not a broken one).
    let core = &world.resource::<CoreRes>().0;
    let mut probed = false;
    for i in 0..core.fleet.slots() {
        if let Some(view) = core.vehicle_view(i as u32) {
            // Route from this vehicle's edge to itself is always Some (same-edge
            // fast path); a non-trivial pair may be None if disconnected, which
            // is fine. We just require the CH answers the trivial query.
            assert!(
                router.route(&net, view.edge, view.edge).is_some(),
                "rebuilt CH failed a same-edge route query for edge {}",
                view.edge
            );
            probed = true;
            break;
        }
    }
    assert!(
        probed,
        "expected at least one alive vehicle to probe the CH"
    );
}

/// The `/healthz` endpoint must stay responsive while the real 10 Hz tick loop
/// is running (the #91 outage lesson: a busy tick must not starve HTTP). Boots
/// the real loop on an ephemeral port in a background task and probes health.
#[tokio::test]
async fn healthz_stays_responsive_under_live_loop() {
    let (world, schedule) = build_real_sim(0x5EED, "07:30");

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
