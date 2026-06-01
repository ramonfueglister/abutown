//! Macro demand-driven cross-market flow (Economy LOD). Replaces warm-flow: a
//! mean-field spatial-price-equilibrium step over ALL dormant markets (warm AND
//! asleep), per coarse interval, per good. Goods flow surplus->deficit when the
//! price gap strictly exceeds transport; the realized band-clamped price is
//! written back so prices drift toward equilibrium across intervals.
//! Conservation-exact (atomic clone-validate-apply) and deterministic.

use std::collections::{BTreeMap, BTreeSet};

use crate::economy::pools::affordable_qty;
use crate::economy::{
    AccountBook, DemandPools, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent, GoodId,
    InventoryBook, MarketDistances, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money,
    Quantity, SettlementPolicy, SupplyPools, TRANSPORT_OPERATOR, checked_order_value,
    prorata_distribute, settlement_price_with_policy, transport_cost,
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
    /// Tile distance src->dst (0 for self-edges); lets STEP F recompute transport
    /// exactly on the actual `q` via `transport_cost(dist, q, rate)`.
    pub dist: i64,
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
                    dist: 0,
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
                        dist,
                    });
                }
            }
        }
    }
    Ok(candidates)
}

/// STEP E: total order over distinct keys — `net_gain DESC, good ASC, src ASC,
/// dst ASC`. All ids are BTree-keyed, so no surviving tie affects ordering.
pub fn sort_candidates(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        b.net_gain
            .cmp(&a.net_gain)
            .then(a.good.cmp(&b.good))
            .then(a.src.cmp(&b.src))
            .then(a.dst.cmp(&b.dst))
    });
}

/// A flow chosen by the STEP-F pass, ready for STEP-G settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFlow {
    pub good: GoodId,
    pub src: MarketId,
    pub dst: MarketId,
    pub q: i64,
    pub p_src: Money,
    pub p_dst: Money,
    pub dist: i64,
}

/// STEP F: single greedy pass over the sorted candidates with disjoint per-market
/// budgets (`remaining_matched` / `remaining_surplus` / `remaining_need`). Self-edges
/// consume matched; cross-edges consume surplus/need. Each budget is consumed
/// exactly once -> no double-spend; the output is a pure function of the sort.
pub fn plan_flows(
    candidates: &[Candidate],
    buckets: &BTreeMap<MarketGoodKey, MacroBucket>,
) -> Vec<PlannedFlow> {
    let mut remaining_matched: BTreeMap<(GoodId, MarketId), i64> = BTreeMap::new();
    let mut remaining_surplus: BTreeMap<(GoodId, MarketId), i64> = BTreeMap::new();
    let mut remaining_need: BTreeMap<(GoodId, MarketId), i64> = BTreeMap::new();
    for (key, b) in buckets {
        let (matched, surplus, deficit) = classify_bucket(b.total_demand(), b.total_supply());
        remaining_matched.insert((key.good, key.market), matched);
        remaining_surplus.insert((key.good, key.market), surplus);
        remaining_need.insert((key.good, key.market), deficit);
    }

    let mut flows: Vec<PlannedFlow> = Vec::new();
    for c in candidates {
        if c.src == c.dst {
            let avail = remaining_matched
                .get_mut(&(c.good, c.src))
                .copied()
                .unwrap_or(0);
            let q = avail.min(c.q_cap);
            if q <= 0 {
                continue;
            }
            if let Some(slot) = remaining_matched.get_mut(&(c.good, c.src)) {
                *slot -= q;
            }
            flows.push(PlannedFlow {
                good: c.good,
                src: c.src,
                dst: c.dst,
                q,
                p_src: c.p_src,
                p_dst: c.p_dst,
                dist: c.dist,
            });
        } else {
            let surplus = remaining_surplus
                .get(&(c.good, c.src))
                .copied()
                .unwrap_or(0);
            let need = remaining_need.get(&(c.good, c.dst)).copied().unwrap_or(0);
            let q = surplus.min(need).min(c.q_cap);
            if q <= 0 {
                continue;
            }
            if let Some(slot) = remaining_surplus.get_mut(&(c.good, c.src)) {
                *slot -= q;
            }
            if let Some(slot) = remaining_need.get_mut(&(c.good, c.dst)) {
                *slot -= q;
            }
            flows.push(PlannedFlow {
                good: c.good,
                src: c.src,
                dst: c.dst,
                q,
                p_src: c.p_src,
                p_dst: c.p_dst,
                dist: c.dist,
            });
        }
    }
    flows
}

