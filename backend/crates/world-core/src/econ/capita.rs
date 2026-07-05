//! Per-capita scaling factor: economic throughput tracks the live citizen
//! count. `capita_factor = max(1, live_count / capita_baseline)` (integer
//! floor). Default `capita_baseline` keeps the factor at 1 (identity) until
//! deliberately ramped by LOWERING the baseline.
//!
//! Harvested from bbd0159 WITHOUT the `AgentMarker` live-count binding: in M1
//! the live count will come from the `CitizenRegistry` (Task 7). Only the pure
//! arithmetic + the `CapitaFactor` resource live here.

use bevy_ecs::prelude::Resource;

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
/// the identity default — `EconomyConfig::default()` references this. Lower
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
}
