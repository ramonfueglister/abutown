use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, EconomyError, EconomyEvent, GoodId,
    InventoryBook, MarketGoodKey, MarketId, Money, OrderId, Quantity, TradeLedger,
    checked_order_value,
};

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextOrderId(pub u64);

impl NextOrderId {
    // Not an Iterator; `next` is an ID counter. Suppress iterator-trait confusion lint.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> OrderId {
        self.0 += 1;
        OrderId(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

// All parameters are distinct resources — cannot bundle further without introducing a
// builder struct that is out of scope for v0. Filled in Task 6/8.
#[allow(clippy::too_many_arguments)]
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
    ledger.0.push(EconomyEvent::OrderCreated {
        order: id,
        actor: owner,
        market,
        good,
    });
    ledger.0.push(EconomyEvent::CashLocked {
        actor: owner,
        amount: locked,
    });
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
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
    ledger.0.push(EconomyEvent::OrderCreated {
        order: id,
        actor: owner,
        market,
        good,
    });
    ledger.0.push(EconomyEvent::GoodsLocked {
        actor: owner,
        good,
        qty,
    });
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
        dirty.0.insert(MarketGoodKey {
            market: bid.market,
            good: bid.good,
        });
        ledger.0.push(EconomyEvent::OrderExpired {
            order: id,
            actor: bid.owner,
            market: bid.market,
            good: bid.good,
        });
        ledger.0.push(EconomyEvent::CashReleased {
            actor: bid.owner,
            amount: bid.cash_locked_remaining,
        });
        orders.bids.remove(&id);
    }

    let expired_asks: Vec<_> = orders
        .asks
        .iter()
        .filter_map(|(id, ask)| (current_tick >= ask.expires_tick).then_some((*id, ask.clone())))
        .collect();
    for (id, ask) in expired_asks {
        inventory.release_goods(ask.owner, ask.good, ask.goods_locked_remaining)?;
        dirty.0.insert(MarketGoodKey {
            market: ask.market,
            good: ask.good,
        });
        ledger.0.push(EconomyEvent::OrderExpired {
            order: id,
            actor: ask.owner,
            market: ask.market,
            good: ask.good,
        });
        ledger.0.push(EconomyEvent::GoodsReleased {
            actor: ask.owner,
            good: ask.good,
            qty: ask.goods_locked_remaining,
        });
        orders.asks.remove(&id);
    }

    Ok(())
}

/// S2 drain primitive (ask side): release `q` units of a residual ask's goods lock
/// (locked→available, 1:1), shrink the order, and remove it at `qty_remaining == 0`.
/// PURE — mutates only the passed-in (scratch) clones; performs NO settle and is NOT
/// wired into any schedule (that is S3). Returns `true` if the order was fully drained
/// and removed. Rejects `q <= 0`, `q > qty_remaining`, or a missing id with
/// `InvalidOrder`. Never uses `debit_locked_goods` (the auction fill path) — that would
/// conflate matched-exit with residual-exit and let `expire_orders` double-release.
pub fn drain_residual_ask(
    orders: &mut OrderBook,
    inventory: &mut InventoryBook,
    ask_id: OrderId,
    q: Quantity,
) -> Result<bool, EconomyError> {
    let ask = orders
        .asks
        .get_mut(&ask_id)
        .ok_or(EconomyError::InvalidOrder)?;
    if q.0 <= 0 || q > ask.qty_remaining {
        return Err(EconomyError::InvalidOrder);
    }
    inventory.release_goods(ask.owner, ask.good, q)?;
    ask.qty_remaining = ask.qty_remaining.checked_sub(q)?;
    ask.goods_locked_remaining = ask.goods_locked_remaining.checked_sub(q)?;
    let removed = ask.qty_remaining.0 == 0;
    if removed {
        orders.asks.remove(&ask_id);
    }
    Ok(removed)
}

/// S2 drain primitive (bid side): release a residual bid's cash lock by the
/// FIELD-DIFFERENCE — never a recomputed per-`q` product — shrink the order, and
/// remove it at `qty_remaining == 0`. `released = cash_locked_remaining −
/// checked_order_value(max_price, new_qty)`; at `new_qty == 0` this is the full locked
/// field (`checked_order_value(_, 0) == 0`), disposing any floor-drift remainder so no
/// cash is orphaned in `locked` (spec §5.1, resolves CRITICAL #1). PURE — mutates only
/// the passed-in clones; NO settle, NOT wired (S3 adds the settle+refund wrapper).
/// Returns `true` if fully drained + removed. Rejects bad input with `InvalidOrder`.
pub fn drain_residual_bid(
    orders: &mut OrderBook,
    accounts: &mut AccountBook,
    bid_id: OrderId,
    q: Quantity,
) -> Result<bool, EconomyError> {
    let bid = orders
        .bids
        .get_mut(&bid_id)
        .ok_or(EconomyError::InvalidOrder)?;
    if q.0 <= 0 || q > bid.qty_remaining {
        return Err(EconomyError::InvalidOrder);
    }
    let new_qty = bid.qty_remaining.checked_sub(q)?;
    let target_lock = checked_order_value(bid.max_price, new_qty)?;
    let released = bid.cash_locked_remaining.checked_sub(target_lock)?;
    accounts.release_cash(bid.owner, released)?;
    bid.cash_locked_remaining = target_lock;
    bid.qty_remaining = new_qty;
    let removed = new_qty.0 == 0;
    if removed {
        orders.bids.remove(&bid_id);
    }
    Ok(removed)
}
