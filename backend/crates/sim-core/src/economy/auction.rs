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
        let qty = Quantity(sorted_bids[i].qty_remaining.0.min(sorted_asks[j].qty_remaining.0));
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
        .try_fold(Quantity::ZERO, |sum, bid| sum.checked_add(bid.qty_remaining))?;
    let unsold_supply = sorted_asks[j..]
        .iter()
        .try_fold(Quantity::ZERO, |sum, ask| sum.checked_add(ask.qty_remaining))?;

    Ok(ClearingPlan {
        key,
        fills,
        settlement_price: settlement,
        unmet_demand,
        unsold_supply,
    })
}
