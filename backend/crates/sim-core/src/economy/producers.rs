//! Firms-as-buyers: producer policies (θ-dividend + working-capital target) and
//! Leontief input pools. Spec: docs/superpowers/specs/2026-06-10-economy-production-chains-design.md
//! Grounding: Caiani et al. (2016) dividend share θ + liquidity buffer;
//! Carvalho & Tahbaz-Salehi (2019) fixed-coefficient input demand.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::pools::{affordable_qty, interval_elapsed};
use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, EconomyConfig, EconomyError, EconomyEvent,
    GoodId, InventoryBook, MarketGoodKey, MarketGoods, MarketId, Money, NextOrderId, OrderBook,
    Quantity, TradeLedger, checked_order_value, create_bid,
};

/// Authored payout policy per producer. NOT persisted — re-applied from the
/// markets layer at every start (the #83 lesson: config must not silently
/// revert to defaults on restart). Actors absent from this map keep the #75
/// behavior exactly: theta_bps = 10_000, wc_target = 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProducerPolicy {
    pub theta_bps: u16,      // validated 0..=10_000 at seed
    pub batches_target: u32, // validated >= 1 at seed; ONE knob: stock AND cash target
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ProducerPolicies(pub BTreeMap<EconomicActorId, ProducerPolicy>);

/// Leontief input pool: derived demand for one producer's input good at its home
/// market. `max_price` is the participation bound, rewritten every generation pass
/// (§5.4 of the spec); `last_generated_tick` is the only true state (persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputPool {
    pub actor: EconomicActorId,
    pub market: MarketId,
    pub good: GoodId,
    pub in_qty: Quantity, // recipe input per batch (denormalized from ProductionPools at seed)
    pub out_qty: Quantity, // recipe output per batch (for the participation bound)
    pub out_good: GoodId, // whose reference price bounds the bid
    pub interval_ticks: u64,
    pub last_generated_tick: Option<u64>,
    pub max_price: Money, // last computed participation bound (telemetry + order price)
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct InputPools(pub BTreeMap<EconomicActorId, InputPool>);

/// Working-capital target in Money: expected input cost of `batches_target` batches
/// at the current participation bound. Computed with the SAME scale arithmetic as
/// order settlement (`checked_order_value`, `price·qty / ECONOMY_SCALE` in i128),
/// so the buffer for n batches equals the settle value of buying them. The batch
/// quantity is capita-scaled (`×capita_factor`, factor 1 byte-identical) because
/// production consumes `in_qty·capita_factor` per batch (production.rs) — the buffer
/// must cover the SCALED batches or the dividend drains the firm's input budget.
/// A non-positive `max_price` surfaces loudly as `NegativeMoney`/`ZeroPrice` (a
/// seeded pool's participation bound is validated/recomputed > 0 — anything else
/// is a seed or generation bug).
///
/// **Callers must guard `max_price > 0` before invoking.** The dividend path
/// (`run_distribute_profit_at_tick` in wages.rs) checks `pool.max_price.0 <= 0` and
/// retains conservatively rather than calling this function, so `wc_target` itself
/// stays strict: any `max_price <= 0` reaching it is a caller bug, not a recoverable
/// state.
pub(crate) fn wc_target(
    policy: ProducerPolicy,
    pool: &InputPool,
    capita_factor: i64,
) -> Result<Money, EconomyError> {
    let batch_qty = i64::try_from(
        i128::from(policy.batches_target)
            * (pool.in_qty.0 as i128)
            * (capita_factor.max(1) as i128),
    )
    .map_err(|_| EconomyError::Overflow)?;
    checked_order_value(pool.max_price, Quantity(batch_qty))
}

/// Participation bound (§5.4): never bid more per input unit than the expected
/// output covers AFTER the labor share — keeps the wage flow payable at any
/// accepted price.
///
/// Formula: `floor(p_out_ref * (10_000 − labor_share_bps) / 10_000 * out_qty / in_qty)`
///
/// **Double-floor note:** the integer division `/10_000` floors the labor-adjusted
/// output price before multiplying by `out_qty/in_qty`, and then `/in_qty` floors
/// again. This is intentional and conservative (the bound never over-estimates what
/// the output covers), consistent with the "never overbid" doctrine. Callers that
/// need the exact rational bound can reconstruct it, but for order placement the
/// conservative floor is correct.
///
/// A zero or negative reference price is an honest `ZeroPrice` error (propagated),
/// never a default — every output market is seeded with a positive opening price.
/// Zero or negative `in_qty` / `out_qty` is `InvalidOrder` (a seed bug).
pub(crate) fn participation_bound(
    p_out_ref: Money,
    labor_share_bps: i128,
    out_qty: Quantity,
    in_qty: Quantity,
) -> Result<Money, EconomyError> {
    if p_out_ref.0 <= 0 {
        return Err(EconomyError::ZeroPrice);
    }
    if in_qty.0 <= 0 || out_qty.0 <= 0 {
        return Err(EconomyError::InvalidOrder);
    }
    let raw = (p_out_ref.0 as i128) * (10_000 - labor_share_bps) / 10_000 * (out_qty.0 as i128)
        / (in_qty.0 as i128);
    Ok(Money(
        i64::try_from(raw).map_err(|_| EconomyError::Overflow)?,
    ))
}

