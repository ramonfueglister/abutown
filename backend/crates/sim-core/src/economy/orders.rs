use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    checked_order_value, AccountBook, DirtyMarketGoods, EconomicActorId, EconomyError,
    EconomyEvent, GoodId, InventoryBook, MarketGoodKey, MarketId, Money, OrderId, Quantity,
    TradeLedger,
};

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextOrderId(pub u64);

impl NextOrderId {
    pub fn next(&mut self) -> OrderId {
        self.0 += 1;
        OrderId(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bid {
    pub id: OrderId,
    pub owner: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub qty_remaining: Quantity,
    pub max_price: Money,
    pub cash_locked_remaining: Money,
    pub created_tick: u64,
    pub expires_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub id: OrderId,
    pub owner: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub qty_remaining: Quantity,
    pub min_price: Money,
    pub goods_locked_remaining: Quantity,
    pub created_tick: u64,
    pub expires_tick: u64,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct OrderBook {
    pub bids: BTreeMap<OrderId, Bid>,
    pub asks: BTreeMap<OrderId, Ask>,
}

pub fn create_bid(
    accounts: &mut AccountBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    current_tick: u64,
    owner: EconomicActorId,
    market: MarketId,
    good: GoodId,
    qty: Quantity,
    max_price: Money,
    ttl_ticks: u64,
) -> Result<OrderId, EconomyError> {
    let locked = checked_order_value(max_price, qty)?;
    accounts.lock_cash(owner, locked)?;
    let id = next.next();
    orders.bids.insert(
        id,
        Bid {
            id,
            owner,
            market,
            good,
            qty_remaining: qty,
            max_price,
            cash_locked_remaining: locked,
            created_tick: current_tick,
            expires_tick: current_tick.saturating_add(ttl_ticks),
        },
    );
    dirty.0.insert(MarketGoodKey { market, good });
    ledger.0.push(EconomyEvent::OrderCreated { order: id, actor: owner, market, good });
    ledger.0.push(EconomyEvent::CashLocked { actor: owner, amount: locked });
    Ok(id)
}

pub fn create_ask(
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    current_tick: u64,
    owner: EconomicActorId,
    market: MarketId,
    good: GoodId,
    qty: Quantity,
    min_price: Money,
    ttl_ticks: u64,
) -> Result<OrderId, EconomyError> {
    if min_price.0 <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    inventory.lock_goods(owner, good, qty)?;
    let id = next.next();
    orders.asks.insert(
        id,
        Ask {
            id,
            owner,
            market,
            good,
            qty_remaining: qty,
            min_price,
            goods_locked_remaining: qty,
            created_tick: current_tick,
            expires_tick: current_tick.saturating_add(ttl_ticks),
        },
    );
    dirty.0.insert(MarketGoodKey { market, good });
    ledger.0.push(EconomyEvent::OrderCreated { order: id, actor: owner, market, good });
    ledger.0.push(EconomyEvent::GoodsLocked { actor: owner, good, qty });
    Ok(id)
}

pub fn expire_orders_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    current_tick: u64,
) -> Result<(), EconomyError> {
    let expired_bids: Vec<_> = orders
        .bids
        .iter()
        .filter_map(|(id, bid)| (current_tick >= bid.expires_tick).then_some((*id, bid.clone())))
        .collect();
    for (id, bid) in expired_bids {
        accounts.release_cash(bid.owner, bid.cash_locked_remaining)?;
        dirty.0.insert(MarketGoodKey { market: bid.market, good: bid.good });
        ledger.0.push(EconomyEvent::OrderExpired { order: id, actor: bid.owner, market: bid.market, good: bid.good });
        ledger.0.push(EconomyEvent::CashReleased { actor: bid.owner, amount: bid.cash_locked_remaining });
        orders.bids.remove(&id);
    }

    let expired_asks: Vec<_> = orders
        .asks
        .iter()
        .filter_map(|(id, ask)| (current_tick >= ask.expires_tick).then_some((*id, ask.clone())))
        .collect();
    for (id, ask) in expired_asks {
        inventory.release_goods(ask.owner, ask.good, ask.goods_locked_remaining)?;
        dirty.0.insert(MarketGoodKey { market: ask.market, good: ask.good });
        ledger.0.push(EconomyEvent::OrderExpired { order: id, actor: ask.owner, market: ask.market, good: ask.good });
        ledger.0.push(EconomyEvent::GoodsReleased { actor: ask.owner, good: ask.good, qty: ask.goods_locked_remaining });
        orders.asks.remove(&id);
    }

    Ok(())
}
