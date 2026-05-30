use crate::economy::{Ask, Bid, EconomyError, MarketGoodKey, Money, OrderId, Quantity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    pub bid: OrderId,
    pub ask: OrderId,
    pub qty: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearingPlan {
    pub key: MarketGoodKey,
    pub fills: Vec<Fill>,
    pub settlement_price: Option<Money>,
    pub unmet_demand: Quantity,
    pub unsold_supply: Quantity,
}

pub fn settlement_price(last: Money, marginal_bid: Money, marginal_ask: Money) -> Money {
    debug_assert!(marginal_bid.0 >= marginal_ask.0);
    if last.0 < marginal_ask.0 {
        marginal_ask
    } else if last.0 > marginal_bid.0 {
        marginal_bid
    } else {
        last
    }
}

/// Largest-remainder (Hamilton) integer apportionment. Distributes `total` units
/// across `weights` proportionally to each weight; leftover units from flooring
/// are assigned one-by-one to the largest fractional remainders, ties broken by
/// ascending index (callers pass weights in a deterministic order). Returns a Vec
/// the same length as `weights`. When `total <= sum(weights)` each output is
/// `<= its weight`; when `total >= sum(weights)` each output equals its weight.
/// All inputs are treated as non-negative.
pub fn prorata_distribute(weights: &[i64], total: i64) -> Vec<i64> {
    let n = weights.len();
    let sum: i128 = weights.iter().map(|w| (*w).max(0) as i128).sum();
    if sum <= 0 || total <= 0 {
        return vec![0; n];
    }
    let total = (total as i128).min(sum);
    let mut alloc = vec![0i64; n];
    let mut remainders: Vec<(i128, usize)> = Vec::with_capacity(n);
    let mut distributed: i128 = 0;
    for (idx, &w) in weights.iter().enumerate() {
        let w = w.max(0) as i128;
        let num = total * w;
        let base = num / sum;
        alloc[idx] = base as i64;
        distributed += base;
        remainders.push((num % sum, idx));
    }
    let mut leftover = (total - distributed) as usize;
    // Largest remainder first; ties by ascending index for determinism.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, idx) in &remainders {
        if leftover == 0 {
            break;
        }
        alloc[idx] += 1;
        leftover -= 1;
    }
    alloc
}

pub fn build_clearing_plan(
    key: MarketGoodKey,
    bids: &[Bid],
    asks: &[Ask],
    last_settlement_price: Money,
) -> Result<ClearingPlan, EconomyError> {
    let mut sorted_bids = bids.to_vec();
    sorted_bids.sort_by(|a, b| {
        b.max_price
            .cmp(&a.max_price)
            .then(a.created_tick.cmp(&b.created_tick))
            .then(a.id.cmp(&b.id))
    });

    let mut sorted_asks = asks.to_vec();
    sorted_asks.sort_by(|a, b| {
        a.min_price
            .cmp(&b.min_price)
            .then(a.created_tick.cmp(&b.created_tick))
            .then(a.id.cmp(&b.id))
    });

    let mut i = 0;
    let mut j = 0;
    let mut fills = Vec::new();
    let mut marginal_bid = None;
    let mut marginal_ask = None;

    while i < sorted_bids.len() && j < sorted_asks.len() {
        if sorted_bids[i].max_price < sorted_asks[j].min_price {
            break;
        }
        let qty = Quantity(
            sorted_bids[i]
                .qty_remaining
                .0
                .min(sorted_asks[j].qty_remaining.0),
        );
        if qty.0 <= 0 {
            return Err(EconomyError::InvalidOrder);
        }
        fills.push(Fill {
            bid: sorted_bids[i].id,
            ask: sorted_asks[j].id,
            qty,
        });
        marginal_bid = Some(sorted_bids[i].max_price);
        marginal_ask = Some(sorted_asks[j].min_price);
        sorted_bids[i].qty_remaining = sorted_bids[i].qty_remaining.checked_sub(qty)?;
        sorted_asks[j].qty_remaining = sorted_asks[j].qty_remaining.checked_sub(qty)?;
        if sorted_bids[i].qty_remaining.0 == 0 {
            i += 1;
        }
        if sorted_asks[j].qty_remaining.0 == 0 {
            j += 1;
        }
    }

    let settlement = match (marginal_bid, marginal_ask) {
        (Some(bid), Some(ask)) => Some(settlement_price(last_settlement_price, bid, ask)),
        _ => None,
    };
    let unmet_demand = sorted_bids[i..]
        .iter()
        .try_fold(Quantity::ZERO, |sum, bid| {
            sum.checked_add(bid.qty_remaining)
        })?;
    let unsold_supply = sorted_asks[j..]
        .iter()
        .try_fold(Quantity::ZERO, |sum, ask| {
            sum.checked_add(ask.qty_remaining)
        })?;

    Ok(ClearingPlan {
        key,
        fills,
        settlement_price: settlement,
        unmet_demand,
        unsold_supply,
    })
}

