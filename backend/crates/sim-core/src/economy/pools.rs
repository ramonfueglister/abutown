use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DirtyMarketGoods, ECONOMY_SCALE, EconomicActorId, EconomyError, EconomyEvent,
    GoodId, InventoryBook, MarketId, Money, NextOrderId, OrderBook, Quantity, TradeLedger,
    create_ask, create_bid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn affordable_qty(cash: Money, price: Money) -> Result<Quantity, EconomyError> {
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
