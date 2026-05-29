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

/// Gompertz–Makeham monthly mortality system.
///
/// Implemented as an exclusive system (`&mut World`) so that it has full
/// access to all world data in a single borrow.  Runs in
/// `MobilitySet::Bookkeeping` (after the Advance pass, before EventEmit)
/// so despawns happen in a clean bookkeeping phase and don't interfere
/// with mid-tick movement logic.
///
/// Two-phase approach: collect victims into a Vec first (via `AgentIdIndex`),
/// then despawn and evict the index entry.  Collecting from the index avoids
/// query-state caching issues and keeps the borrow checker happy because the
/// resource borrow is released before `world.despawn` is called.
pub fn mortality_system(world: &mut World) {
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

    // Phase 1: collect (entity, agent_id) pairs from the index so we don't
    // hold the resource borrow across the despawn calls below.
    let agent_entries: Vec<(crate::ids::AgentId, Entity)> = world
        .resource::<crate::mobility::resources::AgentIdIndex>()
        .0
        .iter()
        .map(|(id, e)| (id.clone(), *e))
        .collect();

    // Phase 2: determine which agents die in each new month.
    let mut victims: Vec<(Entity, crate::ids::AgentId)> = Vec::new();
    for m in (last + 1)..=current_month {
        for (agent_id, entity) in &agent_entries {
            // Skip entities that were already despawned earlier this loop
            // (e.g. by another system before mortality ran, or by a previous
            // month's iteration above).
            let Some(birth_tick) = world.get::<crate::mobility::components::BirthTick>(*entity)
            else {
                continue;
            };
            let age = clock.age_years(now_tick, birth_tick.0);
            let draw = unit_draw(stable_agent_hash(agent_id), m, 0);
            if draw < death_probability_month(age, &cfg) {
                victims.push((*entity, agent_id.clone()));
            }
        }
    }

    // Phase 3: despawn victims and evict from AgentIdIndex eagerly so the
    // rest of the tick sees the correct agent count without waiting for the
    // post-schedule tick_mobility sync pass.
    for (entity, agent_id) in victims {
        world.despawn(entity);
        world
            .resource_mut::<crate::mobility::resources::AgentIdIndex>()
            .0
            .remove(&agent_id);
    }

    world.resource_mut::<LastProcessedMonth>().0 = current_month;
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
        // Run mortality in MobilitySet::Bookkeeping — same phase as
        // tick_increment_system, after Advance and Output, so despawns
        // happen after all movement logic is done for this tick.
        schedule.add_systems(
            mortality_system
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
}