/// STEP G: settle ONE accepted flow against the (cloned) books and write the
/// discovered prices back into `market_goods`. Aggregate-floor cash scheme: one
/// `src_revenue` floor; transport carved out of (never added on top of) the buyer
/// total; sellers credited from `src_revenue`, buyers charged `dst_payment` via
/// lock+debit. Returns the `MacroFlow` event. Does NOT touch `dirty`. The caller
/// passes the bucket-time effective demand/supply per endpoint for the residual
/// imbalance write-back (mirrors auction.rs:395-402).
#[allow(clippy::too_many_arguments)]
pub fn settle_flow(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    market_goods: &mut MarketGoods,
    flow: &PlannedFlow,
    sellers: &[(EconomicActorId, i64)],
    buyers: &[(EconomicActorId, i64)],
    eff_demand_src: i64,
    eff_supply_src: i64,
    eff_demand_dst: i64,
    eff_supply_dst: i64,
    config: &EconomyConfig,
    current_tick: u64,
) -> Result<EconomyEvent, EconomyError> {
    let q = flow.q;
    let src_revenue = checked_order_value(flow.p_src, Quantity(q))?;
    let transport_total =
        transport_cost(flow.dist, Quantity(q), config.transport_cost_per_tile_unit)?;
    let dst_payment = src_revenue.checked_add(transport_total)?;

    // Sellers at src: prorata goods, prorata cash out of src_revenue (Σ == src_revenue).
    let seller_w: Vec<i64> = sellers.iter().map(|(_, w)| *w).collect();
    let seller_goods = prorata_distribute(&seller_w, q);
    let seller_cash = prorata_distribute(&seller_goods, src_revenue.0);
    for (idx, (actor, _)) in sellers.iter().enumerate() {
        let goods = seller_goods[idx];
        if goods > 0 {
            inventory.consume(*actor, flow.good, Quantity(goods))?;
        }
        let receipt = Money(seller_cash[idx]);
        if receipt.0 > 0 {
            accounts.deposit(*actor, receipt)?;
        }
    }

    // Buyers at dst: prorata goods, prorata charge out of dst_payment (Σ == dst_payment).
    let buyer_w: Vec<i64> = buyers.iter().map(|(_, w)| *w).collect();
    let buyer_goods = prorata_distribute(&buyer_w, q);
    let buyer_charge = prorata_distribute(&buyer_goods, dst_payment.0);
    for (idx, (actor, _)) in buyers.iter().enumerate() {
        let goods = buyer_goods[idx];
        let charge = Money(buyer_charge[idx]);
        if charge.0 > 0 {
            accounts.lock_cash(*actor, charge)?;
            accounts.debit_locked(*actor, charge)?;
        }
        if goods > 0 {
            inventory.deposit(*actor, flow.good, Quantity(goods))?;
        }
    }

    // Transport: deposit to the reserved operator (transfer, never destroyed).
    if transport_total.0 > 0 {
        accounts.deposit(TRANSPORT_OPERATOR, transport_total)?;
    }

    // Write-back at src and dst. Residuals are against EFFECTIVE demand/supply:
    // post-flow unmet/unsold = effective_side - traded_q (clamped at 0).
    write_back(
        market_goods,
        MarketGoodKey {
            market: flow.src,
            good: flow.good,
        },
        flow.p_src,
        q,
        (eff_demand_src - q).max(0),
        (eff_supply_src - q).max(0),
        current_tick,
    );
    if flow.dst != flow.src {
        write_back(
            market_goods,
            MarketGoodKey {
                market: flow.dst,
                good: flow.good,
            },
            flow.p_dst,
            q,
            (eff_demand_dst - q).max(0),
            (eff_supply_dst - q).max(0),
            current_tick,
        );
    }

    Ok(EconomyEvent::MacroFlow {
        from_market: flow.src,
        to_market: flow.dst,
        good: flow.good,
        qty: Quantity(q),
        price: flow.p_dst,
        transport: transport_total,
    })
}

/// Apply the STEP-G market-state write-back for one endpoint. Accumulates
/// `traded_qty_last_tick` (a market may both self-clear and import in one
/// interval), sets the discovered price, last cleared tick, and post-flow
/// residual imbalance. Intentionally does NOT touch `dirty`.
fn write_back(
    market_goods: &mut MarketGoods,
    key: MarketGoodKey,
    price: Money,
    traded: i64,
    unmet_demand: i64,
    unsold_supply: i64,
    current_tick: u64,
) {
    let state = market_goods
        .0
        .entry(key)
        .or_insert_with(|| MarketGoodState::new(key));
    state.last_settlement_price = price;
    state.traded_qty_last_tick = Quantity(state.traded_qty_last_tick.0 + traded);
    state.unmet_demand_last_tick = Quantity(unmet_demand);
    state.unsold_supply_last_tick = Quantity(unsold_supply);
    state.last_cleared_tick = current_tick;
}
