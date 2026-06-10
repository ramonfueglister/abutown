//! Macro demand-driven cross-market flow (Economy LOD). Replaces warm-flow: a
//! mean-field spatial-price-equilibrium step over ALL dormant markets (warm AND
//! asleep), per coarse interval, per good. Goods flow surplus->deficit when the
//! price gap strictly exceeds transport; the realized band-clamped price is
//! written back so prices drift toward equilibrium across intervals.
//! Conservation-exact (atomic clone-validate-apply) and deterministic.

use std::collections::{BTreeMap, BTreeSet};

use crate::economy::orders::{drain_residual_ask, drain_residual_bid};
use crate::economy::pools::affordable_qty;
use crate::economy::producers::{
    InputPools, ProducerPolicies, participation_bound, scaled_batch_qty,
};
use crate::economy::{
    AccountBook, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyConfig, EconomyError,
    EconomyEvent, GoodId, InventoryBook, MarketDistances, MarketGoodKey, MarketGoodState,
    MarketGoods, MarketId, Money, OrderBook, OrderId, Quantity, SettlementPolicy, SupplyPools,
    TRANSPORT_OPERATOR, TradeLedger, apportion_cash, checked_order_value, prorata_distribute,
    settlement_price_with_policy, transport_cost,
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
    /// S3: `true` ⇒ active/residual-sourced (the auction already intra-cleared this
    /// market this tick); `false` ⇒ dormant/pool-sourced. Suppresses the self-edge for
    /// active buckets (the flow must not re-clear units the auction refused on price).
    pub intra_cleared: bool,
    /// S3 active-only provenance, index-aligned to `buyers`/`sellers`; **empty** for
    /// dormant buckets. `buyer_orders[i]` is the residual bid backing `buyers[i]`,
    /// `buyer_max_prices[i]` its `max_price` (the affordability-predicate input), and
    /// `seller_orders[i]` the residual ask backing `sellers[i]`. One entry per `OrderId`
    /// (NOT per owner) so the per-row drain and the per-row affordability test agree.
    pub buyer_orders: Vec<OrderId>,
    pub buyer_max_prices: Vec<Money>,
    pub seller_orders: Vec<OrderId>,
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

/// STEP A: build the aggregate buckets from TWO sources. **Dormant** markets are
/// pool-sourced (this function's original behavior): group dormant demand/supply by
/// market-good, derive the synthetic price from the raw band, cap demand by
/// affordability and supply by on-hand stock; `price <= 0` buckets are dropped;
/// `intra_cleared = false`. Dormant demand has TWO pool sources: `DemandPools`
/// (consumer flow demand) and producer `InputPools` (Leontief stock demand,
/// firms-as-buyers — see the inline doc on the input-pool loop; this is the path
/// that keeps a production chain alive while its market never wakes). **Active**
/// (non-dormant) markets are residual-ORDER-sourced
/// (S3, gated on `drain_active_residual`): one bucket entry per residual `OrderId`,
/// weight = `qty_remaining`, price = the auction's `last_settlement_price` (via
/// `prior_price`), `intra_cleared = true`. ALL residual bids are admitted — affordability
/// is decided per-edge by `prune_unaffordable_buyers` against the actual landed charge
/// (`value(p_src, q) + transport`), there is no build-time price filter. The two sources
/// are disjoint by market.
#[allow(clippy::too_many_arguments)]
pub fn build_macro_buckets(
    accounts: &AccountBook,
    inventory: &InventoryBook,
    demand: &DemandPools,
    supply: &SupplyPools,
    input_pools: &mut InputPools,
    policies: &ProducerPolicies,
    capita_factor: i64,
    current_tick: u64,
    market_goods: &MarketGoods,
    dormant: &BTreeSet<MarketId>,
    config: &EconomyConfig,
    orders: &OrderBook,
    drain_active_residual: bool,
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
    // Dormant-market PRODUCER input demand (firms-as-buyers, design spec §4: the
    // chain's input leg rides "the unchanged macro_flow" with NO viewer). While a
    // producer's market is dormant the order path
    // (`run_generate_input_orders_at_tick`) skips its `InputPool` entirely, so the
    // Leontief derived demand must be expressed HERE or the chain is inert in the
    // normal headless state (no bid, `max_price` stuck at 0, zero production).
    //
    // - qty: the SAME stock-target demand the active order path would express —
    //   `scaled_batch_qty(policy, pool, capita_factor) − held` floored at 0. The
    //   target is a STOCK, so expressing it on the (coarser) macro cadence instead
    //   of the pool's own `interval_ticks` can never over-buy: each pass re-derives
    //   the gap against current holdings.
    // - ceiling: the SAME participation bound the order path computes, from the
    //   CURRENT output reference price; a missing market-good entry is a fail-loud
    //   `ZeroPrice` exactly like the active path (every output market is seeded
    //   with a positive opening price).
    // - write-back: the discovered bound is stored in `pool.max_price` even when no
    //   demand is expressed (stocked, or the bound floored to 0). The θ-dividend
    //   retention (wages.rs) keys its working-capital target off `pool.max_price`;
    //   if the bound stayed 0 while the firm buys via this path, the firm would
    //   retain its entire profit forever (the unpriced-pool conservative-retention
    //   guard in `run_distribute_profit_at_tick`, wages.rs)
    //   and its cash would grow without bound. `last_generated_tick` is stamped
    //   too: the cursor means "a demand-generation pass ran for this pool at this
    //   tick" — this IS that pass on the macro cadence (the order path remains the
    //   stamping authority while the market is active). Both writes happen at
    //   bucket-build time, before the atomic settle boundary: bound discovery is
    //   price telemetry, not a money move, and stays valid even when the interval
    //   settles nothing (or the key is later dirty-filtered).
    // - mismatch fail-fast: an `InputPool` without a `ProducerPolicy` is a config
    //   bug (#83 class) — `InvalidOrder`, same doctrine as the order path.
    //
    // KNOWN GAP: like the `DemandPools`/`SupplyPools`
    // loops above, the macro flow's dormant QUANTITIES are capita-blind on the
    // supply side (`offered_qty_per_tick` unscaled), so the chain's input arrives
    // at the unscaled per-cadence trickle while a scaled batch needs
    // `in_qty·capita_factor` — slow-but-alive, not a starvation bug.
    let labor_share = config.validated_labor_share_bps()?;
    for pool in input_pools.0.values_mut() {
        if !dormant.contains(&pool.market) {
            continue;
        }
        let policy = policies
            .0
            .get(&pool.actor)
            .copied()
            .ok_or(EconomyError::InvalidOrder)?;
        let p_out_ref = market_goods
            .0
            .get(&MarketGoodKey {
                market: pool.market,
                good: pool.out_good,
            })
            .ok_or(EconomyError::ZeroPrice)?
            .ewma_reference_price;
        pool.max_price = participation_bound(p_out_ref, labor_share, pool.out_qty, pool.in_qty)?;
        pool.last_generated_tick = Some(current_tick);

        let target = scaled_batch_qty(policy, pool, capita_factor)?;
        let held = inventory.balance(pool.actor, pool.good).available;
        let desired = (target.0 - held.0).max(0);
        if desired <= 0 || pool.max_price.0 <= 0 {
            continue; // stocked, or the bound floored to 0 (recovers when p_out rises)
        }
        let key = MarketGoodKey {
            market: pool.market,
            good: pool.good,
        };
        let entry = raw_demand.entry(key).or_insert_with(|| (Vec::new(), None));
        entry.0.push((pool.actor, desired));
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
                intra_cleared: false,
                buyer_orders: Vec::new(),
                buyer_max_prices: Vec::new(),
                seller_orders: Vec::new(),
            },
        );
    }

    // ACTIVE CONTRIBUTOR (S3): for markets NOT dormant, source buckets from the
    // post-auction RESIDUAL ORDERS — one entry per OrderId (DECISION-1) so the per-row
    // affordability predicate and the per-OrderId drain agree. Gated on
    // `drain_active_residual` so S1/S2 land dark.
    if drain_active_residual {
        #[derive(Default)]
        struct ActiveAccum {
            buyers: Vec<(EconomicActorId, i64)>,
            buyer_orders: Vec<OrderId>,
            buyer_max_prices: Vec<Money>,
            sellers: Vec<(EconomicActorId, i64)>,
            seller_orders: Vec<OrderId>,
        }
        let mut active: BTreeMap<MarketGoodKey, ActiveAccum> = BTreeMap::new();
        // BTreeMap iteration is OrderId-ascending -> deterministic per-key entry order.
        for (id, bid) in orders.bids.iter() {
            if dormant.contains(&bid.market) || bid.qty_remaining.0 <= 0 {
                continue;
            }
            let key = MarketGoodKey {
                market: bid.market,
                good: bid.good,
            };
            // No build-time affordability filter: a flow charges buyers the SOURCE price
            // + transport (settle_flow's `src_revenue + transport`, not p_dst), which is
            // edge-specific and unknown here. The per-edge Stage-2 prune
            // (`prune_unaffordable_buyers`, priced at the actual landed charge) is the
            // sole affordability authority — admitting all residual bids serves every
            // buyer willing to pay the landed price (Law of One Price).
            let e = active.entry(key).or_default();
            e.buyers.push((bid.owner, bid.qty_remaining.0));
            e.buyer_orders.push(*id);
            e.buyer_max_prices.push(bid.max_price);
        }
        for (id, ask) in orders.asks.iter() {
            if dormant.contains(&ask.market) || ask.qty_remaining.0 <= 0 {
                continue;
            }
            let key = MarketGoodKey {
                market: ask.market,
                good: ask.good,
            };
            let e = active.entry(key).or_default();
            e.sellers.push((ask.owner, ask.qty_remaining.0));
            e.seller_orders.push(*id);
        }
        for (key, acc) in active {
            // Dormant pools and active orders never collide on a key (a dormant market
            // generates no orders; an active market is not dormant).
            debug_assert!(
                !buckets.contains_key(&key),
                "active-order bucket collides with a dormant-pool bucket"
            );
            buckets.insert(
                key,
                MacroBucket {
                    price: prior_price(market_goods, key, config),
                    buyers: acc.buyers,
                    sellers: acc.sellers,
                    intra_cleared: true,
                    buyer_orders: acc.buyer_orders,
                    buyer_max_prices: acc.buyer_max_prices,
                    seller_orders: acc.seller_orders,
                },
            );
        }
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
    // Per good, per market: (matched, surplus, deficit, price, intra_cleared).
    type MarketClassification = (i64, i64, i64, Money, bool);
    let mut by_good: BTreeMap<GoodId, BTreeMap<MarketId, MarketClassification>> = BTreeMap::new();
    for (key, b) in buckets {
        let (matched, surplus, deficit) = classify_bucket(b.total_demand(), b.total_supply());
        by_good.entry(key.good).or_default().insert(
            key.market,
            (matched, surplus, deficit, b.price, b.intra_cleared),
        );
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (good, markets) in &by_good {
        // Self-edges: one per market with locally-clearable overlap.
        for (market, (matched, _surplus, _deficit, price, intra_cleared)) in markets {
            // Suppress the self-edge for active (intra_cleared) buckets: the auction
            // already cleared the matched overlap this tick, and a non-crossing active
            // market's matched>0 reflects bids/asks the auction REFUSED on price — a
            // flow self-edge would double-clear them. Dormant self-edges stay (their
            // self-edge IS the intra-clear).
            if *matched > 0 && !*intra_cleared {
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
        for (src, (_m_s, surplus, _d_s, p_src, _intra_s)) in markets {
            if *surplus <= 0 {
                continue;
            }
            for (dst, (_m_d, _s_d, deficit, p_dst, _intra_d)) in markets {
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
        // Defense-in-depth (mirrors build_candidates' self-edge suppression): force the
        // matched budget to 0 for active buckets, so a stray self-edge Candidate that
        // bypassed build_candidates finds no budget and skips at the `q <= 0` continue.
        let matched = if b.intra_cleared { 0 } else { matched };
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

/// Output of [`prune_unaffordable_buyers`]: the residual bids that survive the
/// landed-price affordability test, index-aligned, plus `q_prime` — the flow quantity
/// those survivors actually absorb (`Σ prorata(surviving_weights, q)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedBuyers {
    pub buyers: Vec<(EconomicActorId, i64)>,
    pub orders: Vec<OrderId>,
    pub max_prices: Vec<Money>,
    pub q_prime: i64,
}

/// S3 affordability bound (Stage-2, per-edge). A residual bid drained for its share
/// `g_i` releases ~`value(max_price_i, g_i)` to available; `settle_flow` then charges it
/// `apportion_cash(buyer_goods, dst_payment)[i]`, where `dst_payment = value(p_src, q') +
/// transport` — the SOURCE price + transport, exactly as `settle_flow` computes it
/// (`src_revenue + transport`, NOT `p_dst`; the buyer pays what the seller takes plus
/// transport, not the destination's discovered price). If `value(max_price_i, g_i) <
/// charge_i` the release under-funds the charge and `lock_cash` faults → stranded
/// residual. This drops every buyer whose `max_price` cannot cover its apportioned landed
/// charge, recomputes, and repeats to a **fixpoint** (dropping one buyer raises the
/// survivors' per-unit share via prorata, which can newly disqualify a marginal survivor —
/// a single pass would not be order-independent). Pure, deterministic (largest-remainder
/// prorata/apportion, ascending index, i64/i128), bounded by `buyers.len()` passes.
/// Returns the survivors + `q_prime`; `q_prime == 0` ⇒ skip the edge. Because this prices
/// the charge identically to `settle_flow`, a surviving buyer can never fault `lock_cash`
/// (its field-difference release ≥ `value(max_price_i, g_i)` ≥ `charge_i`).
#[allow(clippy::too_many_arguments)]
pub fn prune_unaffordable_buyers(
    buyers: &[(EconomicActorId, i64)],
    buyer_orders: &[OrderId],
    max_prices: &[Money],
    q: i64,
    dist: i64,
    p_src: Money,
    config: &EconomyConfig,
) -> Result<PrunedBuyers, EconomyError> {
    let mut keep: Vec<usize> = (0..buyers.len()).collect();
    loop {
        if keep.is_empty() {
            return Ok(PrunedBuyers {
                buyers: Vec::new(),
                orders: Vec::new(),
                max_prices: Vec::new(),
                q_prime: 0,
            });
        }
        let w: Vec<i64> = keep.iter().map(|&i| buyers[i].1).collect();
        let goods = prorata_distribute(&w, q); // Σ goods = min(q, Σw)
        let q_prime: i64 = goods.iter().sum();
        if q_prime <= 0 {
            return Ok(PrunedBuyers {
                buyers: Vec::new(),
                orders: Vec::new(),
                max_prices: Vec::new(),
                q_prime: 0,
            });
        }
        let dst_payment = checked_order_value(p_src, Quantity(q_prime))?.checked_add(
            transport_cost(dist, Quantity(q_prime), config.transport_cost_per_tile_unit)?,
        )?;
        let charge = apportion_cash(&goods, dst_payment.0);
        let mut next_keep: Vec<usize> = Vec::new();
        for (j, &i) in keep.iter().enumerate() {
            let released = checked_order_value(max_prices[i], Quantity(goods[j]))?;
            if released.0 >= charge[j] {
                next_keep.push(i);
            }
        }
        if next_keep.len() == keep.len() {
            return Ok(PrunedBuyers {
                buyers: keep.iter().map(|&i| buyers[i]).collect(),
                orders: keep.iter().map(|&i| buyer_orders[i]).collect(),
                max_prices: keep.iter().map(|&i| max_prices[i]).collect(),
                q_prime,
            });
        }
        keep = next_keep;
    }
}

/// STEP G: settle ONE accepted flow against the (cloned) books and write the
/// discovered prices back into `market_goods`. Aggregate-floor cash scheme: one
/// `src_revenue` floor; `dst_payment = src_revenue + transport` (transport is
/// the buyer's premium over the seller's take, never destroyed). Sellers are
/// credited a largest-remainder split of the FULL `src_revenue` (Σ seller_cash
/// == src_revenue) and buyers are charged a largest-remainder split of the FULL
/// `dst_payment` (Σ buyer_charge == dst_payment) via lock+debit. The cash split
/// uses [`apportion_cash`], NOT [`prorata_distribute`]: per-unit cash can exceed
/// one goods-unit (any price >= 1.0 scale-unit, including the default reference
/// price), and `prorata_distribute`'s `min(total, Σweights)` clamp would cap the
/// distributed cash at the traded quantity and silently mint `transport` money.
/// Net money delta is exactly zero: -dst_payment (buyers) + src_revenue
/// (sellers) + transport (operator) == 0. Returns the `MacroFlow` event. Does
/// NOT touch `dirty`. The caller passes the bucket-time effective demand/supply
/// per endpoint for the residual imbalance write-back (mirrors
/// auction.rs:395-402).
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
    preserve_price_src: bool,
    preserve_price_dst: bool,
) -> Result<EconomyEvent, EconomyError> {
    let mut discard_receipts = BTreeMap::new();
    let mut discard_outlays = BTreeMap::new();
    settle_flow_with_receipts(
        accounts,
        inventory,
        market_goods,
        flow,
        sellers,
        buyers,
        eff_demand_src,
        eff_supply_src,
        eff_demand_dst,
        eff_supply_dst,
        config,
        current_tick,
        preserve_price_src,
        preserve_price_dst,
        &mut discard_receipts,
        &mut discard_outlays,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn settle_flow_with_receipts(
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
    preserve_price_src: bool,
    preserve_price_dst: bool,
    receipts: &mut BTreeMap<(EconomicActorId, MarketId), Money>,
    outlays: &mut BTreeMap<(EconomicActorId, MarketId), Money>,
) -> Result<EconomyEvent, EconomyError> {
    let q = flow.q;
    let src_revenue = checked_order_value(flow.p_src, Quantity(q))?;
    let transport_total =
        transport_cost(flow.dist, Quantity(q), config.transport_cost_per_tile_unit)?;
    let dst_payment = src_revenue.checked_add(transport_total)?;

    // Sellers at src: prorata goods (clamped to Σweights == q is correct here),
    // then a non-clamping largest-remainder split of the FULL src_revenue across
    // those goods (Σ seller_cash == src_revenue, even when src_revenue > q).
    let seller_w: Vec<i64> = sellers.iter().map(|(_, w)| *w).collect();
    let seller_goods = prorata_distribute(&seller_w, q);
    let seller_cash = apportion_cash(&seller_goods, src_revenue.0);
    for (idx, (actor, _)) in sellers.iter().enumerate() {
        let goods = seller_goods[idx];
        if goods > 0 {
            inventory.consume(*actor, flow.good, Quantity(goods))?;
        }
        let receipt = Money(seller_cash[idx]);
        if receipt.0 > 0 {
            accounts.deposit(*actor, receipt)?;
            // Accumulate seller revenue into receipts (non-monetary statistic).
            let slot = receipts.entry((*actor, flow.src)).or_insert(Money::ZERO);
            *slot = slot.checked_add(receipt)?;
        }
    }

    // Buyers at dst: prorata goods (clamped to Σweights == q is correct here),
    // then a non-clamping largest-remainder split of the FULL dst_payment across
    // those goods (Σ buyer_charge == dst_payment == src_revenue + transport, so
    // the buyers cover both the seller take and the transport premium).
    let buyer_w: Vec<i64> = buyers.iter().map(|(_, w)| *w).collect();
    let buyer_goods = prorata_distribute(&buyer_w, q);
    let buyer_charge = apportion_cash(&buyer_goods, dst_payment.0);
    for (idx, (actor, _)) in buyers.iter().enumerate() {
        let goods = buyer_goods[idx];
        let charge = Money(buyer_charge[idx]);
        if charge.0 > 0 {
            accounts.lock_cash(*actor, charge)?;
            accounts.debit_locked(*actor, charge)?;
            // Accumulate buyer charge into outlays (non-monetary statistic).
            // Booked AFTER the successful money move so a fault aborts both.
            let slot = outlays.entry((*actor, flow.dst)).or_insert(Money::ZERO);
            *slot = slot.checked_add(charge)?;
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
        preserve_price_src,
    )?;
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
            preserve_price_dst,
        )?;
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
/// `traded_qty_last_tick` via [`Quantity::checked_add`] (a market may both
/// self-clear and import in one interval, so this can sum across calls), sets
/// the discovered price, last cleared tick, and post-flow residual imbalance.
/// Intentionally does NOT touch `dirty`. Returns `Err(Overflow)` rather than
/// wrapping the accumulator, matching the checked-everywhere discipline of the
/// rest of this module (and `auction.rs`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_back(
    market_goods: &mut MarketGoods,
    key: MarketGoodKey,
    price: Money,
    traded: i64,
    unmet_demand: i64,
    unsold_supply: i64,
    current_tick: u64,
    preserve_price: bool,
) -> Result<(), EconomyError> {
    let state = market_goods
        .0
        .entry(key)
        .or_insert_with(|| MarketGoodState::new(key));
    // For an ACTIVE endpoint the auction discovered `last_settlement_price` this tick and
    // is authoritative (user decision a) — the flow must not clobber it. Dormant
    // endpoints keep writing the flow-discovered price. traded/unmet/unsold ALWAYS update
    // (the post-drain residual is the reality #71's shopper projection reads).
    if !preserve_price {
        state.last_settlement_price = price;
    }
    state.traded_qty_last_tick = state.traded_qty_last_tick.checked_add(Quantity(traded))?;
    state.unmet_demand_last_tick = Quantity(unmet_demand);
    state.unsold_supply_last_tick = Quantity(unsold_supply);
    state.last_cleared_tick = current_tick;
    Ok(())
}

/// S3: release the active endpoints' residual locks into available on the per-edge
/// SCRATCH clones, so `settle_flow` (which consumes/charges from available) can move
/// them. BOTH sides drain the **exact per-entry settle share** so released-per-actor ==
/// consumed/charged-per-actor (no per-actor mismatch). Ask side: each seller's share
/// `prorata(seller_weights, q')[i]` from its backing ask (`seller_orders[i]`). Bid side:
/// each surviving buyer's share `prorata(buyer_weights, q')[i]` from its backing bid
/// (`buyer_orders[i]`). One OrderId per entry (DECISION-1), index-aligned to the
/// `sellers`/`buyers` weight lists `settle_flow` is given, so the drain and the settle
/// reference the identical quantities. Dormant endpoints (`!active`) are no-ops (their
/// goods/cash are already available from pools).
#[allow(clippy::too_many_arguments)]
fn drain_active_endpoints(
    scratch_orders: &mut OrderBook,
    scratch_inventory: &mut InventoryBook,
    scratch_accounts: &mut AccountBook,
    eflow: &PlannedFlow,
    sellers: &[(EconomicActorId, i64)],
    seller_orders: &[OrderId],
    buyers: &[(EconomicActorId, i64)],
    buyer_orders: &[OrderId],
    src_active: bool,
    dst_active: bool,
) -> Result<(), EconomyError> {
    let q_prime = eflow.q;
    if src_active {
        // Mirror settle_flow's seller prorata (macro_flow `prorata_distribute(seller_w, q)`)
        // so each seller's released goods == the goods settle will consume from it. A
        // greedy OrderId walk would release a DIFFERENT per-actor split than the prorata
        // consume and fault `inventory.consume` for q' < Σ supply with >= 2 asks.
        let seller_w: Vec<i64> = sellers.iter().map(|(_, w)| *w).collect();
        let seller_goods = prorata_distribute(&seller_w, q_prime);
        for (i, &oid) in seller_orders.iter().enumerate() {
            if seller_goods[i] > 0 {
                drain_residual_ask(
                    scratch_orders,
                    scratch_inventory,
                    oid,
                    Quantity(seller_goods[i]),
                )?;
            }
        }
    }
    if dst_active {
        let buyer_w: Vec<i64> = buyers.iter().map(|(_, w)| *w).collect();
        let buyer_goods = prorata_distribute(&buyer_w, q_prime);
        for (i, &oid) in buyer_orders.iter().enumerate() {
            if buyer_goods[i] > 0 {
                drain_residual_bid(
                    scratch_orders,
                    scratch_accounts,
                    oid,
                    Quantity(buyer_goods[i]),
                )?;
            }
        }
    }
    Ok(())
}

/// STEP H/I + assembly: the per-interval macro flow over dormant AND active markets.
/// Interval-gated, conditional-clone (no clone on a quiescent interval), atomic
/// clone-validate-apply with per-edge settle-fault isolation, then commit + emit.
#[allow(clippy::too_many_arguments)]
pub fn run_macro_flow_at_tick(
    accounts: &mut AccountBook,
    inventory: &mut InventoryBook,
    ledger: &mut TradeLedger,
    demand: &DemandPools,
    supply: &SupplyPools,
    input_pools: &mut InputPools,
    policies: &ProducerPolicies,
    capita_factor: i64,
    market_goods: &mut MarketGoods,
    dirty: &DirtyMarketGoods,
    dormant: &BTreeSet<MarketId>,
    distances: &MarketDistances,
    config: &EconomyConfig,
    current_tick: u64,
    shipments: &mut crate::economy::FlowShipments,
    next_shipment_id: &mut crate::economy::NextShipmentId,
    realized: &mut crate::economy::RealizedFlows,
    orders: &mut crate::economy::OrderBook,
    next_order_id: &mut crate::economy::NextOrderId,
    receipts: &mut BTreeMap<(EconomicActorId, MarketId), Money>,
    outlays: &mut BTreeMap<(EconomicActorId, MarketId), Money>,
) -> Result<(), EconomyError> {
    realized.0.clear();
    if config.macro_flow_interval_ticks == 0
        || !current_tick.is_multiple_of(config.macro_flow_interval_ticks)
    {
        return Ok(());
    }

    // STEP A-D against the LIVE books, read-only. Skip keys still settling under
    // the auction (handoff skip-guard §5): a (market,good) currently dirty is
    // dropped from the dormant bucket set.
    let buckets = build_macro_buckets(
        accounts,
        inventory,
        demand,
        supply,
        input_pools,
        policies,
        capita_factor,
        current_tick,
        market_goods,
        dormant,
        config,
        &*orders,
        config.drain_active_residual,
    )?;
    let buckets: BTreeMap<MarketGoodKey, MacroBucket> = buckets
        .into_iter()
        .filter(|(key, _)| !dirty.0.contains(key))
        .collect();
    let mut candidates = build_candidates(&buckets, distances, config)?;
    sort_candidates(&mut candidates);
    let flows = plan_flows(&candidates, &buckets);
    if flows.is_empty() {
        return Ok(()); // truly-quiescent interval: NO clone.
    }

    // Atomic boundary: mutate clones, commit on success.
    let mut next_accounts = accounts.clone();
    let mut next_inventory = inventory.clone();
    let mut next_goods = market_goods.clone();
    // S1: carry the OrderBook + id counter through the SAME atomic boundary so S3's
    // residual-drain mutation lands inside the per-edge fault isolation below, not
    // outside it. S1 mutates neither — both round-trip unchanged (guarded by test).
    let mut next_orders = orders.clone();
    let next_oid = *next_order_id;
    let mut next_receipts = receipts.clone();
    let mut next_outlays = outlays.clone();
    let mut events: Vec<EconomyEvent> = Vec::new();

    // Per-market effective demand/supply for the write-back residuals (bucket-time).
    let effective = |market: MarketId, good: GoodId| -> (i64, i64) {
        match buckets.get(&MarketGoodKey { market, good }) {
            Some(b) => (b.total_demand(), b.total_supply()),
            None => (0, 0),
        }
    };

    for flow in &flows {
        let src_bucket = buckets.get(&MarketGoodKey {
            market: flow.src,
            good: flow.good,
        });
        let dst_bucket = buckets.get(&MarketGoodKey {
            market: flow.dst,
            good: flow.good,
        });
        let sellers = src_bucket.map(|b| b.sellers.clone()).unwrap_or_default();
        let seller_orders = src_bucket
            .map(|b| b.seller_orders.clone())
            .unwrap_or_default();
        let src_active = src_bucket.map(|b| b.intra_cleared).unwrap_or(false);
        let dst_active = dst_bucket.map(|b| b.intra_cleared).unwrap_or(false);
        let (eff_demand_src, eff_supply_src) = effective(flow.src, flow.good);
        let (eff_demand_dst_pre, eff_supply_dst) = effective(flow.dst, flow.good);

        // Stage-2 affordability prune (active dst only): drop residual bids whose
        // max_price cannot cover the landed charge; recompute q'. buyers/buyer_orders/
        // q'/eff_demand_dst then reflect the SURVIVING set (DECISION-2). A dormant dst
        // keeps the bucket buyers, q'=flow.q, eff_demand_dst=bucket-time demand — so the
        // dormant path is byte-identical to pre-S3.
        let (buyers, buyer_orders, q_prime, eff_demand_dst) = if dst_active {
            let b = dst_bucket.expect("dst_active implies a dst bucket");
            let pruned = match prune_unaffordable_buyers(
                &b.buyers,
                &b.buyer_orders,
                &b.buyer_max_prices,
                flow.q,
                flow.dist,
                flow.p_src,
                config,
            ) {
                Ok(p) => p,
                Err(reason) => {
                    events.push(EconomyEvent::MarketClearFailed {
                        market: flow.dst,
                        good: flow.good,
                        reason,
                    });
                    continue;
                }
            };
            let eff: i64 = pruned.buyers.iter().map(|(_, q)| *q).sum();
            (pruned.buyers, pruned.orders, pruned.q_prime, eff)
        } else {
            let buyers = dst_bucket.map(|b| b.buyers.clone()).unwrap_or_default();
            (buyers, Vec::new(), flow.q, eff_demand_dst_pre)
        };

        if q_prime <= 0 {
            continue; // no affordable demand survived -> skip the edge entirely.
        }

        // q' threads to the drain, settle, write-back residual, and the shipment.
        let mut eflow = flow.clone();
        eflow.q = q_prime;

        // STEP H fault isolation: drain active endpoints + settle into scratch clones;
        // fold back only on success, else emit MarketClearFailed and drop the scratch
        // (books + OrderBook byte-identical).
        let mut scratch_accounts = next_accounts.clone();
        let mut scratch_inventory = next_inventory.clone();
        let mut scratch_goods = next_goods.clone();
        let mut scratch_orders = next_orders.clone();
        let mut scratch_receipts = next_receipts.clone();
        let mut scratch_outlays = next_outlays.clone();

        if let Err(reason) = drain_active_endpoints(
            &mut scratch_orders,
            &mut scratch_inventory,
            &mut scratch_accounts,
            &eflow,
            &sellers,
            &seller_orders,
            &buyers,
            &buyer_orders,
            src_active,
            dst_active,
        ) {
            events.push(EconomyEvent::MarketClearFailed {
                market: flow.dst,
                good: flow.good,
                reason,
            });
            continue;
        }

        match settle_flow_with_receipts(
            &mut scratch_accounts,
            &mut scratch_inventory,
            &mut scratch_goods,
            &eflow,
            &sellers,
            &buyers,
            eff_demand_src,
            eff_supply_src,
            eff_demand_dst,
            eff_supply_dst,
            config,
            current_tick,
            src_active,
            dst_active,
            &mut scratch_receipts,
            &mut scratch_outlays,
        ) {
            Ok(event) => {
                next_accounts = scratch_accounts;
                next_inventory = scratch_inventory;
                next_goods = scratch_goods;
                next_orders = scratch_orders;
                next_receipts = scratch_receipts;
                next_outlays = scratch_outlays;
                if eflow.src != eflow.dst {
                    let id = next_shipment_id.next();
                    let travel_ticks =
                        crate::economy::flow_shipments::shipment_travel_ticks(eflow.dist, config);
                    shipments.0.insert(
                        id,
                        crate::economy::FlowShipment {
                            id,
                            from_market: eflow.src,
                            to_market: eflow.dst,
                            good: eflow.good,
                            qty: crate::economy::Quantity(eflow.q),
                            start_tick: current_tick,
                            travel_ticks,
                        },
                    );
                }
                if eflow.q > 0 {
                    realized.0.push(crate::economy::RealizedFlow {
                        src: eflow.src,
                        dst: eflow.dst,
                        good: eflow.good,
                        qty: eflow.q,
                        p_src: eflow.p_src,
                        p_dst: eflow.p_dst,
                        dist: eflow.dist,
                    });
                }
                events.push(event);
            }
            Err(reason) => {
                events.push(EconomyEvent::MarketClearFailed {
                    market: flow.dst,
                    good: flow.good,
                    reason,
                });
            }
        }
    }

    *accounts = next_accounts;
    *inventory = next_inventory;
    *market_goods = next_goods;
    *orders = next_orders;
    *next_order_id = next_oid;
    *receipts = next_receipts;
    *outlays = next_outlays;
    ledger.0.extend(events);
    Ok(())
}
