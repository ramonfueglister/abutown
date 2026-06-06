//! Per-capita scaling factor: economic throughput (and the visible attribution
//! cohort) track the live citizen count. `capita_factor = max(1, live_count /
//! capita_baseline)` (integer floor). Default `capita_baseline` keeps the factor
//! at 1 (identity) until deliberately ramped by LOWERING the baseline.

use bevy_ecs::prelude::Resource;
use bevy_ecs::world::World;

/// Per-tick scaling multiplier applied to real-quantity flows + cohort caps.
/// `1` = identity (byte-identical to the un-scaled economy).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapitaFactor(pub i64);

impl Default for CapitaFactor {
    fn default() -> Self {
        CapitaFactor(1)
    }
}

/// The identity `capita_baseline`: large enough that `capita_factor` stays at `1`
/// (the un-scaled economy) at realistic citizen counts. Single source of truth for
/// the identity default — `EconomyConfig::default()` and the `markets.json`
/// serde-default (`base_world::default_capita_baseline`) both reference this. Lower
/// the authored baseline to ramp the factor up.
pub const CAPITA_BASELINE_IDENTITY: i64 = 1_000_000;

/// Derive the factor from the live citizen count and the configured baseline.
/// Floor division, clamped to `>= 1`. `capita_baseline <= 0` is treated as the
/// neutral 1 (never divide by zero, never invert the meaning).
pub fn capita_factor(live_count: u64, capita_baseline: i64) -> i64 {
    if capita_baseline <= 0 {
        return 1;
    }
    let raw = (live_count as i128) / (capita_baseline as i128);
    i64::try_from(raw).unwrap_or(i64::MAX).max(1)
}

