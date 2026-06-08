//! Phase 8l (slice 1): per-agent birth/death for active agents. Aggregate cohort
//! and tracked-lineage are later slices. Deterministic + replay-safe.
use bevy_ecs::prelude::*;

/// The persisted cursor recording the last sim-month this system advanced
/// through. Defined in `mobility::resources` next to `Tick` (both are persisted
/// in the mobility snapshot and installed by `install_mobility`); re-exported
/// here because the population system owns its semantics.
pub use crate::mobility::resources::LastProcessedMonth;

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

/// Density-dependent fertility multiplier in `[0,1]`. Full fertility (1.0) while
/// the active population `n` is at or below the carrying capacity `K`; linear ramp
/// 1→0 across `[K, K_hard]` where `K_hard = K * max(capacity_overshoot, 1.0)`; 0
/// at/above `K_hard`. `K <= 0` disables regulation (returns 1.0 — unbounded). A
/// zero-width band (`capacity_overshoot <= 1.0` ⇒ `K_hard == K`) degenerates to a
/// clean hard cap exactly at `K` (1.0 below, 0.0 at/above) — never NaN.
///
/// NOTE: deliberately NOT `1 - n/K`. The base schedule is only mildly
/// super-replacement (NRR≈1.044), so a linear-from-zero suppression would balance
/// at ~4% of K and collapse the population; the ceiling form keeps full fertility
/// until `n` nears `K`, so the bounded equilibrium sits just above `K`.
pub fn fertility_density_factor(n: usize, c: &PopulationConfig) -> f32 {
    let k = c.carrying_capacity;
    if k <= 0.0 {
        return 1.0;
    }
    let k_hard = k * c.capacity_overshoot.max(1.0);
    let n = n as f32;
    // Floor the divisor so a zero-width band (k_hard == k) yields a hard step at K
    // instead of 0.0/0.0 = NaN.
    let denom = (k_hard - k).max(f32::EPSILON);
    ((k_hard - n) / denom).clamp(0.0, 1.0)
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
    /// Active-population carrying capacity K. Fertility is full at/below K and
    /// ramps to zero across [K, K*capacity_overshoot]. `<= 0.0` disables
    /// regulation (unbounded growth). Set per-world by the runtime.
    pub carrying_capacity: f32,
    /// Upper band as a multiple of K: hard fertility ceiling K_hard = K*overshoot.
    pub capacity_overshoot: f32,
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
            carrying_capacity: 0.0, // unbounded by default; the runtime sets it per-world
            capacity_overshoot: 1.25,
        }
    }
}

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
            let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
            let draw = unit_draw(stable_agent_hash(agent_id), m, 0);
            if draw < death_probability_month(age, &cfg) {
                victims.push((*entity, agent_id.clone()));
            }
        }
        let deaths = victims.len();
        for (entity, agent_id) in victims {
            world.despawn(entity);
            world
                .resource_mut::<crate::mobility::resources::AgentIdIndex>()
                .0
                .remove(&agent_id);
        }

        // ---- Fertility (density-regulated) ----
        let live_n = world.resource::<crate::mobility::resources::AgentIdIndex>().0.len();
        let density = fertility_density_factor(live_n, &cfg);
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
            mother_binding: Option<crate::mobility::MarketBinding>,
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
            let age = clock.age_years_at(clock.month_start_seconds(m), birth_tick.0);
            if age < cfg.fertile_min || age > cfg.fertile_max {
                continue;
            }
            let draw = unit_draw(stable_agent_hash(agent_id), m, 1);
            if draw >= birth_probability_month(age, &cfg) * density {
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
            let mother_binding = world
                .get::<crate::mobility::MarketBinding>(*entity)
                .copied();
            candidates.push(BirthCandidate {
                mother_id: agent_id.clone(),
                mother_pos: pos,
                mother_state,
                mother_plan,
                mother_plan_cursor,
                walk_speed,
                mother_binding,
            });
        }

        // Spawn children after collecting all candidates to avoid borrow
        // conflicts on the world.
        let births = candidates.len();
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
                i64::try_from(now_tick).unwrap_or(i64::MAX),
            );
            child_record.plan_cursor = candidate.mother_plan_cursor;
            child_record.sex = child_sex;
            child_record.parent_id = Some(candidate.mother_id.clone());
            if let Some(b) = candidate.mother_binding {
                child_record.home_market = b.home_market;
                child_record.work_market = b.work_market;
            }

            crate::mobility::api::spawn_agent_from_record_at_position(
                world,
                child_record,
                (candidate.mother_pos.x, candidate.mother_pos.y),
            );
        }

        let live_after = world.resource::<crate::mobility::resources::AgentIdIndex>().0.len();
        tracing::info!(
            target: "population::liveness",
            month = m,
            n = live_after,
            births = births,
            deaths = deaths,
            "population month"
        );
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
        // `LastProcessedMonth` is installed by `install_mobility` (it is a
        // mobility-snapshot-persisted cursor, like `Tick`); no insert here.
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
    fn density_factor_unbounded_when_capacity_non_positive() {
        let c = PopulationConfig { carrying_capacity: 0.0, ..PopulationConfig::default() };
        assert_eq!(fertility_density_factor(0, &c), 1.0);
        assert_eq!(fertility_density_factor(100_000, &c), 1.0);
    }

    #[test]
    fn density_factor_full_below_k_zero_at_hard_ceiling() {
        // K=100, overshoot 1.25 => K_hard=125.
        let c = PopulationConfig { carrying_capacity: 100.0, capacity_overshoot: 1.25, ..PopulationConfig::default() };
        assert_eq!(fertility_density_factor(50, &c), 1.0, "full fertility well below K");
        assert_eq!(fertility_density_factor(100, &c), 1.0, "full fertility at K");
        let mid = fertility_density_factor(112, &c); // ~halfway through [100,125]
        assert!(mid > 0.4 && mid < 0.6, "linear ramp in the band, got {mid}");
        assert_eq!(fertility_density_factor(125, &c), 0.0, "zero at K_hard");
        assert_eq!(fertility_density_factor(200, &c), 0.0, "zero above K_hard");
    }

    #[test]
    fn density_factor_zero_width_band_is_hard_cap_not_nan() {
        // capacity_overshoot <= 1.0 ⇒ K_hard == K (zero-width band): must degenerate
        // to a clean hard cap at K (1.0 below, 0.0 at/above), never NaN.
        let c = PopulationConfig { carrying_capacity: 100.0, capacity_overshoot: 1.0, ..PopulationConfig::default() };
        for n in [0usize, 50, 99] {
            let f = fertility_density_factor(n, &c);
            assert!(f.is_finite() && f == 1.0, "n={n} below K must be 1.0, got {f}");
        }
        for n in [100usize, 101, 500] {
            let f = fertility_density_factor(n, &c);
            assert!(f.is_finite() && f == 0.0, "n={n} at/above K must be 0.0, got {f}");
        }
    }

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
        let ticks_per_year: i64 =
            i64::try_from(crate::time::SECONDS_PER_YEAR / 200).expect("test tick rate fits i64");
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
        let now_tick_u64 = u64::try_from(now_tick).expect("test tick fits u64");
        let now_month = clock.month_index(now_tick_u64);
        world.resource_mut::<LastProcessedMonth>().0 = now_month - 1;
        world.resource_mut::<Tick>().0 = now_tick_u64;

        let ticks_per_month = crate::time::SECONDS_PER_MONTH / 200;

        // TFR=100 ⟹ peak monthly rate ≈ 0.56. Run 20 months —
        // P(no birth) ≈ (0.44)^20 < 10^-7, effectively impossible.
        let mut child_born = false;
        let mut child_info: Option<(Sex, Option<AgentId>)> = None;
        let mut born_tick: Option<i64> = None;
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
                    bt,
                    i64::try_from(tick_before).expect("test tick fits i64"),
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

    /// Per-month age: in a multi-month catch-up, fertility must be judged by the
    /// agent's age in each processed month, not at the final tick. The mother is
    /// past `fertile_max` at now_tick but fertile in the early processed months;
    /// with per-month age she gives birth, with now-age she never would.
    #[test]
    fn catch_up_judges_fertility_by_per_month_age() {
        use crate::ids::AgentId;
        use crate::mobility::components::{
            AgentMarker, AgentMobilityStateComponent, BirthTick, Sex, StableAgentId, WalkPlan,
            WalkSpeed,
        };
        use crate::mobility::resources::{AgentIdIndex, Tick};
        use crate::mobility::{AgentMobilityState, PlanStage};
        use crate::time::{SECONDS_PER_YEAR, SimClock};
        use bevy_ecs::prelude::*;
        use bevy_ecs::schedule::Schedule;

        let mut world = World::new();
        let mut schedule = Schedule::default();
        world.insert_resource(SimClock::default());
        world.insert_resource(PopulationConfig {
            mort_a: 0.0,
            mort_b: 0.0,
            tfr: 1000.0,
            ..PopulationConfig::default()
        });
        world.insert_resource(LastProcessedMonth::default());
        world.insert_resource(Tick(0));
        world.insert_resource(AgentIdIndex::default());
        schedule.add_systems(population_monthly_system);

        let clock = *world.resource::<SimClock>();
        let ticks_per_year = i64::try_from(SECONDS_PER_YEAR / clock.sim_seconds_per_tick)
            .expect("test tick rate fits i64");
        let now_tick = 200 * ticks_per_year;
        let mother_birth_tick = now_tick - 55 * ticks_per_year; // 55 > fertile_max
        let age28_tick = mother_birth_tick + 28 * ticks_per_year;
        let start_month = clock.month_index(u64::try_from(age28_tick).expect("test tick fits u64"));
        world.resource_mut::<LastProcessedMonth>().0 = start_month.saturating_sub(1);
        world.resource_mut::<Tick>().0 = u64::try_from(now_tick).expect("test tick fits u64");

        let mother_id = AgentId("agent:mother:permonth".to_string());
        let entity = world
            .spawn((
                AgentMarker,
                StableAgentId(mother_id.clone()),
                BirthTick(mother_birth_tick),
                Sex::Female,
                AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                    activity_id: "home".to_string(),
                }),
                WalkPlan {
                    stages: vec![PlanStage::Activity {
                        activity_id: "home".to_string(),
                    }],
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
            .insert(mother_id.clone(), entity);

        schedule.run(&mut world);

        let born = world
            .resource::<AgentIdIndex>()
            .0
            .keys()
            .any(|id| id.0.starts_with("agent:born:"));
        assert!(
            born,
            "mother must give birth in an early fertile month (per-month age)"
        );
    }

    // ---- Reusable test harness -----------------------------------------------

    struct PopulationTestHarness {
        world: bevy_ecs::world::World,
        schedule: bevy_ecs::schedule::Schedule,
        /// Monotonically-increasing synthetic agent counter, used to generate
        /// unique deterministic AgentIds for `seed_agents`.
        next_seed_id: usize,
    }

    impl PopulationTestHarness {
        fn new(cfg: PopulationConfig) -> Self {
            use crate::mobility::resources::{AgentIdIndex, Tick};
            use crate::time::SimClock;
            use bevy_ecs::prelude::*;
            use bevy_ecs::schedule::Schedule;

            let mut world = World::new();
            let mut schedule = Schedule::default();

            world.insert_resource(SimClock::default());
            world.insert_resource(cfg);
            world.insert_resource(super::LastProcessedMonth::default());
            world.insert_resource(Tick(0));
            world.insert_resource(AgentIdIndex::default());

            schedule.add_systems(population_monthly_system);

            // Set the tick to 1 full sim-year in so there is a valid "current
            // month" to process, and set LastProcessedMonth so the FIRST
            // advance_one_month call processes exactly one month.
            let clock = *world.resource::<SimClock>();
            let ticks_per_year = crate::time::SECONDS_PER_YEAR / clock.sim_seconds_per_tick;
            let now_tick = ticks_per_year; // 1 year in
            let current_month = clock.month_index(now_tick);
            world.resource_mut::<super::LastProcessedMonth>().0 = current_month; // nothing to process yet
            world.resource_mut::<Tick>().0 = now_tick;

            Self { world, schedule, next_seed_id: 0 }
        }

        /// Spawn `n` agents with deterministic ids, `Sex::Female` (fertile),
        /// and `BirthTick` chosen so each agent's age falls inside `age_range`
        /// (at the current `now_tick`). Ages are distributed evenly across the
        /// range by cycling through it.
        fn seed_agents(&mut self, n: usize, age_range: std::ops::Range<u32>) {
            use crate::ids::AgentId;
            use crate::mobility::components::{
                AgentMarker, AgentMobilityStateComponent, BirthTick, Sex, StableAgentId, WalkPlan,
                WalkSpeed,
            };
            use crate::mobility::resources::Tick;
            use crate::mobility::{AgentMobilityState, PlanStage};

            let clock = *self.world.resource::<crate::time::SimClock>();
            let now_tick_u64 = self.world.resource::<Tick>().0;
            let ticks_per_year =
                i64::try_from(crate::time::SECONDS_PER_YEAR / clock.sim_seconds_per_tick)
                    .expect("test tick rate fits i64");
            let range_len = (age_range.end - age_range.start).max(1) as usize;

            for i in 0..n {
                let age_years = age_range.start as i64
                    + (i % range_len) as i64;
                let birth_tick =
                    i64::try_from(now_tick_u64).expect("test tick fits i64")
                        - age_years * ticks_per_year;

                let id = AgentId(format!("agent:seed:{}", self.next_seed_id));
                self.next_seed_id += 1;

                let entity = self
                    .world
                    .spawn((
                        AgentMarker,
                        StableAgentId(id.clone()),
                        BirthTick(birth_tick),
                        Sex::Female,
                        AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                            activity_id: "home".to_string(),
                        }),
                        WalkPlan {
                            stages: vec![PlanStage::Activity {
                                activity_id: "home".to_string(),
                            }],
                            cursor: 0,
                            cyclic: false,
                        },
                        WalkSpeed(1.0),
                        crate::mobility::components::Position { x: 16.0, y: 16.0 },
                    ))
                    .id();
                self.world
                    .resource_mut::<crate::mobility::resources::AgentIdIndex>()
                    .0
                    .insert(id, entity);
            }
        }

        /// Advance the sim-clock by exactly one month and run the monthly system.
        fn advance_one_month(&mut self) {
            use crate::mobility::resources::Tick;
            let ticks_per_month = crate::time::SECONDS_PER_MONTH
                / self.world.resource::<crate::time::SimClock>().sim_seconds_per_tick;
            let cur = self.world.resource::<Tick>().0;
            self.world.resource_mut::<Tick>().0 = cur + ticks_per_month;
            self.schedule.run(&mut self.world);
        }

        fn active_agent_count(&self) -> usize {
            self.world
                .resource::<crate::mobility::resources::AgentIdIndex>()
                .0
                .len()
        }
    }

    // ---- Carrying-capacity integration tests ---------------------------------

    #[test]
    fn carrying_capacity_bounds_population_in_a_band() {
        let k = 80.0_f32;
        let overshoot = 1.25_f32;
        let k_hard = (k * overshoot).ceil() as usize; // 100
        let mut h = PopulationTestHarness::new(PopulationConfig {
            carrying_capacity: k,
            capacity_overshoot: overshoot,
            ..PopulationConfig::default()
        });
        h.seed_agents(60, 20..35); // 60 fertile-age agents, below K
        let mut max_n = 0usize;
        for _ in 0..(40 * 12) {
            // 40 sim-years of monthly steps
            h.advance_one_month();
            let n = h.active_agent_count();
            assert!(n > 0, "population must never reach 0 within the band");
            assert!(n <= k_hard, "must never exceed K_hard={k_hard}, got {n}");
            max_n = max_n.max(n);
        }
        assert!(
            h.active_agent_count() >= 40,
            "should settle near K, not collapse"
        );
        assert!(
            max_n >= 70,
            "should grow up toward K from the seed (saw max {max_n})"
        );
    }

    #[test]
    fn zero_capacity_is_unbounded() {
        let mut h = PopulationTestHarness::new(PopulationConfig {
            carrying_capacity: 0.0,
            ..PopulationConfig::default()
        });
        h.seed_agents(60, 20..35);
        for _ in 0..(60 * 12) {
            h.advance_one_month();
        }
        assert!(
            h.active_agent_count() > 60,
            "unbounded schedule should grow above seed"
        );
    }

    // ---- End carrying-capacity tests -----------------------------------------

    /// Birth inherits mother's MarketBinding: a mother with `{9001, 9002}` must
    /// produce a newborn that carries the same binding without recomputing it.
    #[test]
    fn birth_inherits_mother_market_binding() {
        use crate::ids::AgentId;
        use crate::mobility::MarketBinding;
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
        world.insert_resource(PopulationConfig {
            tfr: 100.0,
            ..PopulationConfig::default()
        });
        world.insert_resource(LastProcessedMonth::default());
        world.insert_resource(Tick(0));
        world.insert_resource(AgentIdIndex::default());

        schedule.add_systems(population_monthly_system);

        let ticks_per_year: i64 =
            i64::try_from(crate::time::SECONDS_PER_YEAR / 200).expect("test tick rate fits i64");
        let age_ticks = 28 * ticks_per_year;
        let now_tick = 100 * ticks_per_year;
        let mother_birth_tick = now_tick - age_ticks;

        let mother_id = AgentId("agent:mother:binding".to_string());
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
                // Mother already has a real market binding.
                MarketBinding {
                    home_market: 9001,
                    work_market: 9002,
                },
            ))
            .id();
        world
            .resource_mut::<AgentIdIndex>()
            .0
            .insert(mother_id.clone(), mother_entity);

        let clock = *world.resource::<SimClock>();
        let now_tick_u64 = u64::try_from(now_tick).expect("test tick fits u64");
        let now_month = clock.month_index(now_tick_u64);
        world.resource_mut::<LastProcessedMonth>().0 = now_month - 1;
        world.resource_mut::<Tick>().0 = now_tick_u64;

        let ticks_per_month = crate::time::SECONDS_PER_MONTH / 200;

        // Run up to 20 months — birth is near-certain at TFR=100.
        let mut child_entity_opt: Option<Entity> = None;
        for _iter in 0..20 {
            schedule.run(&mut world);
            let child_entry = {
                let index = world.resource::<AgentIdIndex>();
                index
                    .0
                    .iter()
                    .filter(|(id, _)| **id != mother_id)
                    .map(|(_, entity)| *entity)
                    .next()
            };
            if let Some(e) = child_entry {
                child_entity_opt = Some(e);
                break;
            }
            let cur = world.resource::<Tick>().0;
            world.resource_mut::<Tick>().0 = cur + ticks_per_month;
        }

        let child_entity = child_entity_opt.expect("child must be born within 20 months");

        // Verify parent_id points to mother.
        let parent = world
            .get::<ParentId>(child_entity)
            .expect("child must have ParentId");
        assert_eq!(
            parent.0,
            Some(mother_id.clone()),
            "child parent_id must be mother"
        );

        // Critical: child must inherit EXACTLY the mother's binding without recomputation.
        let binding = world
            .get::<MarketBinding>(child_entity)
            .expect("child must have MarketBinding after birth");
        assert_eq!(
            binding.home_market, 9001,
            "child must inherit mother's home_market exactly"
        );
        assert_eq!(
            binding.work_market, 9002,
            "child must inherit mother's work_market exactly"
        );
    }
}
