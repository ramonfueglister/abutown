//! Free / market-clearing prices: a damped tâtonnement nudge that moves pool reservation
//! prices toward scarcity. Pure over its refs (no `World`). Conservation-trivial — it writes
//! only i64 price fields and reads telemetry; it NEVER touches money/inventory. Deterministic:
//! i128 intermediates, floor division, keys-first BTreeMap iteration.

use std::collections::BTreeSet;

use crate::economy::{
    DemandPools, EconomyConfig, EconomyError, MarketGoodKey, MarketGoodState, MarketGoods, Money,
    RealizedFlows, SupplyPools,
};

/// Set of `(MarketId.0, GoodId.0)` pairs used to mark flow-coupled pools that the
/// local-unmet tâtonnement should skip (the flow-margin term anchors them instead).
/// Factored out to satisfy `clippy::type_complexity`.
type MarketGoodKeySet = BTreeSet<(u32, u16)>;

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
///
/// `skip_demand` / `skip_supply`: `(market.0, good.0)` pairs governed by the flow-margin term
/// this cadence. Those pools are skipped here so the LoOP margin anchors them and the local
/// tâtonnement doesn't fight the convergence. Pass `&BTreeSet::new()` for both to preserve
/// the pre-Task-3 behaviour (all pools nudged by local signal).
pub fn run_adjust_reservation_prices_at_tick(
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    market_goods: &MarketGoods,
    config: &EconomyConfig,
    skip_demand: &MarketGoodKeySet,
    skip_supply: &MarketGoodKeySet,
) -> Result<(), EconomyError> {
    let k_bps = config.validated_price_adjust_k_bps()?;
    let max_step_bps = config.validated_price_adjust_max_step_bps()?;
    let (floor, ceiling) = config.validated_price_band()?;

    for pool in demand.0.values_mut() {
        if skip_demand.contains(&(pool.market.0, pool.good.0)) {
            continue;
        }
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
        if skip_supply.contains(&(pool.market.0, pool.good.0)) {
            continue;
        }
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

/// Returns `(demand_keys, supply_keys)` — the `(market.0, good.0)` pairs governed by the
/// flow-margin term this cadence (i.e., every dst market/good pair for demand, every src
/// market/good pair for supply). Used by `run_adjust_reservation_prices_at_tick` to skip
/// flow-coupled pools (the margin anchors them; local tâtonnement would fight convergence).
/// Deterministic: BTreeSet insertion order. Conservation-trivial (pure read of realized).
pub fn flow_coupled_keys(realized: &RealizedFlows) -> (MarketGoodKeySet, MarketGoodKeySet) {
    let mut demand_keys = MarketGoodKeySet::new();
    let mut supply_keys = MarketGoodKeySet::new();
    for f in realized.0.iter() {
        demand_keys.insert((f.dst.0, f.good.0));
        supply_keys.insert((f.src.0, f.good.0));
    }
    (demand_keys, supply_keys)
}

/// Anchor flow-coupled (one-sided) markets to the spatial Law of One Price. For each
/// realized flow S→D (good g) this cadence: `target_D = p_src + rate·dist` (the landed
/// cost), `target_S = p_dst − rate·dist`. Nudge D's demand pools' `max_price` toward
/// `target_D` and S's supply pools' `min_price` toward `target_S`, damped/speed-limited/
/// clamped. Fixpoint: `p_D − p_S = rate·dist` (Samuelson, 1952; Takayama & Judge, 1971).
/// Complementarity: only realized (q>0) edges are touched — dormant routes are NOT forced
/// to equality. Conservation-trivial (no money moved). Keys-first/deterministic.
pub fn run_flow_margin_feedback_at_tick(
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    realized: &RealizedFlows,
    config: &EconomyConfig,
) -> Result<(), EconomyError> {
    let k_bps = config.validated_price_adjust_k_bps()?;
    let max_step_bps = config.validated_price_adjust_max_step_bps()?;
    let (floor, ceiling) = config.validated_price_band()?;
    let rate = config.transport_cost_per_tile_unit.0 as i128;

    for f in realized.0.iter() {
        let t = rate * f.dist as i128;
        let target_d = Money(
            i64::try_from((f.p_src.0 as i128 + t).clamp(floor as i128, ceiling as i128))
                .map_err(|_| EconomyError::Overflow)?,
        );
        let target_s = Money(
            i64::try_from((f.p_dst.0 as i128 - t).clamp(floor as i128, ceiling as i128))
                .map_err(|_| EconomyError::Overflow)?,
        );
        for pool in demand.0.values_mut() {
            if pool.market == f.dst && pool.good == f.good {
                pool.max_price = nudge_price_toward_target(
                    pool.max_price,
                    target_d,
                    k_bps,
                    max_step_bps,
                    floor,
                    ceiling,
                )?;
            }
        }
        for pool in supply.0.values_mut() {
            if pool.market == f.src && pool.good == f.good {
                pool.min_price = nudge_price_toward_target(
                    pool.min_price,
                    target_s,
                    k_bps,
                    max_step_bps,
                    floor,
                    ceiling,
                )?;
            }
        }
    }
    Ok(())
}
