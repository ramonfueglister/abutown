//! Regression: demographic state must survive a snapshot save+reload without
//! replaying months. Before the fix, `LastProcessedMonth` was not persisted and
//! reset to 0 on reload while `Tick` was restored, so the first post-reload tick
//! replayed every month from 1..=month_index(tick) at once (mass die-off / baby
//! boom, duplicate agents). Uses only the public API so it compiles against the
//! pre-fix code and fails there — a true red→green test.

use std::collections::HashSet;

use sim_core::ids::AgentId;
use sim_core::mobility::api::{empty_world_and_schedule, spawn_agent_from_record};
use sim_core::mobility::components::Sex;
use sim_core::mobility::resources::{ActivityWaypoints, AgentIdIndex, Tick};
use sim_core::mobility::{
    AgentMobilityState, AgentRecord, MobilityPersistSnapshot, PlanStage, apply_into_world,
    extract_from_world,
};
use sim_core::population::{LastProcessedMonth, PopulationConfig, population_monthly_system};
use sim_core::time::{SECONDS_PER_MONTH, SECONDS_PER_YEAR, SimClock};

/// A fresh mobility world with the `"home"` activity waypoint registered, so
/// `spawn_agent_from_record` can resolve `AtActivity { activity_id: "home" }`
/// positions (mirrors how the base world seeds waypoints in production, and how
/// `population_lifecycle.rs` sets up its cohort).
fn fresh_world() -> bevy_ecs::world::World {
    let (mut world, _schedule) = empty_world_and_schedule();
    world
        .resource_mut::<ActivityWaypoints>()
        .0
        .insert("home".to_string(), (0.0, 0.0));
    world
}

/// Build a world seeded with a small cohort and aged forward several months so
/// `LastProcessedMonth` and `Tick` are both well past 0.
fn aged_world() -> bevy_ecs::world::World {
    let mut world = fresh_world();

    let clock = *world.resource::<SimClock>();
    let ticks_per_year = SECONDS_PER_YEAR / clock.sim_seconds_per_tick;
    let ticks_per_month = SECONDS_PER_MONTH / clock.sim_seconds_per_tick;

    let now_tick: u64 = 50 * ticks_per_year; // sim-year ≈ 50
    world.resource_mut::<Tick>().0 = now_tick;
    let now_month = clock.month_index(now_tick);
    world.resource_mut::<LastProcessedMonth>().0 = now_month.saturating_sub(1);
    world.insert_resource(PopulationConfig::default());

    for i in 0..6 {
        let id = AgentId(format!("agent:seed:{i}"));
        let birth_tick = now_tick - 30 * ticks_per_year;
        let mut rec = AgentRecord::new_born_at(
            id.clone(),
            AgentMobilityState::AtActivity {
                activity_id: "home".to_string(),
            },
            vec![PlanStage::Activity {
                activity_id: "home".to_string(),
            }],
            1.0,
            birth_tick,
        );
        rec.sex = if i % 2 == 0 { Sex::Female } else { Sex::Male };
        spawn_agent_from_record(&mut world, rec);
    }

    for _ in 0..3 {
        population_monthly_system(&mut world);
        let cur = world.resource::<Tick>().0;
        world.resource_mut::<Tick>().0 = cur + ticks_per_month;
    }
    population_monthly_system(&mut world);

    world
}

fn living_ids(world: &bevy_ecs::world::World) -> HashSet<String> {
    world
        .resource::<AgentIdIndex>()
        .0
        .keys()
        .map(|id| id.0.clone())
        .collect()
}

#[test]
fn reload_does_not_replay_months_or_change_population() {
    let world = aged_world();
    let before_ids = living_ids(&world);
    let before_last = world.resource::<LastProcessedMonth>().0;
    assert!(
        before_last > 0,
        "precondition: aged world advanced past month 0"
    );

    // Round-trip through JSON exactly like the persistence layer does.
    let snap = extract_from_world(&world);
    let json = serde_json::to_string(&snap).expect("serialize");
    let restored: MobilityPersistSnapshot = serde_json::from_str(&json).expect("deserialize");

    let mut reloaded = fresh_world();
    apply_into_world(&mut reloaded, restored);

    // Cursor must survive reload, NOT reset to 0.
    assert_eq!(
        reloaded.resource::<LastProcessedMonth>().0,
        before_last,
        "LastProcessedMonth must be restored to its saved value, not 0"
    );

    // One population tick at the restored tick must be a no-op: current_month
    // == last, so no month is processed — no replay, no births/deaths.
    population_monthly_system(&mut reloaded);
    assert_eq!(
        living_ids(&reloaded),
        before_ids,
        "reload + one population tick must not change the living-agent set"
    );
}

#[test]
fn last_processed_month_round_trips_through_json() {
    let world = aged_world();
    let snap = extract_from_world(&world);
    assert!(
        snap.last_processed_month > 0,
        "precondition: non-zero cursor"
    );

    let json = serde_json::to_string(&snap).expect("serialize");
    let back: MobilityPersistSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        back.last_processed_month, snap.last_processed_month,
        "last_processed_month must survive JSON serialize → deserialize"
    );
}