use crate::economy::{
    AccountBook, EconomyEvent, InventoryBook, MarketGoodState, MarketGoods, OrderBook, TradeLedger,
    checked_order_value,
};

pub fn clear_market_good(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    market_goods: &mut MarketGoods,
    key: MarketGoodKey,
    current_tick: u64,
) -> Result<(), EconomyError> {
    // Get-or-create the market-good state so a freshly-dirtied key (the system
    // path) clears instead of failing with InvalidOrder. A never-traded market
    // starts at `last_settlement_price = ZERO`.
    let last_settlement_price = market_goods
        .0
        .entry(key)
        .or_insert_with(|| MarketGoodState::new(key))
        .last_settlement_price;
    let bids: Vec<_> = orders
        .bids
        .values()
        .filter(|bid| bid.market == key.market && bid.good == key.good)
        .cloned()
        .collect();
    let asks: Vec<_> = orders
        .asks
        .values()
        .filter(|ask| ask.market == key.market && ask.good == key.good)
        .cloned()
        .collect();
    let plan = build_clearing_plan(key, &bids, &asks, last_settlement_price)?;
    let Some(price) = plan.settlement_price else {
        if let Some(state) = market_goods.0.get_mut(&key) {
            state.traded_qty_last_tick = Quantity::ZERO;
            state.unmet_demand_last_tick = plan.unmet_demand;
            state.unsold_supply_last_tick = plan.unsold_supply;
            state.last_cleared_tick = current_tick;
            state.dirty = false;
        }
        return Ok(());
    };

    let mut next_accounts = accounts.clone();
    let mut next_inventory = inventory.clone();
    let mut next_orders = orders.clone();
    let mut trade_events = Vec::new();
    let mut traded_qty = Quantity::ZERO;

    for fill in &plan.fills {
        let bid = next_orders
            .bids
            .get_mut(&fill.bid)
            .ok_or(EconomyError::InvalidOrder)?
            .clone();
        let ask = next_orders
            .asks
            .get_mut(&fill.ask)
            .ok_or(EconomyError::InvalidOrder)?
            .clone();
        let locked_for_q = checked_order_value(bid.max_price, fill.qty)?;
        let actual_cost = checked_order_value(price, fill.qty)?;
        let refund = locked_for_q.checked_sub(actual_cost)?;

        next_accounts.debit_locked(bid.owner, locked_for_q)?;
        if refund.0 > 0 {
            next_accounts.deposit(bid.owner, refund)?;
        }
        next_accounts.deposit(ask.owner, actual_cost)?;
        next_inventory.debit_locked_goods(ask.owner, ask.good, fill.qty)?;
        next_inventory.deposit(bid.owner, bid.good, fill.qty)?;

        let bid_mut = next_orders.bids.get_mut(&fill.bid).unwrap();
        bid_mut.qty_remaining = bid_mut.qty_remaining.checked_sub(fill.qty)?;
        bid_mut.cash_locked_remaining = bid_mut.cash_locked_remaining.checked_sub(locked_for_q)?;
        let ask_mut = next_orders.asks.get_mut(&fill.ask).unwrap();
        ask_mut.qty_remaining = ask_mut.qty_remaining.checked_sub(fill.qty)?;
        ask_mut.goods_locked_remaining = ask_mut.goods_locked_remaining.checked_sub(fill.qty)?;

        trade_events.push(EconomyEvent::Trade {
            market: key.market,
            good: key.good,
            buyer: bid.owner,
            seller: ask.owner,
            qty: fill.qty,
            price,
            total: actual_cost,
        });
        if refund.0 > 0 {
            trade_events.push(EconomyEvent::CashReleased {
                actor: bid.owner,
                amount: refund,
            });
        }
        traded_qty = traded_qty.checked_add(fill.qty)?;
    }

    next_orders.bids.retain(|_, bid| bid.qty_remaining.0 > 0);
    next_orders.asks.retain(|_, ask| ask.qty_remaining.0 > 0);

    *accounts = next_accounts;
    *inventory = next_inventory;
    *orders = next_orders;
    ledger.0.extend(trade_events);
    if let Some(state) = market_goods.0.get_mut(&key) {
        state.last_settlement_price = price;
        state.traded_qty_last_tick = traded_qty;
        state.unmet_demand_last_tick = plan.unmet_demand;
        state.unsold_supply_last_tick = plan.unsold_supply;
        state.last_cleared_tick = current_tick;
        state.dirty = false;
    }
    Ok(())
}
