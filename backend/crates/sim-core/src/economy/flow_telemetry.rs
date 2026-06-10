//! In-memory (never persisted) EWMA of realized macro-flow quantities per
//! (src, dst, good) edge. Powers the on-wire EconomySnapshot.flows field.
//! Deliberately NOT part of EconomyPersistSnapshot: a restart reconverges in
//! a few intervals, and persisting it would force an economy_snapshots wipe.

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;

use super::money::integer_ewma;
use super::{GoodId, MarketId, Money, RealizedFlows};

pub type FlowKey = (MarketId, MarketId, GoodId);

/// Smoothing weight for new observations, in basis points.
pub const FLOW_RATE_ALPHA_BPS: u16 = 3_000;

#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowRateEwma(pub BTreeMap<FlowKey, Money>);

/// Fold the current interval's realized flows into the EWMA. Edges that shipped
/// nothing decay toward zero and are dropped once they reach it.
pub fn update_flow_rate_ewma(ewma: &mut FlowRateEwma, realized: &RealizedFlows) {
    let mut current: BTreeMap<FlowKey, i64> = BTreeMap::new();
    for flow in &realized.0 {
        *current.entry((flow.src, flow.dst, flow.good)).or_insert(0) += flow.qty;
    }
    let keys: std::collections::BTreeSet<FlowKey> = ewma
        .0
        .keys()
        .copied()
        .chain(current.keys().copied())
        .collect();
    for key in keys {
        let old = ewma.0.get(&key).copied().unwrap_or(Money(0));
        let cur = Money(current.get(&key).copied().unwrap_or(0));
        let next = integer_ewma(old, cur, FLOW_RATE_ALPHA_BPS)
            .expect("flow ewma: alpha is a valid const and qty magnitudes cannot overflow i128");
        if next.0 <= 0 {
            ewma.0.remove(&key);
        } else {
            ewma.0.insert(key, next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::{GOOD_FOOD, RealizedFlow};

    fn realized(entries: &[(u32, u32, i64)]) -> RealizedFlows {
        RealizedFlows(
            entries
                .iter()
                .map(|&(src, dst, qty)| RealizedFlow {
                    src: MarketId(src),
                    dst: MarketId(dst),
                    good: GOOD_FOOD,
                    qty,
                    p_src: Money(0),
                    p_dst: Money(0),
                    dist: 1,
                })
                .collect(),
        )
    }

    #[test]
    fn first_observation_is_alpha_weighted() {
        let mut ewma = FlowRateEwma::default();
        update_flow_rate_ewma(&mut ewma, &realized(&[(1, 2, 1000)]));
        // 0.3 * 1000 = 300
        assert_eq!(ewma.0[&(MarketId(1), MarketId(2), GOOD_FOOD)], Money(300));
    }

    #[test]
    fn same_edge_entries_sum_before_smoothing() {
        let mut ewma = FlowRateEwma::default();
        update_flow_rate_ewma(&mut ewma, &realized(&[(1, 2, 600), (1, 2, 400)]));
        assert_eq!(ewma.0[&(MarketId(1), MarketId(2), GOOD_FOOD)], Money(300));
    }

    #[test]
    fn idle_edges_decay_and_are_eventually_dropped() {
        let mut ewma = FlowRateEwma::default();
        update_flow_rate_ewma(&mut ewma, &realized(&[(1, 2, 10)]));
        assert!(ewma.0.contains_key(&(MarketId(1), MarketId(2), GOOD_FOOD)));
        for _ in 0..64 {
            update_flow_rate_ewma(&mut ewma, &realized(&[]));
        }
        assert!(
            ewma.0.is_empty(),
            "decayed-to-zero edges must be dropped, got {:?}",
            ewma.0
        );
    }
}
