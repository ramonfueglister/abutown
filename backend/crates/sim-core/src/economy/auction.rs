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

/// Server-authoritative uniform-price settlement policy. `Anchored` (default)
/// clamps the previous settlement price into `[marginal_ask, marginal_bid]`
/// (price stability); `Midpoint` settles at the band midpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettlementPolicy {
    #[default]
    Anchored,
    Midpoint,
}

/// Settlement price under the chosen `policy`. `Midpoint` is the integer-floored
/// midpoint of the marginal band (always within `[ask, bid]`); the i128 sum
/// avoids overflow.
pub fn settlement_price_with_policy(
    last: Money,
    marginal_bid: Money,
    marginal_ask: Money,
    policy: SettlementPolicy,
) -> Money {
    match policy {
        SettlementPolicy::Anchored => settlement_price(last, marginal_bid, marginal_ask),
        SettlementPolicy::Midpoint => {
            debug_assert!(marginal_bid.0 >= marginal_ask.0);
            Money(((marginal_bid.0 as i128 + marginal_ask.0 as i128) / 2) as i64)
        }
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

/// Largest-remainder (Hamilton) apportionment of a CASH `total` across
/// `weights`, WITHOUT the `min(total, sum(weights))` clamp of
/// [`prorata_distribute`]. Each output is the floor of its proportional share
/// plus at most one leftover unit; the sum of outputs equals `total` exactly
/// (when `sum(weights) > 0` and `total > 0`). Use this — never
/// [`prorata_distribute`] — to apportion a money amount whose per-unit value
/// can exceed one weight-unit (e.g. `src_revenue` / `dst_payment` weighted by
/// traded goods), where clamping the total to the weight sum would silently
/// drop cash and break conservation. Weights are treated as non-negative and
/// must be passed in a deterministic order (ties broken by ascending index).
pub fn apportion_cash(weights: &[i64], total: i64) -> Vec<i64> {
    let n = weights.len();
    let sum: i128 = weights.iter().map(|w| (*w).max(0) as i128).sum();
    if sum <= 0 || total <= 0 {
        return vec![0; n];
    }
    let total = total as i128;
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
    build_clearing_plan_with_policy(
        key,
        bids,
        asks,
        last_settlement_price,
        SettlementPolicy::Anchored,
    )
}

pub fn build_clearing_plan_with_policy(
    key: MarketGoodKey,
    bids: &[Bid],
    asks: &[Ask],
    last_settlement_price: Money,
    policy: SettlementPolicy,
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

    // Phase 1: clearing quantity + marginal prices (unchanged price-time greedy).
    let mut i = 0;
    let mut j = 0;
    let mut total_q: i64 = 0;
    let mut marginal_bid: Option<Money> = None;
    let mut marginal_ask: Option<Money> = None;
    {
        let mut bid_rem: Vec<i64> = sorted_bids.iter().map(|b| b.qty_remaining.0).collect();
        let mut ask_rem: Vec<i64> = sorted_asks.iter().map(|a| a.qty_remaining.0).collect();
        while i < sorted_bids.len() && j < sorted_asks.len() {
            if sorted_bids[i].max_price < sorted_asks[j].min_price {
                break;
            }
            let q = bid_rem[i].min(ask_rem[j]);
            if q <= 0 {
                return Err(EconomyError::InvalidOrder);
            }
            total_q = total_q.checked_add(q).ok_or(EconomyError::Overflow)?;
            marginal_bid = Some(sorted_bids[i].max_price);
            marginal_ask = Some(sorted_asks[j].min_price);
            bid_rem[i] -= q;
            ask_rem[j] -= q;
            if bid_rem[i] == 0 {
                i += 1;
            }
            if ask_rem[j] == 0 {
                j += 1;
            }
        }
    }

    let total_bid_qty: i64 = sorted_bids.iter().map(|b| b.qty_remaining.0).sum();
    let total_ask_qty: i64 = sorted_asks.iter().map(|a| a.qty_remaining.0).sum();

    let (Some(m_bid), Some(m_ask)) = (marginal_bid, marginal_ask) else {
        return Ok(ClearingPlan {
            key,
            fills: Vec::new(),
            settlement_price: None,
            unmet_demand: Quantity(total_bid_qty),
            unsold_supply: Quantity(total_ask_qty),
        });
    };
    let settlement = settlement_price_with_policy(last_settlement_price, m_bid, m_ask, policy);

    // Phase 2: per-side allocation (infra-marginal full; marginal tier pro-rata).
    let bid_prices: Vec<i64> = sorted_bids.iter().map(|b| b.max_price.0).collect();
    let bid_qtys: Vec<i64> = sorted_bids.iter().map(|b| b.qty_remaining.0).collect();
    let bid_alloc = allocate_side(&bid_prices, &bid_qtys, m_bid.0, total_q, true);

    let ask_prices: Vec<i64> = sorted_asks.iter().map(|a| a.min_price.0).collect();
    let ask_qtys: Vec<i64> = sorted_asks.iter().map(|a| a.qty_remaining.0).collect();
    let ask_alloc = allocate_side(&ask_prices, &ask_qtys, m_ask.0, total_q, false);

    // Phase 3: pair allocations into fills (north-west corner).
    let fills = pair_fills(&sorted_bids, &bid_alloc, &sorted_asks, &ask_alloc);

    Ok(ClearingPlan {
        key,
        fills,
        settlement_price: Some(settlement),
        unmet_demand: Quantity(total_bid_qty - total_q),
        unsold_supply: Quantity(total_ask_qty - total_q),
    })
}

/// Allocate `total_q` units across one side. Orders strictly better than
/// `marginal` (higher for bids when `better_is_higher`, lower for asks) are filled
/// in full; orders priced exactly at `marginal` share the remainder pro-rata;
/// worse-priced orders get 0. Indexed parallel to `prices`/`qtys`.
fn allocate_side(
    prices: &[i64],
    qtys: &[i64],
    marginal: i64,
    total_q: i64,
    better_is_higher: bool,
) -> Vec<i64> {
    let n = prices.len();
    let mut alloc = vec![0i64; n];
    let mut infra_sum: i64 = 0;
    let mut marginal_idx: Vec<usize> = Vec::new();
    for (idx, &p) in prices.iter().enumerate() {
        let is_infra = if better_is_higher {
            p > marginal
        } else {
            p < marginal
        };
        if is_infra {
            alloc[idx] = qtys[idx];
            infra_sum += qtys[idx];
        } else if p == marginal {
            marginal_idx.push(idx);
        }
    }
    let to_ration = (total_q - infra_sum).max(0);
    let weights: Vec<i64> = marginal_idx.iter().map(|&k| qtys[k]).collect();
    let shares = prorata_distribute(&weights, to_ration);
    for (s, &k) in shares.iter().zip(marginal_idx.iter()) {
        alloc[k] = *s;
    }
    alloc
}

/// Pair per-bid and per-ask allocations (both summing to the same total) into
/// fills via a north-west-corner walk over the already-sorted orders.
fn pair_fills(bids: &[Bid], bid_alloc: &[i64], asks: &[Ask], ask_alloc: &[i64]) -> Vec<Fill> {
    let mut fills = Vec::new();
    let mut brem = bid_alloc.to_vec();
    let mut arem = ask_alloc.to_vec();
    let mut bi = 0;
    let mut aj = 0;
    while bi < bids.len() && aj < asks.len() {
        if brem[bi] == 0 {
            bi += 1;
            continue;
        }
        if arem[aj] == 0 {
            aj += 1;
            continue;
        }
        let q = brem[bi].min(arem[aj]);
        fills.push(Fill {
            bid: bids[bi].id,
            ask: asks[aj].id,
            qty: Quantity(q),
        });
        brem[bi] -= q;
        arem[aj] -= q;
    }
    fills
}

use crate::economy::{
    AccountBook, EconomicActorId, EconomyEvent, InventoryBook, MarketGoodState, MarketGoods,
    MarketId, OrderBook, TradeLedger, checked_order_value,
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
    clear_market_good_with_policy(
        accounts,
        inventory,
        orders,
        ledger,
        market_goods,
        key,
        current_tick,
        SettlementPolicy::Anchored,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn clear_market_good_with_policy(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    market_goods: &mut MarketGoods,
    key: MarketGoodKey,
    current_tick: u64,
    policy: SettlementPolicy,
) -> Result<(), EconomyError> {
    let mut discard_receipts = std::collections::BTreeMap::new();
    let mut discard_outlays = std::collections::BTreeMap::new();
    clear_market_good_with_receipts(
        accounts,
        inventory,
        orders,
        ledger,
        market_goods,
        key,
        current_tick,
        policy,
        &mut discard_receipts,
        &mut discard_outlays,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn clear_market_good_with_receipts(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    orders: &mut OrderBook,
    ledger: &mut TradeLedger,
    market_goods: &mut MarketGoods,
    key: MarketGoodKey,
    current_tick: u64,
    policy: SettlementPolicy,
    receipts: &mut std::collections::BTreeMap<(EconomicActorId, MarketId), Money>,
    outlays: &mut std::collections::BTreeMap<(EconomicActorId, MarketId), Money>,
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
    let plan = build_clearing_plan_with_policy(key, &bids, &asks, last_settlement_price, policy)?;
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
    let mut next_receipts = receipts.clone();
    let mut next_outlays = outlays.clone();
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
        // Accumulate seller revenue into next_receipts (scratch zone, before any commit).
        let slot = next_receipts
            .entry((ask.owner, key.market))
            .or_insert(Money::ZERO);
        *slot = slot.checked_add(actual_cost)?;
        // Accumulate buyer charge into next_outlays (scratch zone, before any commit).
        let slot = next_outlays
            .entry((bid.owner, key.market))
            .or_insert(Money::ZERO);
        *slot = slot.checked_add(actual_cost)?;
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

    // Commit block: all infallible assignments. Every `?` above fired in the
    // scratch zone (next_* clones) before any state is modified.
    *accounts = next_accounts;
    *inventory = next_inventory;
    *orders = next_orders;
    *receipts = next_receipts;
    *outlays = next_outlays;
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
