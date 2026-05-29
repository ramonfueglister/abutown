//! Phase 8l (slice 1): per-agent birth/death for active agents. Aggregate cohort
//! and tracked-lineage are later slices. Deterministic + replay-safe.
use bevy_ecs::prelude::Resource;

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
        let _ = schedule; // mortality + fertility systems added in Tasks 3 & 4
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
}
