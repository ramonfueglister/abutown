use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DirtyMarketGoods, ECONOMY_SCALE, EconomicActorId, EconomyError, EconomyEvent,
    GoodId, InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money,
    NextOrderId, OrderBook, Quantity, TradeLedger, create_ask, create_bid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DemandPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub desired_qty_per_tick: Quantity,
    pub max_price: Money,
    pub urgency_bps: i32,
    pub elasticity_bps: i32,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
    /// Cursor for the consumption sink (`run_consumption_at_tick`). MUST be separate from
    /// `last_generated_tick` (which gates bidding). `Option<u64>: Copy` keeps `DemandPool`
    /// `Copy`; persists for free inside `demand_pools`.
    pub last_consumed_tick: Option<u64>,
    /// Wage Money this household pool received in the PREVIOUS tick (a period FLOW,
    /// not a balance). Zeroed every tick before the wage split accumulates. Drives the
    /// consumption function (Part B). `Money: Copy` keeps `DemandPool` `Copy`; persists
    /// for free in `demand_pools`. Conservation contract: credited ONLY from the `to`
    /// side of a COMPLETED `transfer(HOUSEHOLD_SECTOR, consumer, share)` — never minted.
    pub income_last_tick: Money,
    /// Marginal propensity to consume (basis points, validated `0..=10_000`). Keynesian
    /// `C = autonomous + mpc_bps*income/10_000`. Default 8_000 (0.8). Persisted per-pool.
    pub mpc_bps: i32,
    /// Autonomous (subsistence) consumption spend per tick, financed from wealth. `> 0`
    /// breaks the zero-trap (income=0 ⇒ C=autonomous ⇒ a floor bid keeps the loop alive).
    /// Persisted per-pool.
    pub autonomous: Money,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SupplyPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub offered_qty_per_tick: Quantity,
    pub min_price: Money,
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct DemandPools(pub BTreeMap<EconomicActorId, DemandPool>);

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SupplyPools(pub BTreeMap<EconomicActorId, SupplyPool>);

pub(crate) fn interval_elapsed(last: Option<u64>, current_tick: u64, interval_ticks: u64) -> bool {
    match last {
        None => true,
        Some(last_tick) => current_tick.saturating_sub(last_tick) >= interval_ticks,
    }
}

/// Keynesian consumption target (Money): `C = autonomous + floor(mpc_bps * income / 10_000)`.
/// `mpc_bps` validated `0..=10_000`. i128 intermediate, floor, `try_from` → Overflow.
pub(crate) fn target_spend(
    autonomous: Money,
    mpc_bps: i32,
    income_last_tick: Money,
) -> Result<Money, EconomyError> {
    if !(0..=10_000).contains(&mpc_bps) {
        return Err(EconomyError::InvalidOrder);
    }
    let induced = i64::try_from((income_last_tick.0 as i128) * (mpc_bps as i128) / 10_000)
        .map_err(|_| EconomyError::Overflow)?;
    autonomous.checked_add(Money(induced))
}

/// Map a target SPEND (Money) to a desired Quantity at a reference price, inverting
/// `affordable_qty`'s SCALE math: `qty = floor(spend * ECONOMY_SCALE / p_ref)`.
pub(crate) fn spend_to_qty(spend: Money, p_ref: Money) -> Result<Quantity, EconomyError> {
    if p_ref.0 <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    let raw = (spend.0 as i128) * ECONOMY_SCALE / p_ref.0 as i128;
    Ok(Quantity(
        i64::try_from(raw).map_err(|_| EconomyError::Overflow)?,
    ))
}

/// Part B: rewrite each consumer pool's `desired_qty_per_tick` from its
/// `income_last_tick` (booked by PayWages THIS tick) and the SMOOTHED reference price.
/// The price comes from the market's seeded/traded `ewma_reference_price`; a missing
/// or zero price is an honest `ZeroPrice` error, never a default. Writes a Quantity
/// ONLY; touches no money field. Pure, deterministic, keys-first. `mpc_bps` is
/// validated; `?` surfaces a genuine bug instead of silently freezing a pool's demand.
/// Pre-condition: every consumer pool's `(market, good)` MUST have a `MarketGoodState`
/// with a positive `ewma_reference_price` (seeded at world creation).
pub fn run_consumption_update_at_tick(
    demand: &mut DemandPools,
    market_goods: &MarketGoods,
) -> Result<(), EconomyError> {
    for pool in demand.0.values_mut() {
        let key = MarketGoodKey {
            market: pool.market,
            good: pool.good,
        };
        let spend = target_spend(pool.autonomous, pool.mpc_bps, pool.income_last_tick)?;
        // The reference price is the market's seeded/traded ewma. A missing market-good
        // or a non-positive price is an honest `ZeroPrice` error (propagated), never a
        // default — every consumer market is seeded with an opening price.
        let state = market_goods.0.get(&key).ok_or(EconomyError::ZeroPrice)?;
        pool.desired_qty_per_tick = spend_to_qty(spend, state.ewma_reference_price)?;
    }
    Ok(())
}

