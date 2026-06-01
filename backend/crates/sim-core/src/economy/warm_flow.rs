//! Warm-tier aggregate economy flow (Economy LOD). A market anchored to a WARM
//! chunk trades min(aggregate demand, aggregate supply) at its frozen reference
//! price, pro-rata, on a coarse interval — no order book, no price discovery.
//! Conservation-exact (atomic clone-validate-apply) and deterministic.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::economy::pools::affordable_qty;
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent,
    InventoryBook, MarketGoodKey, MarketGoods, MarketId, Money, Quantity, SupplyPools, TradeLedger,
    apportion_cash, checked_order_value, prorata_distribute,
};

fn warm_ref_price(market_goods: &MarketGoods, key: MarketGoodKey, config: &EconomyConfig) -> Money {
    match market_goods.0.get(&key) {
        Some(state) if state.last_settlement_price.0 > 0 => state.last_settlement_price,
        _ => config.trader_default_ref_price,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_warm_market_flow_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    demand: &DemandPools,
    supply: &SupplyPools,
    market_goods: &MarketGoods,
    warm_markets: &BTreeSet<MarketId>,
    config: &EconomyConfig,
    current_tick: u64,
) -> Result<(), EconomyError> {
    if config.warm_flow_interval_ticks == 0
        || !current_tick.is_multiple_of(config.warm_flow_interval_ticks)
    {
        return Ok(());
    }

    // Group warm demand/supply by market-good (deterministic BTreeMap order).
    type Side = Vec<(EconomicActorId, i64)>;
    let mut buckets: BTreeMap<MarketGoodKey, (Side, Side)> = BTreeMap::new();
    for pool in demand.0.values() {
        if warm_markets.contains(&pool.market) {
            buckets
                .entry(MarketGoodKey {
                    market: pool.market,
                    good: pool.good,
                })
                .or_default()
                .0
                .push((pool.actor, pool.desired_qty_per_tick.0));
        }
    }
    for pool in supply.0.values() {
        if warm_markets.contains(&pool.market) {
            buckets
                .entry(MarketGoodKey {
                    market: pool.market,
                    good: pool.good,
                })
                .or_default()
                .1
                .push((pool.actor, pool.offered_qty_per_tick.0));
        }
    }

    // Atomic clone-validate-apply (mirrors clear_market_good).
    let mut next_accounts = accounts.clone();
    let mut next_inventory = inventory.clone();
    let mut events: Vec<EconomyEvent> = Vec::new();

    for (key, (demands, supplies)) in &buckets {
        if demands.is_empty() || supplies.is_empty() {
            continue;
        }
        let price = warm_ref_price(market_goods, *key, config);
        if price.0 <= 0 {
            continue;
        }

        // Effective demand capped by affordability; supply capped by stock.
        let mut buyers: Side = Vec::new();
        for (actor, want) in demands {
            let cash = next_accounts.account(*actor).available;
            let afford = affordable_qty(cash, price)?.0;
            let eff = (*want).min(afford);
            if eff > 0 {
                buyers.push((*actor, eff));
            }
        }
        let mut sellers: Side = Vec::new();
        for (actor, offer) in supplies {
            let have = next_inventory.balance(*actor, key.good).available.0;
            let eff = (*offer).min(have);
            if eff > 0 {
                sellers.push((*actor, eff));
            }
        }
        if buyers.is_empty() || sellers.is_empty() {
            continue;
        }

        let total_demand: i64 = buyers.iter().map(|(_, q)| *q).sum();
        let total_supply: i64 = sellers.iter().map(|(_, q)| *q).sum();
        let traded = total_demand.min(total_supply);
        if traded <= 0 {
            continue;
        }

        let buyer_w: Vec<i64> = buyers.iter().map(|(_, q)| *q).collect();
        let seller_w: Vec<i64> = sellers.iter().map(|(_, q)| *q).collect();
        let buyer_goods = prorata_distribute(&buyer_w, traded);
        let seller_goods = prorata_distribute(&seller_w, traded);

        // Per-buyer floored cost; the exact sum is distributed to sellers so
        // both sides move identical cash (money conserved despite rounding).
        // apportion_cash (NOT prorata_distribute): per-unit cash exceeds one
        // goods-unit at any price > 1.0 scale-unit, and prorata_distribute's
        // min(total, Σweights) clamp would cap seller credit at the traded
        // quantity and silently destroy the price premium (warm_flow has no
        // transport operator to absorb it). apportion_cash distributes the
        // FULL buyers_total, so Σ seller_cash == buyers_total exactly.
        let mut costs: Vec<i64> = Vec::with_capacity(buyers.len());
        for goods in &buyer_goods {
            costs.push(checked_order_value(price, Quantity(*goods))?.0);
        }
        let buyers_total: i64 = costs.iter().sum();
        let seller_cash = apportion_cash(&seller_goods, buyers_total);

        for (idx, (actor, _)) in buyers.iter().enumerate() {
            let goods = buyer_goods[idx];
            let cost = Money(costs[idx]);
            if cost.0 > 0 {
                next_accounts.lock_cash(*actor, cost)?;
                next_accounts.debit_locked(*actor, cost)?;
            }
            if goods > 0 {
                next_inventory.deposit(*actor, key.good, Quantity(goods))?;
            }
        }
        for (idx, (actor, _)) in sellers.iter().enumerate() {
            let goods = seller_goods[idx];
            if goods > 0 {
                next_inventory.consume(*actor, key.good, Quantity(goods))?;
            }
            let receipt = Money(seller_cash[idx]);
            if receipt.0 > 0 {
                next_accounts.deposit(*actor, receipt)?;
            }
        }

        events.push(EconomyEvent::WarmMarketFlow {
            market: key.market,
            good: key.good,
            qty: Quantity(traded),
            price,
        });
    }

    *accounts = next_accounts;
    *inventory = next_inventory;
    ledger.0.extend(events);
    Ok(())
}
