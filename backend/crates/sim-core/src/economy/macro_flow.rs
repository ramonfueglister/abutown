//! Macro demand-driven cross-market flow (Economy LOD). Replaces warm-flow: a
//! mean-field spatial-price-equilibrium step over ALL dormant markets (warm AND
//! asleep), per coarse interval, per good. Goods flow surplus->deficit when the
//! price gap strictly exceeds transport; the realized band-clamped price is
//! written back so prices drift toward equilibrium across intervals.
//! Conservation-exact (atomic clone-validate-apply) and deterministic.

use std::collections::{BTreeMap, BTreeSet};

use crate::economy::pools::affordable_qty;
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, InventoryBook,
    MarketGoodKey, MarketGoods, MarketId, Money, SettlementPolicy, SupplyPools,
    settlement_price_with_policy,
};

/// Synthetic per-(market,good) price derived from the pool band each interval.
/// Both-sided markets clamp `prior` into `[ask_floor, bid_ceiling]` via the
/// auction primitive (price-discovering, drifts as `prior` updates). One-sided
/// markets are reservation-price-pinned: supply-only -> `ask_floor` (cheap),
/// demand-only -> `bid_ceiling` (dear), both ignoring `prior` (no local
/// discovery to move a reservation price). Callers must skip a `<= 0` result.
pub fn synthetic_price(
    has_demand: bool,
    has_supply: bool,
    bid_ceiling: Money,
    ask_floor: Money,
    prior: Money,
    policy: SettlementPolicy,
) -> Money {
    if has_demand && has_supply {
        if bid_ceiling.0 >= ask_floor.0 {
            settlement_price_with_policy(prior, bid_ceiling, ask_floor, policy)
        } else {
            // Crossed band (no clearable overlap on price): pin to ask_floor so
            // a usable positive price exists; the matched quantity is 0 anyway.
            ask_floor
        }
    } else if has_supply {
        ask_floor
    } else {
        // demand-only (has_demand == true here, else caller has no bucket)
        bid_ceiling
    }
}

/// Per-(market,good) aggregate after STEP A: synthetic price + effective
/// buyer/seller weight lists (actor, qty), capped by affordability / on-hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroBucket {
    pub price: Money,
    /// (actor, effective_demand_qty), filtered to qty > 0, in ascending actor order.
    pub buyers: Vec<(EconomicActorId, i64)>,
    /// (actor, effective_supply_qty), filtered to qty > 0, in ascending actor order.
    pub sellers: Vec<(EconomicActorId, i64)>,
}

impl MacroBucket {
    pub fn total_demand(&self) -> i64 {
        self.buyers.iter().map(|(_, q)| *q).sum()
    }
    pub fn total_supply(&self) -> i64 {
        self.sellers.iter().map(|(_, q)| *q).sum()
    }
}

fn prior_price(market_goods: &MarketGoods, key: MarketGoodKey, config: &EconomyConfig) -> Money {
    match market_goods.0.get(&key) {
        Some(state) if state.last_settlement_price.0 > 0 => state.last_settlement_price,
        _ => config.trader_default_ref_price,
    }
}

/// STEP C: partition a market's aggregate demand/supply into the locally-clearable
/// overlap `matched = min(D, S)`, the exportable `surplus = S - matched`, and the
/// importable `deficit = D - matched`. At most one of surplus/deficit is non-zero;
/// `matched` and the residual are disjoint quantities (overlap vs excess).
pub fn classify_bucket(total_demand: i64, total_supply: i64) -> (i64, i64, i64) {
    let matched = total_demand.min(total_supply).max(0);
    let surplus = (total_supply - matched).max(0);
    let deficit = (total_demand - matched).max(0);
    (matched, surplus, deficit)
}

