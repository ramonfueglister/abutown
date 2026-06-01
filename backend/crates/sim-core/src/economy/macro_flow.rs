//! Macro demand-driven cross-market flow (Economy LOD). Replaces warm-flow: a
//! mean-field spatial-price-equilibrium step over ALL dormant markets (warm AND
//! asleep), per coarse interval, per good. Goods flow surplus->deficit when the
//! price gap strictly exceeds transport; the realized band-clamped price is
//! written back so prices drift toward equilibrium across intervals.
//! Conservation-exact (atomic clone-validate-apply) and deterministic.

use std::collections::{BTreeMap, BTreeSet};

use crate::economy::pools::affordable_qty;
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, GoodId, InventoryBook,
    MarketDistances, MarketGoodKey, MarketGoods, MarketId, Money, Quantity, SettlementPolicy,
    SupplyPools, checked_order_value, settlement_price_with_policy, transport_cost,
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

/// One accepted-or-candidate directed flow edge for STEP D-F. `src == dst` is a
/// self-edge (local clearing of `matched`, transport 0, gate-exempt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub good: GoodId,
    pub src: MarketId,
    pub dst: MarketId,
    /// Fill cap = matched (self-edge) or min(surplus_src, deficit_dst) (cross-edge).
    pub q_cap: i64,
    pub p_src: Money,
    pub p_dst: Money,
    pub transport_total: Money,
    /// 0 for self-edges; strictly > 0 for kept cross-edges.
    pub net_gain: i64,
}

/// STEP D: enumerate candidate directed edges per good. Self-edges (matched > 0)
/// are always emitted (gate-exempt). Cross-edges (src surplus -> dst deficit) are
/// kept iff aggregate `net_gain > 0` on `q_cap`; any checked-op overflow in the
/// gate PRUNES the edge (an uncomputable edge is not an opportunity). Read-only.
pub fn build_candidates(
    buckets: &BTreeMap<MarketGoodKey, MacroBucket>,
    distances: &MarketDistances,
    config: &EconomyConfig,
) -> Result<Vec<Candidate>, EconomyError> {
    // Per good, per market: (matched, surplus, deficit, price).
    type MarketClassification = (i64, i64, i64, Money);
    let mut by_good: BTreeMap<GoodId, BTreeMap<MarketId, MarketClassification>> = BTreeMap::new();
    for (key, b) in buckets {
        let (matched, surplus, deficit) = classify_bucket(b.total_demand(), b.total_supply());
        by_good
            .entry(key.good)
            .or_default()
            .insert(key.market, (matched, surplus, deficit, b.price));
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (good, markets) in &by_good {
        // Self-edges: one per market with locally-clearable overlap.
        for (market, (matched, _surplus, _deficit, price)) in markets {
            if *matched > 0 {
                candidates.push(Candidate {
                    good: *good,
                    src: *market,
                    dst: *market,
                    q_cap: *matched,
                    p_src: *price,
                    p_dst: *price,
                    transport_total: Money::ZERO,
                    net_gain: 0,
                });
            }
        }
        // Cross-edges: ordered (src surplus, dst deficit) pairs.
        for (src, (_m_s, surplus, _d_s, p_src)) in markets {
            if *surplus <= 0 {
                continue;
            }
            for (dst, (_m_d, _s_d, deficit, p_dst)) in markets {
                if src == dst || *deficit <= 0 {
                    continue;
                }
                let q_cap = (*surplus).min(*deficit);
                if q_cap <= 0 {
                    continue;
                }
                let dist = match distances.0.get(&(*src, *dst)) {
                    Some(d) => *d,
                    None => continue, // no known route: not a candidate
                };
                // Aggregate transport gate on the actual q_cap; checked ops,
                // any overflow PRUNES the edge (no candidate, no event).
                let dst_value = match checked_order_value(*p_dst, Quantity(q_cap)) {
                    Ok(v) => v.0,
                    Err(_) => continue,
                };
                let src_value = match checked_order_value(*p_src, Quantity(q_cap)) {
                    Ok(v) => v.0,
                    Err(_) => continue,
                };
                let transport_total = match transport_cost(
                    dist,
                    Quantity(q_cap),
                    config.transport_cost_per_tile_unit,
                ) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let net_gain = match dst_value
                    .checked_sub(src_value)
                    .and_then(|g| g.checked_sub(transport_total.0))
                {
                    Some(g) => g,
                    None => continue,
                };
                if net_gain > 0 {
                    candidates.push(Candidate {
                        good: *good,
                        src: *src,
                        dst: *dst,
                        q_cap,
                        p_src: *p_src,
                        p_dst: *p_dst,
                        transport_total,
                        net_gain,
                    });
                }
            }
        }
    }
    Ok(candidates)
}
