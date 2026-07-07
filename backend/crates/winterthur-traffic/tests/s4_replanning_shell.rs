//! S4 shell-integration soak: the day-to-day replanning wiring (spawner uses
//! learned plan routes, arrivals are scored, the between-day pass fires on the
//! world-midnight wrap) runs on the REAL Winterthur net across a world-day
//! boundary without breaking the sim's invariants — vehicle conservation,
//! collision-freedom, and seed-reproducible determinism — while plan memories
//! actually accumulate.

mod common;

use common::build_real_sim;
use std::collections::BTreeMap;
use winterthur_traffic::shell::{CoreRes, LastWorldDay, ReplanningRes};

/// Boot the traffic sim near world midnight so the first between-day
/// replanning pass fires within a few hundred ticks, then run past the wrap
/// into the early world morning.
const BOOT_AT: &str = "23:58";
const TICKS: u64 = 2000;

/// Minimum bumper gap across the fleet (negative ⇒ overlap/collision).
fn min_positive_gap(core: &traffic_core::Core) -> f32 {
    let fleet = &core.fleet;
    let mut by_lane: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for i in 0..fleet.slots() {
        if fleet.alive[i] {
            by_lane.entry(fleet.lane[i]).or_default().push(i);
        }
    }
    let mut min_gap = f32::INFINITY;
    for slots in by_lane.values_mut() {
        slots.sort_by(|&a, &b| fleet.s[b].partial_cmp(&fleet.s[a]).unwrap());
        for w in slots.windows(2) {
            let gap = fleet.s[w[0]] - fleet.s[w[1]] - fleet.len_m[w[0]];
            min_gap = min_gap.min(gap);
        }
    }
    min_gap
}

#[test]
fn replanning_soak_over_world_midnight_holds_invariants() {
    let (mut world, mut schedule) = build_real_sim(0xB4B4, BOOT_AT);

    for _ in 0..TICKS {
        schedule.run(&mut world);
        // Conservation is asserted inside `core_tick` (debug_assert) every
        // tick; a collision surfaces as a negative gap.
        let core = &world.resource::<CoreRes>().0;
        let g = min_positive_gap(core);
        assert!(g > -0.01, "collision with replanning active: min gap {g}");
    }

    // The between-day pass must have fired at least once (world crossed
    // midnight) — its bookkeeping resource advanced past the day-0 default.
    let last_day = world.resource::<LastWorldDay>().0;
    assert!(
        last_day >= 1,
        "between-day replanning never fired (LastWorldDay={last_day}) — boot/wrap timing off"
    );

    // Plan memories accumulated: the spawner seeded/tracked recurring trips.
    let tracked = world.resource::<ReplanningRes>().0.tracked_trips();
    assert!(tracked > 0, "no trip plan memories were created");
}

#[test]
fn replanning_is_seed_reproducible() {
    // The whole wired sim (replanning included) must be reproducible from the
    // seed + boot anchor: same inputs → identical kernel state_hash AND the
    // same number of tracked trip memories after the same ticks. Guards against
    // any non-deterministic iteration (e.g. a HashMap) creeping into the
    // replanning path — the memories use a BTreeMap precisely for this.
    let run = || -> (u64, usize) {
        let (mut world, mut schedule) = build_real_sim(0x5EED, BOOT_AT);
        for _ in 0..TICKS {
            schedule.run(&mut world);
        }
        (
            world.resource::<CoreRes>().0.state_hash(),
            world.resource::<ReplanningRes>().0.tracked_trips(),
        )
    };
    assert_eq!(
        run(),
        run(),
        "replanning-wired sim must be seed-reproducible"
    );
}
