//! Geometry-free economy core, harvested from sim-core's `economy/` at commit
//! `bbd0159` (see docs/superpowers/plans/2026-07-05-mmorpg-m1-persistent-world.md,
//! Task 4). Deliberately NOT harvested: attribution, materialize, trader_render,
//! systems (rebuilt leaner in Task 6), markets_layer (Task 5 seeds from
//! `economy.json`), persist (Task 10), transport's graph coupling (distance is
//! [`euclid_m`] over `MarketSite` meters).
//!
//! M1 LOD decision: markets are NEVER dormant (small market count, no
//! chunk/LOD machinery). The old `MarketChunks`/`DormantMarkets` resources and
//! the chunk-derived refresh system are gone; the macro-flow core keeps its
//! plain `&BTreeSet<MarketId>` dormant parameter (pure set arithmetic, no chunk
//! concepts) and M1's system chain always passes an EMPTY set.

pub mod accounts;
pub mod auction;
pub mod audit;
pub mod capita;
pub mod config;
pub mod flow_shipments;
pub mod flow_telemetry;
pub mod goods;
pub mod ids;
pub mod inventory;
pub mod ledger;
pub mod macro_flow;
pub mod market;
pub mod money;
pub mod orders;
pub mod pools;
pub mod pricing;
pub mod producers;
pub mod production;
pub mod wages;

pub use accounts::*;
pub use auction::*;
pub use audit::*;
pub use capita::{CAPITA_BASELINE_IDENTITY, CapitaFactor, capita_factor};
pub use config::EconomyConfig;
pub use flow_shipments::*;
pub use flow_telemetry::{FlowRateEwma, update_flow_rate_ewma};
pub use goods::*;
pub use ids::*;
pub use inventory::*;
pub use ledger::*;
pub use macro_flow::*;
pub use market::*;
pub use money::*;
pub use orders::*;
pub use pools::*;
pub use pricing::*;
pub use producers::*;
pub use production::*;
pub use wages::*;

/// Reserved account that receives transport-cost payments (keeps money conserved).
pub const TRANSPORT_OPERATOR: EconomicActorId = EconomicActorId(u64::MAX);

/// Euclidean distance in whole meters between two local-meter positions,
/// rounded to the nearest integer. Replaces the old graph-based
/// `manhattan_tiles` (transport.rs, NOT harvested): the M1 world model anchors
/// markets at real `(x, z)` meters, so distance is plain geometry. `f64`
/// intermediates + `round()` keep the result identical on every platform for
/// the coordinate magnitudes of a municipality (< 2^24 m).
pub fn euclid_m(a: (f32, f32), b: (f32, f32)) -> i64 {
    let dx = a.0 as f64 - b.0 as f64;
    let dz = a.1 as f64 - b.1 as f64;
    (dx * dx + dz * dz).sqrt().round() as i64
}

/// Cost to move `qty` of a good across `distance_m` at `rate` (Money per
/// meter per unit; the config field keeps its harvested name
/// `transport_cost_per_tile_unit`). Reuses the fixed-point order-value helper;
/// all checked i128. Rate must be > 0 (v0 requirement). Returns
/// `EconomyError::InvalidOrder` for negative distances and
/// `EconomyError::Overflow` on arithmetic overflow.
pub fn transport_cost(distance_m: i64, qty: Quantity, rate: Money) -> Result<Money, EconomyError> {
    if distance_m < 0 {
        return Err(EconomyError::InvalidOrder);
    }
    if distance_m == 0 {
        return Ok(Money(0));
    }
    let per_unit_distance = money::checked_order_value(rate, qty)?;
    let raw = (per_unit_distance.0 as i128)
        .checked_mul(distance_m as i128)
        .ok_or(EconomyError::Overflow)?;
    i64::try_from(raw)
        .map(Money)
        .map_err(|_| EconomyError::Overflow)
}

/// Recycle the accumulated `TRANSPORT_OPERATOR` balance back to the labor households (the
/// buyers paid the transport fee, so it returns to them). Drains the ENTIRE operator
/// balance → `HOUSEHOLD_SECTOR`, then `apportion_cash` over the SAME `pool_weights` as
/// wages → households, crediting `income_last_tick` (ADD). Emits `TransportRebate` with
/// the drained amount. Conservation: only `transfer`s ⇒ `total_money` byte-invariant; own
/// independent `HOUSEHOLD_SECTOR` net-zero `debug_assert`. The macro-flow-interval gating
/// (phase-locked to the operator CREDIT in macro_flow, no persisted cursor) is applied in
/// the system wrapper, NOT here — this function always drains whatever is present.
pub fn run_transport_rebate_at_tick(
    accounts: &mut AccountBook,
    demand: &mut DemandPools,
    household: &HouseholdSector,
    ledger: &mut TradeLedger,
) -> Result<(), EconomyError> {
    let amount = accounts.account(TRANSPORT_OPERATOR).available;
    if amount.0 <= 0 {
        return Ok(());
    }
    let payees: Vec<EconomicActorId> = household.pool_weights.keys().copied().collect();
    let weights: Vec<i64> = household.pool_weights.values().copied().collect();
    let weight_sum: i128 = weights.iter().map(|w| *w as i128).sum();
    if weight_sum <= 0 {
        return Ok(()); // no payout target: leave the fee in the operator (never strand it elsewhere)
    }

    // LEG 1: operator → HOUSEHOLD_SECTOR (the operator holds exactly `amount`).
    accounts.transfer(TRANSPORT_OPERATOR, HOUSEHOLD_SECTOR, amount)?;
    // LEG 2: HOUSEHOLD_SECTOR → households (sum-preserving).
    let splits = apportion_cash(&weights, amount.0);
    for (idx, actor) in payees.iter().enumerate() {
        let share = Money(splits[idx]);
        if share.0 <= 0 {
            continue;
        }
        let pool = demand
            .0
            .get_mut(actor)
            .expect("pool_weights key must reference a seeded demand pool");
        accounts.transfer(HOUSEHOLD_SECTOR, *actor, share)?;
        pool.income_last_tick = pool.income_last_tick.checked_add(share)?;
    }
    ledger.0.push(EconomyEvent::TransportRebate { amount });

    debug_assert_eq!(
        accounts.account(HOUSEHOLD_SECTOR).available,
        Money::ZERO,
        "HOUSEHOLD_SECTOR must net to zero after transport rebate (sentinel-stranded cash)"
    );
    Ok(())
}

#[cfg(test)]
mod euclid_tests {
    use super::euclid_m;

    #[test]
    fn euclid_m_pythagorean_triple() {
        assert_eq!(euclid_m((0.0, 0.0), (3.0, 4.0)), 5);
        assert_eq!(euclid_m((3.0, 4.0), (0.0, 0.0)), 5, "symmetric");
    }

    #[test]
    fn euclid_m_zero_distance() {
        assert_eq!(euclid_m((12.5, -7.25), (12.5, -7.25)), 0);
    }

    #[test]
    fn euclid_m_rounds_to_nearest_meter() {
        // sqrt(2) ≈ 1.414 → 1; sqrt(8) ≈ 2.828 → 3.
        assert_eq!(euclid_m((0.0, 0.0), (1.0, 1.0)), 1);
        assert_eq!(euclid_m((0.0, 0.0), (2.0, 2.0)), 3);
    }

    #[test]
    fn euclid_m_municipality_scale() {
        // 2.5 km straight line stays exact.
        assert_eq!(euclid_m((-1250.0, 0.0), (1250.0, 0.0)), 2500);
    }
}

#[cfg(test)]
mod tests;