/// Exclusive system (EconomySet::RefreshCapita, FIRST in the chain). Derives
/// `CapitaFactor` from the live citizen count each tick. The count only changes at
/// monthly birth/death boundaries; reading it every tick is equivalent to a monthly
/// snapshot — simpler, no month-gating state, fully deterministic.
///
/// No-ops cleanly when either `CapitaFactor` or `EconomyConfig` is absent (e.g.
/// economy-only worlds under test that have not installed the full mobility stack).
pub fn refresh_capita_factor_system(world: &mut World) {
    use crate::economy::systems::EconomyConfig;
    use crate::mobility::components::AgentMarker;
    use bevy_ecs::prelude::With;

    if world.get_resource::<CapitaFactor>().is_none()
        || world.get_resource::<EconomyConfig>().is_none()
    {
        return;
    }

    // Count live citizens. Query borrow is released (via collect) before the
    // resource writes below, keeping the borrow checker happy.
    let live: u64 = {
        let mut q = world.query_filtered::<(), With<AgentMarker>>();
        q.iter(world).count() as u64
    };

    let baseline = world.resource::<EconomyConfig>().capita_baseline;
    let f = capita_factor(live, baseline);
    world.resource_mut::<CapitaFactor>().0 = f;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_baseline_exceeds_count() {
        assert_eq!(capita_factor(300, 1_000_000), 1);
    }
    #[test]
    fn scales_up_when_baseline_lowered() {
        assert_eq!(capita_factor(300, 10), 30);
        assert_eq!(capita_factor(1000, 10), 100);
    }
    #[test]
    fn floor_and_min_one() {
        assert_eq!(capita_factor(9, 10), 1, "floor(0.9)=0 clamped to 1");
        assert_eq!(capita_factor(0, 10), 1, "zero citizens still 1, never 0");
    }
    #[test]
    fn nonpositive_baseline_is_neutral() {
        assert_eq!(capita_factor(300, 0), 1);
        assert_eq!(capita_factor(300, -5), 1);
    }

    // ── refresh_capita_factor_system tests ────────────────────────────────────

    /// No-op safety: system returns without panic when CapitaFactor is absent.
    #[test]
    fn refresh_noop_when_capita_factor_absent() {
        let mut world = World::new();
        // No CapitaFactor, no EconomyConfig inserted.
        refresh_capita_factor_system(&mut world); // must not panic
    }

    /// No-op safety: system returns without panic when EconomyConfig is absent.
    #[test]
    fn refresh_noop_when_economy_config_absent() {
        let mut world = World::new();
        world.insert_resource(CapitaFactor::default());
        // EconomyConfig intentionally absent.
        refresh_capita_factor_system(&mut world); // must not panic
        assert_eq!(world.resource::<CapitaFactor>().0, 1, "factor unchanged");
    }

    /// Basic case: N AgentMarker entities → factor == capita_factor(N, baseline).
    #[test]
    fn refresh_derives_factor_from_agent_count() {
        use crate::economy::systems::EconomyConfig;
        use crate::mobility::components::AgentMarker;

        let mut world = World::new();
        world.insert_resource(CapitaFactor::default());
        world.insert_resource(EconomyConfig {
            capita_baseline: 10,
            ..EconomyConfig::default()
        });

        // Spawn 50 AgentMarker entities.
        for _ in 0..50 {
            world.spawn(AgentMarker);
        }

        refresh_capita_factor_system(&mut world);
        let expected = capita_factor(50, 10); // floor(50/10) = 5, clamped >= 1 → 5
        assert_eq!(world.resource::<CapitaFactor>().0, expected);
    }

    /// Factor steps up when more citizens are added: proves it tracks the live count.
    #[test]
    fn refresh_tracks_count_increment() {
        use crate::economy::systems::EconomyConfig;
        use crate::mobility::components::AgentMarker;

        let mut world = World::new();
        world.insert_resource(CapitaFactor::default());
        world.insert_resource(EconomyConfig {
            capita_baseline: 10,
            ..EconomyConfig::default()
        });

        // First batch: 30 citizens → factor = floor(30/10) = 3.
        for _ in 0..30 {
            world.spawn(AgentMarker);
        }
        refresh_capita_factor_system(&mut world);
        assert_eq!(world.resource::<CapitaFactor>().0, capita_factor(30, 10));

        // Second batch: add 70 more → 100 total → factor = floor(100/10) = 10.
        for _ in 0..70 {
            world.spawn(AgentMarker);
        }
        refresh_capita_factor_system(&mut world);
        assert_eq!(world.resource::<CapitaFactor>().0, capita_factor(100, 10));
        assert!(
            world.resource::<CapitaFactor>().0 > 3,
            "factor must increase when citizens are added"
        );
    }

    /// Determinism: same world → same factor on repeated calls.
    #[test]
    fn refresh_is_deterministic() {
        use crate::economy::systems::EconomyConfig;
        use crate::mobility::components::AgentMarker;

        let mut world = World::new();
        world.insert_resource(CapitaFactor::default());
        world.insert_resource(EconomyConfig {
            capita_baseline: 10,
            ..EconomyConfig::default()
        });
        for _ in 0..25 {
            world.spawn(AgentMarker);
        }

        refresh_capita_factor_system(&mut world);
        let first = world.resource::<CapitaFactor>().0;
        refresh_capita_factor_system(&mut world);
        let second = world.resource::<CapitaFactor>().0;
        assert_eq!(first, second, "repeated calls produce identical factor");
    }

    /// Identity at default baseline (~300 citizens): factor stays 1.
    #[test]
    fn refresh_identity_at_default_baseline() {
        use crate::economy::systems::EconomyConfig;
        use crate::mobility::components::AgentMarker;

        let mut world = World::new();
        world.insert_resource(CapitaFactor::default());
        world.insert_resource(EconomyConfig::default()); // capita_baseline = 1_000_000

        for _ in 0..300 {
            world.spawn(AgentMarker);
        }

        refresh_capita_factor_system(&mut world);
        assert_eq!(
            world.resource::<CapitaFactor>().0,
            1,
            "factor is identity (1) at ~300 citizens with baseline 1_000_000"
        );
    }

    /// Worlds without AgentMarker entities yield live=0 → factor=1 (clamped).
    #[test]
    fn refresh_zero_agents_yields_factor_one() {
        use crate::economy::systems::EconomyConfig;

        let mut world = World::new();
        world.insert_resource(CapitaFactor::default());
        world.insert_resource(EconomyConfig {
            capita_baseline: 10,
            ..EconomyConfig::default()
        });
        // No AgentMarker entities spawned.
        refresh_capita_factor_system(&mut world);
        assert_eq!(
            world.resource::<CapitaFactor>().0,
            1,
            "zero citizens → factor clamped to 1"
        );
    }
}