/// STEP A: build the dormant aggregate buckets. Groups dormant demand/supply by
/// market-good, derives the synthetic price from the raw band, then caps demand
/// by affordability (at `price`) and supply by on-hand stock. Buckets whose
/// `price <= 0` are dropped (the warm-flow zero-band skip, applied before any
/// `affordable_qty`). Empty buyer AND empty seller buckets are dropped.
#[allow(clippy::too_many_arguments)]
pub fn build_macro_buckets(
    accounts: &AccountBook,
    inventory: &InventoryBook,
    demand: &DemandPools,
    supply: &SupplyPools,
    market_goods: &MarketGoods,
    dormant: &BTreeSet<MarketId>,
    config: &EconomyConfig,
) -> Result<BTreeMap<MarketGoodKey, MacroBucket>, EconomyError> {
    // Phase 1: raw bands (max_price ceiling for buyers, min_price floor for sellers).
    type Raw = (Vec<(EconomicActorId, i64)>, Option<Money>); // (entries, band-extreme)
    let mut raw_demand: BTreeMap<MarketGoodKey, Raw> = BTreeMap::new();
    let mut raw_supply: BTreeMap<MarketGoodKey, Raw> = BTreeMap::new();
    for pool in demand.0.values() {
        if !dormant.contains(&pool.market) {
            continue;
        }
        let key = MarketGoodKey {
            market: pool.market,
            good: pool.good,
        };
        let entry = raw_demand.entry(key).or_insert_with(|| (Vec::new(), None));
        entry.0.push((pool.actor, pool.desired_qty_per_tick.0));
        entry.1 = Some(match entry.1 {
            Some(c) if c.0 >= pool.max_price.0 => c,
            _ => pool.max_price,
        });
    }
    for pool in supply.0.values() {
        if !dormant.contains(&pool.market) {
            continue;
        }
        let key = MarketGoodKey {
            market: pool.market,
            good: pool.good,
        };
        let entry = raw_supply.entry(key).or_insert_with(|| (Vec::new(), None));
        entry.0.push((pool.actor, pool.offered_qty_per_tick.0));
        entry.1 = Some(match entry.1 {
            Some(f) if f.0 <= pool.min_price.0 => f,
            _ => pool.min_price,
        });
    }

    // Phase 2: union of keys -> synthetic price -> effective caps.
    let mut keys: BTreeSet<MarketGoodKey> = BTreeSet::new();
    keys.extend(raw_demand.keys().copied());
    keys.extend(raw_supply.keys().copied());

    let mut buckets: BTreeMap<MarketGoodKey, MacroBucket> = BTreeMap::new();
    for key in keys {
        let d = raw_demand.get(&key);
        let s = raw_supply.get(&key);
        let has_demand = d.is_some();
        let has_supply = s.is_some();
        let bid_ceiling = d.and_then(|(_, c)| *c).unwrap_or(Money::ZERO);
        let ask_floor = s.and_then(|(_, f)| *f).unwrap_or(Money::ZERO);
        let prior = prior_price(market_goods, key, config);
        let price = synthetic_price(
            has_demand,
            has_supply,
            bid_ceiling,
            ask_floor,
            prior,
            config.settlement_policy,
        );
        if price.0 <= 0 {
            continue; // zero/negative band: skip, never ZeroPrice-abort.
        }
        let mut buyers: Vec<(EconomicActorId, i64)> = Vec::new();
        if let Some((entries, _)) = d {
            for (actor, want) in entries {
                let cash = accounts.account(*actor).available;
                let afford = affordable_qty(cash, price)?.0;
                let eff = (*want).min(afford);
                if eff > 0 {
                    buyers.push((*actor, eff));
                }
            }
        }
        let mut sellers: Vec<(EconomicActorId, i64)> = Vec::new();
        if let Some((entries, _)) = s {
            for (actor, offer) in entries {
                let have = inventory.balance(*actor, key.good).available.0;
                let eff = (*offer).min(have);
                if eff > 0 {
                    sellers.push((*actor, eff));
                }
            }
        }
        if buyers.is_empty() && sellers.is_empty() {
            continue;
        }
        buckets.insert(
            key,
            MacroBucket {
                price,
                buyers,
                sellers,
            },
        );
    }
    Ok(buckets)
}
