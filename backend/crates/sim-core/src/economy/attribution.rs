//! Conservation-exact attribution of the macro's realized consumption/wages onto
//! observed, market-bound citizens. READ-ONLY over economy quantities: it mints
//! and moves NO money, so the `#78` tick audit is unaffected. It only SELECTS
//! which citizens are economically targeted this tick and proves the partition
//! identity `attributed + unobserved == realized`.

/// One market's attribution outcome for a single channel (shopping OR wages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAttribution {
    /// Citizens selected to represent the realized activity, in deterministic order.
    pub attributed: Vec<crate::ids::AgentId>,
    /// `attributed.len() as i64 * per_unit` — the quantity the visible citizens depict.
    pub attributed_amount: i64,
    /// `realized - attributed_amount` — the part no visible citizen depicts.
    /// `>= 0` whenever `realized >= 0` (the only case in practice: realized is a
    /// consumed quantity or wage, both non-negative).
    pub unobserved_amount: i64,
}

/// Select up to `min(realized / per_unit, cap, candidates.len())` citizens from
/// `candidates` (already sorted deterministically by the caller, e.g. by AgentId),
/// each representing `per_unit` units. Pure; no RNG.
///
/// `realized` is the macro's realized quantity (consumed goods, or wage Money),
/// always `>= 0` in practice; `cap` is expected small (a per-market absolute cap).
/// Guarantees `attributed_amount + unobserved_amount == realized` exactly.
pub fn attribute_channel(
    realized: i64,
    per_unit: i64,
    cap: usize,
    candidates: &[crate::ids::AgentId],
) -> ChannelAttribution {
    debug_assert!(realized >= 0, "realized quantity/wage must be non-negative");
    let per_unit = per_unit.max(1);
    // `try_from` instead of `as usize`: saturate rather than silently truncate a
    // large i64 on a 32-bit target (the value is clamped by cap/len below anyway).
    let by_magnitude = usize::try_from((realized / per_unit).max(0)).unwrap_or(usize::MAX);
    let count = by_magnitude.min(cap).min(candidates.len());
    let attributed: Vec<crate::ids::AgentId> = candidates.iter().take(count).cloned().collect();
    let attributed_amount = (count as i64) * per_unit;
    let unobserved_amount = realized - attributed_amount;
    ChannelAttribution {
        attributed,
        attributed_amount,
        unobserved_amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::AgentId;

    fn ids(n: usize) -> Vec<AgentId> {
        (0..n).map(|i| AgentId(format!("agent:walk:{i}"))).collect()
    }

    #[test]
    fn count_is_min_of_magnitude_cap_and_candidates() {
        // realized 9, per_unit 3 → magnitude 3; cap 4; candidates 10 → count 3.
        let c = attribute_channel(9, 3, 4, &ids(10));
        assert_eq!(c.attributed.len(), 3);
        assert_eq!(c.attributed_amount, 9);
        assert_eq!(c.unobserved_amount, 0);
    }

    #[test]
    fn cap_bounds_the_cohort_and_leaves_unobserved_remainder() {
        // realized 100, per_unit 3 → magnitude 33; cap 4 → count 4; 4*3=12 attributed.
        let c = attribute_channel(100, 3, 4, &ids(10));
        assert_eq!(
            c.attributed.len(),
            4,
            "absolute cap, never scales with population"
        );
        assert_eq!(c.attributed_amount, 12);
        assert_eq!(c.unobserved_amount, 88);
        assert_eq!(
            c.attributed_amount + c.unobserved_amount,
            100,
            "conservation identity"
        );
    }

    #[test]
    fn fewer_candidates_than_magnitude_caps_at_candidates() {
        // realized 9, per_unit 3 → magnitude 3, but only 2 observed citizens bound here.
        let c = attribute_channel(9, 3, 4, &ids(2));
        assert_eq!(c.attributed.len(), 2);
        assert_eq!(c.attributed_amount, 6);
        assert_eq!(c.unobserved_amount, 3);
        assert_eq!(c.attributed_amount + c.unobserved_amount, 9);
    }

    #[test]
    fn zero_realized_attributes_nobody() {
        let c = attribute_channel(0, 3, 4, &ids(10));
        assert!(c.attributed.is_empty());
        assert_eq!(c.attributed_amount, 0);
        assert_eq!(c.unobserved_amount, 0);
    }

    #[test]
    fn zero_per_unit_is_treated_as_one() {
        // per_unit <= 0 is clamped to 1 → count = min(6/1, 4, 10) = 4; identity holds.
        let c = attribute_channel(6, 0, 4, &ids(10));
        assert_eq!(c.attributed.len(), 4);
        assert_eq!(c.attributed_amount, 4);
        assert_eq!(c.attributed_amount + c.unobserved_amount, 6);
    }

    #[test]
    fn selection_is_deterministic_prefix() {
        let c = attribute_channel(9, 3, 4, &ids(10));
        assert_eq!(
            c.attributed,
            vec![
                AgentId("agent:walk:0".into()),
                AgentId("agent:walk:1".into()),
                AgentId("agent:walk:2".into())
            ],
        );
    }
}
