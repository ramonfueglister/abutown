use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent,
    GoodId, InventoryBook, MarketGoodKey, MarketGoods, MarketId, Money, NextOrderId, OrderBook,
    OrderId, Quantity, TradeLedger, create_ask, create_bid, transport_cost,
};

/// Reserved account that receives transport-cost payments (keeps money conserved).
pub const TRANSPORT_OPERATOR: EconomicActorId = EconomicActorId(u64::MAX);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraderState {
    Buying { order: Option<OrderId> },
    ToDest { remaining: u64 },
    Selling { order: Option<OrderId> },
    ToSource { remaining: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trader {
    pub actor: EconomicActorId,
    pub good: GoodId,
    pub source: MarketId,
    pub dest: MarketId,
    pub distance_tiles: i64,
    pub batch_qty: Quantity,
    pub buy_premium_bps: i32,
    pub sell_discount_bps: i32,
    pub order_ttl_ticks: u64,
    pub state: TraderState,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Traders(pub BTreeMap<EconomicActorId, Trader>);

/// price * (10000 + bps) / 10000, checked i128. Result must stay positive.
pub fn adjust_price(price: Money, bps: i32) -> Result<Money, EconomyError> {
    let factor = 10_000_i128 + bps as i128;
    if factor <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    let raw = (price.0 as i128)
        .checked_mul(factor)
        .ok_or(EconomyError::Overflow)?
        / 10_000;
    let out = i64::try_from(raw).map_err(|_| EconomyError::Overflow)?;
    if out <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    Ok(Money(out))
}

pub fn transport_ticks(distance_tiles: i64, config: &EconomyConfig) -> u64 {
    let per = config.trader_tiles_per_tick.max(1);
    ((distance_tiles.max(0) as u64) / per).max(1)
}

fn ref_price(
    market_goods: &MarketGoods,
    market: MarketId,
    good: GoodId,
    config: &EconomyConfig,
) -> Money {
    market_goods
        .0
        .get(&MarketGoodKey { market, good })
        .map(|s| s.last_settlement_price)
        .filter(|p| p.0 > 0)
        .unwrap_or(config.trader_default_ref_price)
}

#[allow(clippy::too_many_arguments)]
pub fn run_traders_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    market_goods: &MarketGoods,
    traders: &mut Traders,
    config: &EconomyConfig,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let actors: Vec<EconomicActorId> = traders.0.keys().copied().collect();
    for actor in actors {
        let mut trader = traders.0[&actor].clone();
        match trader.state {
            TraderState::Buying { order } => {
                let held = inventory.balance(actor, trader.good).available;
                if held >= trader.batch_qty {
                    let cost = transport_cost(
                        trader.distance_tiles,
                        trader.batch_qty,
                        config.transport_cost_per_tile_unit,
                    )?;
                    if cost.0 > 0 {
                        accounts.transfer(actor, TRANSPORT_OPERATOR, cost)?;
                        ledger.0.push(EconomyEvent::TransportPaid {
                            actor,
                            amount: cost,
                        });
                    }
                    trader.state = TraderState::ToDest {
                        remaining: transport_ticks(trader.distance_tiles, config),
                    };
                } else {
                    let need = match order {
                        None => true,
                        Some(id) => !orders.bids.contains_key(&id),
                    };
                    if need {
                        let price = adjust_price(
                            ref_price(market_goods, trader.source, trader.good, config),
                            trader.buy_premium_bps,
                        )?;
                        let want = Quantity(trader.batch_qty.0 - held.0);
                        let id = create_bid(
                            accounts,
                            orders,
                            ledger,
                            dirty,
                            next,
                            current_tick,
                            actor,
                            trader.source,
                            trader.good,
                            want,
                            price,
                            trader.order_ttl_ticks,
                        )?;
                        trader.state = TraderState::Buying { order: Some(id) };
                    }
                }
            }
            TraderState::ToDest { remaining } => {
                trader.state = if remaining <= 1 {
                    TraderState::Selling { order: None }
                } else {
                    TraderState::ToDest {
                        remaining: remaining - 1,
                    }
                };
            }
            TraderState::Selling { order } => {
                let held = inventory.balance(actor, trader.good).available;
                if held.0 == 0 {
                    trader.state = TraderState::ToSource {
                        remaining: transport_ticks(trader.distance_tiles, config),
                    };
                } else {
                    let need = match order {
                        None => true,
                        Some(id) => !orders.asks.contains_key(&id),
                    };
                    if need {
                        let price = adjust_price(
                            ref_price(market_goods, trader.dest, trader.good, config),
                            -trader.sell_discount_bps,
                        )?;
                        let id = create_ask(
                            inventory,
                            orders,
                            ledger,
                            dirty,
                            next,
                            current_tick,
                            actor,
                            trader.dest,
                            trader.good,
                            held,
                            price,
                            trader.order_ttl_ticks,
                        )?;
                        trader.state = TraderState::Selling { order: Some(id) };
                    }
                }
            }
            TraderState::ToSource { remaining } => {
                trader.state = if remaining <= 1 {
                    TraderState::Buying { order: None }
                } else {
                    TraderState::ToSource {
                        remaining: remaining - 1,
                    }
                };
            }
        }
        traders.0.insert(actor, trader);
    }
    Ok(())
}
