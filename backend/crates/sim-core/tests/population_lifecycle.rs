//! Integration test: controlled-cohort lifecycle — births, deaths, determinism.
//!
//! Spawns a cohort of ~40 agents through `spawn_agent_from_record` (so they
//! land in `AgentIdIndex`) and drives the `population_monthly_system` directly
//! (no full schedule, to avoid the LOD demote system despawning agents).
//!
//! Config knobs used to guarantee events in a short run:
//!  - TFR = 50.0  → peak monthly birth prob ≈ 0.28; across 20 reproductive
//!    females, P(no birth in 12 months) < 10^-10.
//!  - mort_b = 0.5 (Gompertz amplitude), keeping mort_c = 0.0866.
//!    At age 100: μ ≈ 0.5·e^(8.66) ≈ 2868/yr → monthly q ≈ 1 − e^(−239) ≈ 1.
//!    This ensures every elder dies within the very first processed month.
//!
//! Determinism is verified by running two identical fresh worlds through the
//! same tick loop and asserting the final living-agent-id sets are equal.

use std::collections::HashSet;

use sim_core::ids::AgentId;
use sim_core::mobility::api::{empty_world_and_schedule, spawn_agent_from_record};
use sim_core::mobility::components::{ParentId, Sex};
use sim_core::mobility::resources::{AgentIdIndex, Tick};
use sim_core::mobility::{AgentMobilityState, AgentRecord, PlanStage};
use sim_core::population::{LastProcessedMonth, PopulationConfig, population_monthly_system};
use sim_core::time::{SECONDS_PER_MONTH, SimClock};

// ── helpers ─────────────────────────────────────────────────────────────────

/// Construct a test `PopulationConfig` that makes events near-certain:
///   - Raised TFR (50) → high monthly birth probability for females aged 25–35.
///   - Raised mort_b (0.5) → centenarians die on the very first month they're
///     processed (monthly q rounds to 1.0 at age ≥ 100).
fn aggressive_config() -> PopulationConfig {
    PopulationConfig {
        mort_a: 0.0001,
        mort_b: 0.5,
        mort_c: 0.0866,
        tfr: 50.0,
        fert_peak_age: 28.0,
        fert_spread: 6.0,
        fertile_min: 15.0,
        fertile_max: 49.0,
        carrying_capacity: 0.0,
        capacity_overshoot: 1.25,
    }
}

/// ticks-per-month given SimClock default rate (200 s/tick)
fn ticks_per_month(clock: &SimClock) -> u64 {
    SECONDS_PER_MONTH / clock.sim_seconds_per_tick
}

/// Build a world + seed the cohort, returning (world, seeded_id_strings, ticks_per_month).
///
/// Cohort layout (40 agents):
///   - 20 reproductive females, age 28 years at `now_tick`.
///   - 10 reproductive males, age 28 years at `now_tick` (control — should not give birth).
///   - 10 elder agents (males, age 100 years at `now_tick`).
///
/// The function overrides the PopulationConfig with `aggressive_config()`.
fn build_world_with_cohort() -> (bevy_ecs::world::World, HashSet<String>, u64) {
    let (mut world, _schedule) = empty_world_and_schedule();

    // Override config.
    world.insert_resource(aggressive_config());
    world
        .resource_mut::<sim_core::mobility::resources::ActivityWaypoints>()
        .0
        .insert("home".to_string(), (0.0, 0.0));

    let clock = *world.resource::<SimClock>();
    let tpm = ticks_per_month(&clock);

    // Choose a "current" tick large enough to express all ages correctly.
    let ticks_per_year: i64 =
        i64::try_from(sim_core::time::SECONDS_PER_YEAR / clock.sim_seconds_per_tick)
            .expect("test tick rate fits i64");
    let now_tick: i64 = 200 * ticks_per_year; // sim-time ≈ year 200

    // Initialise Tick resource to now_tick.
    world.resource_mut::<Tick>().0 = u64::try_from(now_tick).expect("test tick fits u64");

    // Set LastProcessedMonth to one before the current month so the first
    // call to population_monthly_system processes exactly one month.
    let now_month = clock.month_index(u64::try_from(now_tick).expect("test tick fits u64"));
    world.resource_mut::<LastProcessedMonth>().0 = now_month.saturating_sub(1);

    let activity_state = || AgentMobilityState::AtActivity {
        activity_id: "home".to_string(),
    };
    let plan = || {
        vec![PlanStage::Activity {
            activity_id: "home".to_string(),
        }]
    };

    let mut seeded_ids: HashSet<String> = HashSet::new();

    // 20 reproductive females, age 28.
    for i in 0..20 {
        let id = AgentId(format!("agent:female:{i}"));
        let birth_tick = now_tick - 28 * ticks_per_year;
        let mut rec =
            AgentRecord::new_born_at(id.clone(), activity_state(), plan(), 1.0, birth_tick);
        rec.sex = Sex::Female;
        spawn_agent_from_record(&mut world, rec);
        seeded_ids.insert(id.0);
    }

    // 10 reproductive males, age 28 (should not give birth).
    for i in 0..10 {
        let id = AgentId(format!("agent:male:{i}"));
        let birth_tick = now_tick - 28 * ticks_per_year;
        let mut rec =
            AgentRecord::new_born_at(id.clone(), activity_state(), plan(), 1.0, birth_tick);
        rec.sex = Sex::Male;
        spawn_agent_from_record(&mut world, rec);
        seeded_ids.insert(id.0);
    }

    // 10 elder agents, age 100.
    for i in 0..10 {
        let id = AgentId(format!("agent:elder:{i}"));
        let birth_tick = now_tick - 100 * ticks_per_year;
        let mut rec =
            AgentRecord::new_born_at(id.clone(), activity_state(), plan(), 1.0, birth_tick);
        rec.sex = Sex::Male;
        spawn_agent_from_record(&mut world, rec);
        seeded_ids.insert(id.0);
    }

    (world, seeded_ids, tpm)
}