pub(crate) fn affordable_qty(cash: Money, price: Money) -> Result<Quantity, EconomyError> {
    if price.0 <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    let raw = (cash.0 as i128)
        .checked_mul(ECONOMY_SCALE)
        .ok_or(EconomyError::Overflow)?
        / price.0 as i128;
    let out = i64::try_from(raw).map_err(|_| EconomyError::Overflow)?;
    Ok(Quantity(out))
}

// All parameters are distinct Bevy ECS resources passed by mut ref from systems.
// Bundling them would require a new struct out of scope for v0. Filled in Task 6/8.
#[allow(clippy::too_many_arguments)]
pub fn generate_pool_orders_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    demand: &mut DemandPools,
    supply: &mut SupplyPools,
    current_tick: u64,
    ttl_ticks: u64,
    dormant: &BTreeSet<MarketId>,
) -> Result<(), EconomyError> {
    let demand_ids: Vec<_> = demand.0.keys().copied().collect();
    for actor in demand_ids {
        let mut pool = demand.0[&actor];
        if dormant.contains(&pool.market) {
            continue; // dormant market: no orders, last_generated_tick untouched
        }
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        let available = accounts.account(actor).available;
        let qty = affordable_qty(available, pool.max_price)?;
        let capped = Quantity(pool.desired_qty_per_tick.0.min(qty.0));
        if capped.0 <= 0 {
            ledger.0.push(EconomyEvent::OrderRejected {
                actor,
                market: pool.market,
                good: pool.good,
                reason: EconomyError::InsufficientFunds,
            });
        } else {
            create_bid(
                accounts,
                orders,
                ledger,
                dirty,
                next,
                current_tick,
                actor,
                pool.market,
                pool.good,
                capped,
                pool.max_price,
                ttl_ticks,
            )?;
        }
        pool.last_generated_tick = Some(current_tick);
        demand.0.insert(actor, pool);
    }

    let supply_ids: Vec<_> = supply.0.keys().copied().collect();
    for actor in supply_ids {
        let mut pool = supply.0[&actor];
        if dormant.contains(&pool.market) {
            continue; // dormant market: no orders, last_generated_tick untouched
        }
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        let available = inventory.balance(actor, pool.good).available;
        let capped = Quantity(pool.offered_qty_per_tick.0.min(available.0));
        if capped.0 <= 0 {
            ledger.0.push(EconomyEvent::OrderRejected {
                actor,
                market: pool.market,
                good: pool.good,
                reason: EconomyError::InsufficientGoods,
            });
        } else {
            create_ask(
                inventory,
                orders,
                ledger,
                dirty,
                next,
                current_tick,
                actor,
                pool.market,
                pool.good,
                capped,
                pool.min_price,
                ttl_ticks,
            )?;
        }
        pool.last_generated_tick = Some(current_tick);
        supply.0.insert(actor, pool);
    }

    Ok(())
}

/// The demand-side SINK (mirror of `run_production_at_tick`): each consumer consumes
/// `min(held available, desired_qty_per_tick)` of its good per interval, emitting a
/// `FinalConsumed` event. Pure, deterministic (keys-first iteration, no clone — `DemandPool`
/// is `Copy`), in-place. The `min` clamp guarantees `qty <= available`, so `consume` can
/// never fault (the `?` is dead-but-typed-for-symmetry). NOT gated on `DormantMarkets`: the
/// sink is an aggregate authority that runs for ALL markets (viewport-independent), so the
/// economy keeps flowing instead of accumulating forever. Conservation: `total_good` drops
/// by exactly `Σ qty`; money is untouched (the consumer already paid on delivery).
/// Also attributes consumption to the market (`MarketGoodState.consumed_qty_last_tick`, the
/// shopper-projection telemetry): zero it on EVERY present market-good first (the sink is its
/// SOLE writer — reset-all avoids phantom carry-over), then accumulate per pool.
pub fn run_consumption_at_tick(
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    demand: &mut DemandPools,
    market_goods: &mut MarketGoods,
    current_tick: u64,
) -> Result<(), EconomyError> {
    for state in market_goods.0.values_mut() {
        state.consumed_qty_last_tick = Quantity::ZERO;
    }
    let actors: Vec<EconomicActorId> = demand.0.keys().copied().collect();
    for actor in actors {
        let pool = demand.0[&actor];
        if !interval_elapsed(pool.last_consumed_tick, current_tick, pool.interval_ticks) {
            continue;
        }
        let available = inventory.balance(actor, pool.good).available;
        let qty = Quantity(pool.desired_qty_per_tick.0.min(available.0));
        if qty.0 > 0 {
            inventory.consume(actor, pool.good, qty)?;
            ledger.0.push(EconomyEvent::FinalConsumed {
                actor,
                good: pool.good,
                qty,
            });
            let key = MarketGoodKey {
                market: pool.market,
                good: pool.good,
            };
            let state = market_goods
                .0
                .entry(key)
                .or_insert_with(|| MarketGoodState::new(key));
            state.consumed_qty_last_tick = state.consumed_qty_last_tick.checked_add(qty)?;
        }
        if let Some(p) = demand.0.get_mut(&actor) {
            p.last_consumed_tick = Some(current_tick);
        }
    }
    Ok(())
}
