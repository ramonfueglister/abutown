//! Firms-as-buyers: producer policies (θ-dividend + working-capital target) and
//! Leontief input pools. Spec: docs/superpowers/specs/2026-06-10-economy-production-chains-design.md
//! Grounding: Caiani et al. (2016) dividend share θ + liquidity buffer;
//! Carvalho & Tahbaz-Salehi (2019) fixed-coefficient input demand.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    EconomicActorId, EconomyError, GoodId, MarketId, Money, Quantity, checked_order_value,
};

/// Authored payout policy per producer. NOT persisted — re-applied from the
/// markets layer at every start (the #83 lesson: config must not silently
/// revert to defaults on restart). Actors absent from this map keep the #75
/// behavior exactly: theta_bps = 10_000, wc_target = 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProducerPolicy {
    pub theta_bps: u16,      // validated 0..=10_000 at seed
    pub batches_target: u32, // validated >= 1 at seed; ONE knob: stock AND cash target
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ProducerPolicies(pub BTreeMap<EconomicActorId, ProducerPolicy>);

/// Leontief input pool: derived demand for one producer's input good at its home
/// market. `max_price` is the participation bound, rewritten every generation pass
/// (§5.4 of the spec); `last_generated_tick` is the only true state (persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub in_qty: Quantity, // recipe input per batch (denormalized from ProductionPools at seed)
    pub out_qty: Quantity, // recipe output per batch (for the participation bound)
    pub out_good: GoodId, // whose reference price bounds the bid
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
    pub max_price: Money, // last computed participation bound (telemetry + order price)
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InputPools(pub BTreeMap<EconomicActorId, InputPool>);

/// Working-capital target in Money: expected input cost of `batches_target` batches
/// at the current participation bound. Computed with the SAME scale arithmetic as
/// order settlement (`checked_order_value`, `price·qty / ECONOMY_SCALE` in i128),
/// so the buffer for n batches equals the settle value of buying them. The batch
/// quantity product is `checked_mul`-guarded; a non-positive `max_price` surfaces
/// loudly as `NegativeMoney`/`ZeroPrice` (a seeded pool's participation bound is
/// validated/recomputed > 0 — anything else is a seed or generation bug).
pub(crate) fn wc_target(policy: ProducerPolicy, pool: &InputPool) -> Result<Money, EconomyError> {
    let batch_qty = i64::from(policy.batches_target)
        .checked_mul(pool.in_qty.0)
        .ok_or(EconomyError::Overflow)?;
    checked_order_value(pool.max_price, Quantity(batch_qty))
}