/// Advance `world` by `months` months using only `population_monthly_system`.
/// Returns the living-agent-id string set after all months have been processed.
fn run_months(world: &mut bevy_ecs::world::World, months: u32, tpm: u64) -> HashSet<String> {
    for _ in 0..months {
        population_monthly_system(world);
        // Advance tick by one month so the system sees a new month next call.
        let cur = world.resource::<Tick>().0;
        world.resource_mut::<Tick>().0 = cur + tpm;
    }
    world
        .resource::<AgentIdIndex>()
        .0
        .keys()
        .map(|id| id.0.clone())
        .collect()
}

// ── tests ────────────────────────────────────────────────────────────────────

/// Primary lifecycle test: at least one birth AND at least one death must occur.
#[test]
fn cohort_lifecycle_births_and_deaths() {
    let (mut world, seeded_ids, tpm) = build_world_with_cohort();

    // Run for 12 months — sufficient to guarantee both births and elder deaths.
    let living = run_months(&mut world, 12, tpm);

    // --- Death assertion ---
    // Elder agents (age 100, mort_b=0.5) have near-certain monthly death probability.
    // At least some of the original elders must have been despawned.
    let living_elder_count = living
        .iter()
        .filter(|id| id.starts_with("agent:elder:"))
        .count();
    let original_elder_count = seeded_ids
        .iter()
        .filter(|id| id.starts_with("agent:elder:"))
        .count();
    assert!(
        living_elder_count < original_elder_count,
        "at least one elder agent must have died; living={living_elder_count}, original={original_elder_count}"
    );

    // --- Birth assertion ---
    // Children spawned by `population_monthly_system` have ids of the form
    // `"agent:born:<mother_id>:<month>"`.  Any living agent NOT in seeded_ids
    // is a child.  Also verify at least one has `ParentId` set.
    let born_ids: HashSet<_> = living.difference(&seeded_ids).cloned().collect();
    assert!(
        !born_ids.is_empty(),
        "at least one child must have been born; living_agent_count={}",
        living.len()
    );

    // Verify at least one born agent has a non-None ParentId.
    let has_parent = born_ids.iter().any(|id_str| {
        let agent_id = AgentId(id_str.clone());
        let entity = *world.resource::<AgentIdIndex>().0.get(&agent_id).unwrap();
        world
            .get::<ParentId>(entity)
            .and_then(|p| p.0.as_ref())
            .is_some()
    });
    assert!(
        has_parent,
        "at least one newborn must have a non-None ParentId"
    );
}

/// Determinism test: two fresh worlds with identical config + steps must
/// produce the identical living-agent-id set.
#[test]
fn cohort_lifecycle_is_deterministic() {
    // Run #1.
    let (mut world1, _seeded1, tpm1) = build_world_with_cohort();
    let living1 = run_months(&mut world1, 12, tpm1);

    // Run #2 — entirely separate world, same parameters.
    let (mut world2, _seeded2, tpm2) = build_world_with_cohort();
    let living2 = run_months(&mut world2, 12, tpm2);

    // Compute symmetric difference for diagnostics.
    let only_in_1: Vec<_> = living1.difference(&living2).cloned().collect();
    let only_in_2: Vec<_> = living2.difference(&living1).cloned().collect();
    assert!(
        only_in_1.is_empty() && only_in_2.is_empty(),
        "population simulation must be deterministic: \
         two identical worlds must converge to the same living-agent-id set.\n\
         Only in run1: {only_in_1:?}\n\
         Only in run2: {only_in_2:?}"
    );
}