/// Leontief derived demand: for each input pool (keys-first iteration), rewrite
/// `max_price` from the participation bound (§5.4), size `desired = batches_target *
/// in_qty * capita_factor − held` (floored at 0), cap by affordability, and place a
/// bid via the SAME `create_bid` path as consumer demand pools. The target is
/// capita-scaled (factor 1 byte-identical) because production consumes
/// `in_qty·capita_factor` per batch (production.rs) — unscaled orders would top out
/// below one scaled batch and production would never fire.
///
/// Mirrors the structure of `generate_pool_orders_at_tick`:
/// - dormant-market skip (cursor untouched)
/// - interval guard (cursor untouched)
/// - `OrderRejected { reason: InsufficientFunds }` when affordable == 0 (cash = 0).
///   When `max_price.0 <= 0` after the bound computation the `if desired.0 > 0 &&
///   pool.max_price.0 > 0` guard silently skips ordering for this interval; the cursor
///   IS stamped so the next generation fires on schedule. This is not an error: it means
///   the output reference price dropped to zero (market structurally unviable), which
///   will surface downstream as starved production. Price signals can recover next
///   interval when the output price rises.
/// - cursor (`last_generated_tick`) stamped after the bid/rejection/stocked-skip
///
/// **Mismatch invariant (fail-fast):** every `InputPool` entry MUST have a matching
/// `ProducerPolicy`. An absent policy is `Err(EconomyError::InvalidOrder)` — same
/// doctrine as `run_distribute_profit_at_tick` (wages.rs): partial state is a
/// config-revert (#83 class), surfaces as an error rather than a silent default.
///
/// **Zero desired = fully stocked:** when `held >= batches_target * in_qty`, `desired`
/// is floored to 0 and the pool is skipped (no bid, no OrderRejected event). The
/// cursor IS stamped so the next generation fires on schedule.
#[allow(clippy::too_many_arguments)]
pub fn run_generate_input_orders_at_tick(
    accounts: &mut AccountBook,
    orders: &mut OrderBook,
    inventory: &InventoryBook,
    ledger: &mut TradeLedger,
    dirty: &mut DirtyMarketGoods,
    next: &mut NextOrderId,
    input_pools: &mut InputPools,
    policies: &ProducerPolicies,
    market_goods: &MarketGoods,
    config: &EconomyConfig,
    current_tick: u64,
    ttl_ticks: u64,
    dormant: &BTreeSet<MarketId>,
    capita_factor: i64,
) -> Result<(), EconomyError> {
    let labor_share = config.validated_labor_share_bps()?;
    let actors: Vec<EconomicActorId> = input_pools.0.keys().copied().collect();
    for actor in actors {
        let mut pool = input_pools.0[&actor];

        // dormant-market skip: no orders, cursor untouched
        if dormant.contains(&pool.market) {
            continue;
        }
        // interval guard: not yet due, cursor untouched
        if !interval_elapsed(pool.last_generated_tick, current_tick, pool.interval_ticks) {
            continue;
        }

        // Mismatch fail-fast: InputPool without matching ProducerPolicy is a config bug.
        let policy = policies
            .0
            .get(&actor)
            .copied()
            .ok_or(EconomyError::InvalidOrder)?;

        // Participation bound from output reference price.
        let p_out_ref = market_goods
            .0
            .get(&MarketGoodKey {
                market: pool.market,
                good: pool.out_good,
            })
            .ok_or(EconomyError::ZeroPrice)?
            .ewma_reference_price;
        pool.max_price = participation_bound(p_out_ref, labor_share, pool.out_qty, pool.in_qty)?;

        // Leontief desired quantity: batches_target * in_qty * capita_factor − held
        // (floored at 0). Same i128 scale pattern as pools.rs / production.rs.
        let target_qty = Quantity(
            i64::try_from(
                i128::from(policy.batches_target)
                    * (pool.in_qty.0 as i128)
                    * (capita_factor.max(1) as i128),
            )
            .map_err(|_| EconomyError::Overflow)?,
        );
        let held = inventory.balance(actor, pool.good).available;
        let desired = Quantity((target_qty.0 - held.0).max(0));

        if desired.0 > 0 && pool.max_price.0 > 0 {
            // Cap by what the actor can afford at max_price.
            let affordable = affordable_qty(accounts.account(actor).available, pool.max_price)?;
            let capped = Quantity(desired.0.min(affordable.0));
            if capped.0 <= 0 {
                ledger.0.push(EconomyEvent::OrderRejected {
                    actor,
                    market: pool.market,
                    good: pool.good,
                    reason: EconomyError::InsufficientFunds,
                });
            } else {
                create_bid(
                    accounts,
                    orders,
                    ledger,
                    dirty,
                    next,
                    current_tick,
                    actor,
                    pool.market,
                    pool.good,
                    capped,
                    pool.max_price,
                    ttl_ticks,
                )?;
            }
        }
        // Stamp cursor: stocked (desired==0) OR bid/rejected — all paths stamp.
        pool.last_generated_tick = Some(current_tick);
        input_pools.0.insert(actor, pool);
    }
    Ok(())
}
