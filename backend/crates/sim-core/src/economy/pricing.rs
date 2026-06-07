//! Free / market-clearing prices: a damped tâtonnement nudge that moves pool reservation
//! prices toward scarcity. Pure over its refs (no `World`). Conservation-trivial — it writes
//! only i64 price fields and reads telemetry; it NEVER touches money/inventory. Deterministic:
//! i128 intermediates, floor division, keys-first BTreeMap iteration.

use crate::economy::{
    DemandPools, EconomyConfig, EconomyError, MarketGoodKey, MarketGoodState, MarketGoods, Money,
    SupplyPools,
};

/// Normalized excess-demand intensity for one market-good, in basis points ∈ [-10_000, +10_000].
/// `net = unmet − unsold`; `scale = max(1, unmet + unsold)`; `x = net*10_000/scale`. Since
/// `|net| <= unmet+unsold = scale`, `|x| <= 10_000`. i128, floor (truncates toward zero).
fn intensity_bps(state: &MarketGoodState) -> i128 {
    let unmet = state.unmet_demand_last_tick.0 as i128;
    let unsold = state.unsold_supply_last_tick.0 as i128;
    let net = unmet - unsold;
    let scale = (unmet + unsold).max(1);
    (net * 10_000) / scale
}

/// Nudge one reservation `price` by the market's scarcity intensity, speed-limited and clamped.
/// `step_bps = clamp(k_bps * x_bps / 10_000, ±max_step_bps)`; `new = price + price*step/10_000`,
/// then clamped into `[floor, ceiling]`. Shortage (x>0) raises, glut (x<0) lowers. Checked i128.
fn nudge_price(
    price: Money,
    state: &MarketGoodState,
    k_bps: i128,
    max_step_bps: i128,
    floor: i64,
    ceiling: i64,
) -> Result<Money, EconomyError> {
    let x_bps = intensity_bps(state);
    let step_bps = ((k_bps * x_bps) / 10_000).clamp(-max_step_bps, max_step_bps);
    let delta = ((price.0 as i128) * step_bps) / 10_000;
    let raw = (price.0 as i128) + delta;
    let clamped = raw.clamp(floor as i128, ceiling as i128);
    Ok(Money(
        i64::try_from(clamped).map_err(|_| EconomyError::Overflow)?,
    ))
}

/// Nudge `price` toward a spatial-equilibrium `target` (Law of One Price: target =
/// source_price + rate·dist). Signed, speed-limited, clamped — the inter-market
/// arbitrage term that anchors a one-sided market's price (Samuelson, 1952; Takayama
/// & Judge, 1971). `step_bps = clamp(k_bps · gap_bps / 10_000, ±max_step_bps)` where
/// `gap_bps = (target − price)·10_000 / max(1, price)`; `new = price + price·step/10_000`,
/// clamped into `[floor, ceiling]`. Above target → pulls down (the recovery force a
/// pure-sink's local-unmet term lacks); below → pulls up. Conservation-trivial
/// (writes no money). Checked i128, floor.
pub fn nudge_price_toward_target(
    price: Money,
    target: Money,
    k_bps: i128,
    max_step_bps: i128,
    floor: i64,
    ceiling: i64,
) -> Result<Money, EconomyError> {
    let p = price.0 as i128;
    let denom = p.max(1);
    let gap_bps = ((target.0 as i128 - p) * 10_000) / denom;
    let step_bps = ((k_bps * gap_bps) / 10_000).clamp(-max_step_bps, max_step_bps);
    let delta = (p * step_bps) / 10_000;
    let raw = p + delta;
    let clamped = raw.clamp(floor as i128, ceiling as i128);
    Ok(Money(
        i64::try_from(clamped).map_err(|_| EconomyError::Overflow)?,
    ))
}

/// For every demand pool, nudge `max_price`; for every supply pool, nudge `min_price` — each by
/// the excess-demand signal of ITS OWN `(market, good)` state (shortage→up, glut→down: both walls
/// translate the same direction). A pool whose `(market, good)` has no `MarketGoodState` yet (a
/// market that has never cleared) has NO scarcity signal this interval, so its price correctly
/// stays put — this is "no data, no action", NOT a defaulted price. Keys-first (BTreeMap) → deterministic.
pub fn run_adjust_reservation_prices_at_tick(
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    market_goods: &MarketGoods,
    config: &EconomyConfig,
) -> Result<(), EconomyError> {
    let k_bps = config.validated_price_adjust_k_bps()?;
    let max_step_bps = config.validated_price_adjust_max_step_bps()?;
    let (floor, ceiling) = config.validated_price_band()?;

    for pool in demand.0.values_mut() {
        let key = MarketGoodKey {
            market: pool.market,
            good: pool.good,
        };
        if let Some(state) = market_goods.0.get(&key) {
            pool.max_price =
                nudge_price(pool.max_price, state, k_bps, max_step_bps, floor, ceiling)?;
        }
    }
    for pool in supply.0.values_mut() {
        let key = MarketGoodKey {
            market: pool.market,
            good: pool.good,
        };
        if let Some(state) = market_goods.0.get(&key) {
            pool.min_price =
                nudge_price(pool.min_price, state, k_bps, max_step_bps, floor, ceiling)?;
        }
    }
    Ok(())
}
