//! Phase 8l (slice 1): per-agent birth/death for active agents. Aggregate cohort
//! and tracked-lineage are later slices. Deterministic + replay-safe.
use bevy_ecs::prelude::*;

/// Order-independent, reproducible unit draw in [0,1) for one event.
/// Keyed by a stable agent hash, the sim-month, and a salt (0=death, 1=birth, 2=child sex).
pub fn unit_draw(agent_hash: u64, month: u64, salt: u64) -> f32 {
    let mut z = agent_hash
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(month.wrapping_mul(0xD1B5_4A32_D192_ED03))
        .wrapping_add(salt.wrapping_mul(0xCA5A_8265_7BEE_9B3D));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 40) as f32) / ((1u64 << 24) as f32)
}

pub fn stable_agent_hash(id: &crate::ids::AgentId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.0.hash(&mut h);
    h.finish()
}

/// Gompertz–Makeham instantaneous mortality hazard at `age_years`.
/// μ(t) = a + b·e^(c·t)
pub fn mortality_hazard(age_years: f32, c: &PopulationConfig) -> f32 {
    c.mort_a + c.mort_b * (c.mort_c * age_years).exp()
}

/// Discrete-time monthly death probability: 1 − e^(−μ·Δt), Δt = 1/12 year.
pub fn death_probability_month(age_years: f32, c: &PopulationConfig) -> f32 {
    let mu = mortality_hazard(age_years, c);
    1.0 - (-mu / 12.0).exp()
}

/// Gaussian-shaped age-specific fertility rate kernel (un-normalised).
fn asfr_shape(age: f32, c: &PopulationConfig) -> f32 {
    if age < c.fertile_min || age > c.fertile_max {
        return 0.0;
    }
    let z = (age - c.fert_peak_age) / c.fert_spread;
    (-0.5 * z * z).exp()
}

/// Annual age-specific fertility rate, scaled so the integer-age window sums to TFR.
pub fn fertility_rate(age: f32, c: &PopulationConfig) -> f32 {
    let shape = asfr_shape(age, c);
    if shape == 0.0 {
        return 0.0;
    }
    let norm: f32 = (c.fertile_min as i32..=c.fertile_max as i32)
        .map(|a| asfr_shape(a as f32, c))
        .sum();
    c.tfr * shape / norm
}

/// Discrete-time monthly birth probability for a female of `age_years`.
pub fn birth_probability_month(age_years: f32, c: &PopulationConfig) -> f32 {
    fertility_rate(age_years, c) / 12.0
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct PopulationConfig {
    pub mort_a: f32,
    pub mort_b: f32,
    pub mort_c: f32,
    pub tfr: f32,
    pub fert_peak_age: f32,
    pub fert_spread: f32,
    pub fertile_min: f32,
    pub fertile_max: f32,
}
impl Default for PopulationConfig {
    fn default() -> Self {
        Self {
            mort_a: 0.0001,
            mort_b: 0.00002,
            mort_c: 0.0866,
            tfr: 2.1,
            fert_peak_age: 28.0,
            fert_spread: 6.0,
            fertile_min: 15.0,
            fertile_max: 49.0,
        }
    }
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LastProcessedMonth(pub u64);

/// Combined monthly population system: mortality then fertility, one cadence.
///
/// Implemented as an exclusive system (`&mut World`) so that it has full
/// access to all world data in a single borrow.  Runs in
/// `MobilitySet::Bookkeeping` (after the Advance pass, before EventEmit)
/// so despawns/spawns happen in a clean bookkeeping phase and don't interfere
/// with mid-tick movement logic.
///
/// For each uncrossed month m in `(last+1..=current_month)`:
///
/// 1. Mortality: Gompertz–Makeham two-phase despawn for agents that die.
/// 2. Fertility: ASFR birth for living female agents in fertile window.
///
/// Updates `LastProcessedMonth` once at the end.
pub fn population_monthly_system(world: &mut World) {
    let now_tick = world.resource::<crate::mobility::resources::Tick>().0;
    let current_month = world
        .resource::<crate::time::SimClock>()
        .month_index(now_tick);
    let last = world.resource::<LastProcessedMonth>().0;

    if current_month <= last {
        return;
    }

    let clock = *world.resource::<crate::time::SimClock>();
    let cfg = *world.resource::<PopulationConfig>();

    // Collect (agent_id, entity) snapshot once — avoids re-borrowing the
    // index inside the inner loops.
    let agent_entries: Vec<(crate::ids::AgentId, Entity)> = world
        .resource::<crate::mobility::resources::AgentIdIndex>()
        .0
        .iter()
        .map(|(id, e)| (id.clone(), *e))
        .collect();

    for m in (last + 1)..=current_month {
        // ---- Mortality ----
        let mut victims: Vec<(Entity, crate::ids::AgentId)> = Vec::new();
        for (agent_id, entity) in &agent_entries {
            let Some(birth_tick) = world.get::<crate::mobility::components::BirthTick>(*entity)
            else {
                continue; // already despawned
            };
            let age = clock.age_years(now_tick, birth_tick.0);
            let draw = unit_draw(stable_agent_hash(agent_id), m, 0);
            if draw < death_probability_month(age, &cfg) {
                victims.push((*entity, agent_id.clone()));
            }
        }
        for (entity, agent_id) in victims {
            world.despawn(entity);
            world
                .resource_mut::<crate::mobility::resources::AgentIdIndex>()
                .0
                .remove(&agent_id);
        }

        // ---- Fertility ----
        // Collect living females in fertile window; also grab their position
        // and a copy of their mobility state so we can spawn at the mother's
        // exact location without needing activity geometry resolution.
        struct BirthCandidate {
            mother_id: crate::ids::AgentId,
            mother_pos: crate::mobility::components::Position,
            mother_state: crate::mobility::AgentMobilityState,
            mother_plan: Vec<crate::mobility::PlanStage>,
            mother_plan_cursor: usize,
            walk_speed: f32,
        }
        let mut candidates: Vec<BirthCandidate> = Vec::new();
        for (agent_id, entity) in &agent_entries {
            // Skip if already despawned this month
            let Some(birth_tick) = world.get::<crate::mobility::components::BirthTick>(*entity)
            else {
                continue;
            };
            let Some(sex) = world.get::<crate::mobility::components::Sex>(*entity) else {
                continue;
            };
            if *sex != crate::mobility::components::Sex::Female {
                continue;
            }
            let age = clock.age_years(now_tick, birth_tick.0);
            if age < cfg.fertile_min || age > cfg.fertile_max {
                continue;
            }
            let draw = unit_draw(stable_agent_hash(agent_id), m, 1);
            if draw >= birth_probability_month(age, &cfg) {
                continue;
            }
            // Candidate gives birth this month-step.
            let Some(pos) = world.get::<crate::mobility::components::Position>(*entity) else {
                continue;
            };
            let pos = *pos;
            let Some(state_comp) =
                world.get::<crate::mobility::components::AgentMobilityStateComponent>(*entity)
            else {
                continue;
            };
            let mother_state = state_comp.0.clone();
            let (mother_plan, mother_plan_cursor, walk_speed) = world
                .get::<crate::mobility::components::WalkPlan>(*entity)
                .map(|wp| (wp.stages.clone(), wp.cursor, 0.0))
                .unwrap_or_default();
            let walk_speed = world
                .get::<crate::mobility::components::WalkSpeed>(*entity)
                .map(|ws| ws.0)
                .unwrap_or(walk_speed);
            candidates.push(BirthCandidate {
                mother_id: agent_id.clone(),
                mother_pos: pos,
                mother_state,
                mother_plan,
                mother_plan_cursor,
                walk_speed,
            });
        }

        // Spawn children after collecting all candidates to avoid borrow
        // conflicts on the world.
        for candidate in candidates {
            let child_id =
                crate::ids::AgentId(format!("agent:born:{}:{}", candidate.mother_id.0, m));
            // Determine child sex from a second draw (salt=2).
            let sex_draw = unit_draw(stable_agent_hash(&child_id), m, 2);
            let child_sex = if sex_draw >= 0.5 {
                crate::mobility::components::Sex::Female
            } else {
                crate::mobility::components::Sex::Male
            };

            // Build child record from the mother's mobility state and place it
            // at the mother's current authoritative world coordinate.
            let mut child_record = crate::mobility::AgentRecord::new_born_at(
                child_id,
                candidate.mother_state.clone(),
                candidate.mother_plan.clone(),
                candidate.walk_speed,
                now_tick,
            );
            child_record.plan_cursor = candidate.mother_plan_cursor;
            child_record.sex = child_sex;
            child_record.parent_id = Some(candidate.mother_id.clone());

            crate::mobility::api::spawn_agent_from_record_at_position(
                world,
                child_record,
                (candidate.mother_pos.x, candidate.mother_pos.y),
            );
        }
    }

    world.resource_mut::<LastProcessedMonth>().0 = current_month;
}

/// Retained for backwards-compatibility with tests that call it directly.
/// Delegates to `population_monthly_system`.
pub fn mortality_system(world: &mut World) {
    population_monthly_system(world);
}

pub struct PopulationPlugin;
impl crate::world::schedule::SimPlugin for PopulationPlugin {
    fn name(&self) -> &'static str {
        "population"
    }
    fn install(
        &self,
        world: &mut bevy_ecs::world::World,
        schedule: &mut bevy_ecs::schedule::Schedule,
    ) {
        world.insert_resource(PopulationConfig::default());
        world.insert_resource(LastProcessedMonth::default());
        // Run the combined population system in MobilitySet::Bookkeeping —
        // same phase as tick_increment_system, after Advance and Output, so
        // despawns/spawns happen after all movement logic is done.
        schedule.add_systems(
            population_monthly_system
                .in_set(crate::mobility::systems::MobilitySet::Bookkeeping)
                .before(crate::mobility::systems::tick_increment_system),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_draw_is_in_range_and_deterministic() {
        let a = unit_draw(7, 3, 1);
        assert_eq!(a, unit_draw(7, 3, 1));
        assert!((0.0..1.0).contains(&a));
        assert!(unit_draw(7, 3, 1) != unit_draw(8, 3, 1));
    }
    #[test]
    fn config_has_sane_defaults() {
        let c = PopulationConfig::default();
        assert!(c.mort_c > 0.0 && c.tfr > 0.0 && c.fertile_min < c.fertile_max);
    }
    #[test]
    fn gompertz_makeham_monotonic_and_bounded() {
        let c = PopulationConfig::default();
        assert!(mortality_hazard(80.0, &c) > mortality_hazard(20.0, &c));
        let q = death_probability_month(80.0, &c);
        assert!(q > 0.0 && q < 1.0);
        assert!(death_probability_month(80.0, &c) > death_probability_month(20.0, &c));
    }

    #[test]
    fn asfr_peaks_in_window_and_scales_to_tfr() {
        let c = PopulationConfig::default();
        assert!(fertility_rate(28.0, &c) > fertility_rate(45.0, &c));
        assert_eq!(fertility_rate(10.0, &c), 0.0);
        let total: f32 = (15..=49).map(|a| fertility_rate(a as f32, &c)).sum();
        assert!((total - c.tfr).abs() < 0.05 * c.tfr, "got {total}");
    }

    #[test]
    fn old_agent_dies_deterministically() {
        use crate::ids::AgentId;
        use crate::mobility::components::{AgentMarker, BirthTick, StableAgentId};
        use crate::mobility::resources::{AgentIdIndex, Tick};
        use crate::time::SimClock;
        use bevy_ecs::prelude::*;
        use bevy_ecs::schedule::Schedule;

        // Build a minimal world containing only the resources the mortality
        // system needs. Avoids the full mobility schedule so the LOD systems
        // (which demote/despawn agents at Warm chunks) don't interfere.
        let mut world = World::new();
        let mut schedule = Schedule::default();

        world.insert_resource(SimClock::default());
        world.insert_resource(PopulationConfig::default());
        world.insert_resource(LastProcessedMonth::default());
        world.insert_resource(Tick(0));
        world.insert_resource(AgentIdIndex::default());

        schedule.add_systems(mortality_system);

        // Spawn one agent born at tick 0 with just the components mortality
        // needs: AgentMarker + BirthTick + StableAgentId.
        let agent_id = AgentId("agent:mortal:0".to_string());
        let entity = world
            .spawn((AgentMarker, StableAgentId(agent_id.clone()), BirthTick(0)))
            .id();
        world
            .resource_mut::<AgentIdIndex>()
            .0
            .insert(agent_id.clone(), entity);

        // Set Tick to 130 sim-years. SimClock default is 200 s/tick;
        // SECONDS_PER_YEAR = 31_536_000; 130 yr = 20_498_400 s = 102_492 ticks.
        // Set LastProcessedMonth to target_month-1 so the system processes
        // exactly one month per schedule.run call.
        // Monthly death probability at 130 yr ≈ 12%, so cumulative 100-month
        // survival ≈ 0.88^100 ≈ 0.00002 — effectively certain to die.
        let ticks_per_year: u64 = crate::time::SECONDS_PER_YEAR / 200;
        let now_tick = 130 * ticks_per_year;
        let clock = *world.resource::<SimClock>();
        let target_month = clock.month_index(now_tick);
        world.resource_mut::<LastProcessedMonth>().0 = target_month - 1;
        world.resource_mut::<Tick>().0 = now_tick;

        let ticks_per_month = crate::time::SECONDS_PER_MONTH / 200;
        let mut despawned = false;
        for _ in 0..100 {
            schedule.run(&mut world);
            let still_alive = world.resource::<AgentIdIndex>().0.contains_key(&agent_id);
            if !still_alive {
                despawned = true;
                break;
            }
            // Advance by one month for the next iteration.
            let cur = world.resource::<Tick>().0;
            world.resource_mut::<Tick>().0 = cur + ticks_per_month;
        }
        assert!(
            despawned,
            "130-year-old agent must be despawned by mortality system within 100 months"
        );
    }

    /// Integration test: a 28-year-old female agent should produce a newborn
    /// (age=0, sex set, parent_id correct) within a small number of months
    /// when TFR is set very high (100), making each monthly draw near-certain.
    /// TFR=2.1 gives only ~50% in the remaining fertile years; this test uses
    /// an elevated TFR to be deterministic with a fixed seed.
    #[test]
    fn female_agent_gives_birth_deterministically() {
        use crate::ids::AgentId;
        use crate::mobility::components::{
            AgentMarker, AgentMobilityStateComponent, BirthTick, ParentId, Sex, StableAgentId,
            WalkPlan, WalkSpeed,
        };
        use crate::mobility::resources::{AgentIdIndex, Tick};
        use crate::mobility::{AgentMobilityState, PlanStage};
        use crate::time::SimClock;
        use bevy_ecs::prelude::*;
        use bevy_ecs::schedule::Schedule;

        let mut world = World::new();
        let mut schedule = Schedule::default();

        world.insert_resource(SimClock::default());
        // Use a very high TFR so the monthly birth probability is large (~56% at
        // peak) and birth is guaranteed within a few iterations.
        world.insert_resource(PopulationConfig {
            tfr: 100.0,
            ..PopulationConfig::default()
        });
        world.insert_resource(LastProcessedMonth::default());
        world.insert_resource(Tick(0));
        world.insert_resource(AgentIdIndex::default());

        schedule.add_systems(population_monthly_system);

        // birth_tick chosen so age ≈ 28 years at now_tick.
        let ticks_per_year: u64 = crate::time::SECONDS_PER_YEAR / 200;
        let age_ticks = 28 * ticks_per_year;
        let now_tick = 100 * ticks_per_year; // arbitrary "now"
        let mother_birth_tick = now_tick - age_ticks;

        let mother_id = AgentId("agent:mother:0".to_string());
        let state = AgentMobilityState::AtActivity {
            activity_id: "home".to_string(),
        };
        let plan = vec![PlanStage::Activity {
            activity_id: "home".to_string(),
        }];
        let mother_entity = world
            .spawn((
                AgentMarker,
                StableAgentId(mother_id.clone()),
                BirthTick(mother_birth_tick),
                Sex::Female,
                AgentMobilityStateComponent(state),
                WalkPlan {
                    stages: plan,
                    cursor: 0,
                    cyclic: false,
                },
                WalkSpeed(1.0),
                crate::mobility::components::Position { x: 16.0, y: 16.0 },
            ))
            .id();
        world
            .resource_mut::<AgentIdIndex>()
            .0
            .insert(mother_id.clone(), mother_entity);

        // Start just before now_tick's month so each schedule.run processes one month.
        let clock = *world.resource::<SimClock>();
        let now_month = clock.month_index(now_tick);
        world.resource_mut::<LastProcessedMonth>().0 = now_month - 1;
        world.resource_mut::<Tick>().0 = now_tick;

        let ticks_per_month = crate::time::SECONDS_PER_MONTH / 200;

        // TFR=100 ⟹ peak monthly rate ≈ 0.56. Run 20 months —
        // P(no birth) ≈ (0.44)^20 < 10^-7, effectively impossible.
        let mut child_born = false;
        let mut child_info: Option<(Sex, Option<AgentId>)> = None;
        let mut born_tick: Option<u64> = None;
        for _iter in 0..20 {
            let tick_before = world.resource::<Tick>().0;
            schedule.run(&mut world);
            // Detect any agent in the index that is not the mother — that's a child.
            let child_entry = {
                let index = world.resource::<AgentIdIndex>();
                index
                    .0
                    .iter()
                    .filter(|(id, _)| **id != mother_id)
                    .map(|(id, entity)| (id.clone(), *entity))
                    .next()
            };
            if let Some((_child_id, child_entity)) = child_entry {
                let sex = world
                    .get::<Sex>(child_entity)
                    .copied()
                    .expect("child must have Sex");
                let parent = world
                    .get::<ParentId>(child_entity)
                    .cloned()
                    .expect("child must have ParentId");
                let bt = world
                    .get::<BirthTick>(child_entity)
                    .map(|b| b.0)
                    .unwrap_or(0);
                child_info = Some((sex, parent.0));
                born_tick = Some(bt);
                // The birth_tick must equal the tick at which the system ran.
                assert_eq!(
                    bt, tick_before,
                    "newborn birth_tick {bt} must equal the tick when system ran {tick_before}"
                );
                child_born = true;
                break;
            }
            let cur = world.resource::<Tick>().0;
            world.resource_mut::<Tick>().0 = cur + ticks_per_month;
        }

        assert!(
            child_born,
            "a 28-year-old female must give birth within 20 months at TFR=100"
        );
        let (_child_sex, parent) = child_info.unwrap();
        assert_eq!(
            parent,
            Some(mother_id.clone()),
            "child's parent_id must be the mother's AgentId"
        );
        let _ = born_tick;
    }
}
