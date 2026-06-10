use std::collections::BTreeSet;

use crate::economy::producers::{InputPool, InputPools, ProducerPolicies, ProducerPolicy};
use crate::economy::producers::{participation_bound, run_generate_input_orders_at_tick};
use crate::economy::{
    AccountBook, DirtyMarketGoods, EconomicActorId, EconomyError, EconomyEvent, GOOD_TOOLS,
    GOOD_WOOD, InventoryBook, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Money,
    NextOrderId, OrderBook, Quantity, TradeLedger,
};

// ── Shared fixture ─────────────────────────────────────────────────────────────

fn input_pool(actor: EconomicActorId, market: MarketId) -> InputPool {
    InputPool {
        actor,
        market,
        good: GOOD_WOOD,       // input good: WOOD
        in_qty: Quantity(10),  // 10 WOOD per batch
        out_qty: Quantity(10), // 10 TOOLS per batch
        out_good: GOOD_TOOLS,  // output whose price bounds the bid
        interval_ticks: 1,
        last_generated_tick: None,
        max_price: Money(0), // rewritten by run_generate_input_orders_at_tick
    }
}

fn producer_policy() -> ProducerPolicy {
    ProducerPolicy {
        theta_bps: 6_000,
        batches_target: 2,
    }
}

fn seeded_market_good(market: MarketId) -> MarketGoodState {
    let key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    let mut state = MarketGoodState::new(key);
    state.last_settlement_price = Money(1_000);
    state.ewma_reference_price = Money(1_000);
    state
}

// ── participation_bound pure tests ────────────────────────────────────────────

/// §5.4 formula check:
/// floor(p_out_ref * (10_000 − labor_share) / 10_000 * out_qty / in_qty)
/// = floor(1000 * 4000/10_000 * 10/10) = floor(400) = 400
#[test]
fn participation_bound_formula() {
    let p_out_ref = Money(1_000);
    let labor_share_bps: i128 = 6_000;
    let out_qty = Quantity(10);
    let in_qty = Quantity(10);
    let bound = participation_bound(p_out_ref, labor_share_bps, out_qty, in_qty).unwrap();
    assert_eq!(bound, Money(400));
}

/// Zero reference price is an honest ZeroPrice error — no silent default.
#[test]
fn zero_reference_price_is_honest_error() {
    let result = participation_bound(Money(0), 6_000, Quantity(10), Quantity(10));
    assert_eq!(result, Err(EconomyError::ZeroPrice));
}

// ── run_generate_input_orders_at_tick integration tests ───────────────────────

/// batches_target=2, in_qty=10, held=5 → desired = 2*10 − 5 = 15
/// The bid is placed via create_bid with qty=15 (subject to affordability; funded with large cash).
#[test]
fn input_order_sizes_to_batches_target_minus_held() {
    let actor = EconomicActorId(9_001);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default(); // labor_share_bps=6_000

    // Seed funds (large enough to afford desired=15 at bound=400: need 15*400/1000=6)
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    // Seed held inventory: 5 WOOD in hand
    inventory.deposit(actor, GOOD_WOOD, Quantity(5)).unwrap();

    // Seed the output's ewma_reference_price so participation_bound can read it
    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy()); // batches_target=2
    input_pools.0.insert(actor, input_pool(actor, market));

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        1,
    )
    .unwrap();

    // Exactly one bid placed with qty = desired = 15
    assert_eq!(orders.bids.len(), 1, "exactly one bid placed");
    let bid = orders.bids.values().next().unwrap();
    assert_eq!(
        bid.qty_remaining,
        Quantity(15),
        "bid qty = batches_target*in_qty - held = 15"
    );

    // cursor stamped
    assert_eq!(
        input_pools.0[&actor].last_generated_tick,
        Some(1),
        "cursor stamped on tick 1"
    );
}

/// Per-capita scaling: at capita_factor=3 the Leontief target is
/// batches_target*in_qty*3 = 2*10*3 = 60, so with held=5 the bid is 55. Production
/// consumes in_qty×factor per batch (production.rs), so an unscaled order target
/// would starve it — this is the live-world inertness regression (factor ~30).
#[test]
fn input_order_scales_target_by_capita_factor() {
    let actor = EconomicActorId(9_005);
    let market = MarketId(5);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    // Seed funds (large enough to afford desired=55 at bound=400: need 55*400/1000=22)
    accounts.deposit(actor, Money(1_000_000)).unwrap();
    inventory.deposit(actor, GOOD_WOOD, Quantity(5)).unwrap();

    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy()); // batches_target=2
    input_pools.0.insert(actor, input_pool(actor, market));

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        3, // capita_factor
    )
    .unwrap();

    assert_eq!(orders.bids.len(), 1, "exactly one bid placed");
    let bid = orders.bids.values().next().unwrap();
    assert_eq!(
        bid.qty_remaining,
        Quantity(55),
        "bid qty = batches_target*in_qty*capita_factor - held = 2*10*3 - 5 = 55"
    );
}

/// held = 20 >= batches_target*in_qty = 2*10 = 20 → desired = 0 → no bid, no OrderRejected,
/// but cursor is still stamped.
#[test]
fn input_order_skipped_when_stocked() {
    let actor = EconomicActorId(9_002);
    let market = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    accounts.deposit(actor, Money(1_000_000)).unwrap();
    // held = 20 = batches_target * in_qty → desired = 0
    inventory.deposit(actor, GOOD_WOOD, Quantity(20)).unwrap();

    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy());
    input_pools.0.insert(actor, input_pool(actor, market));

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        1,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "no bid placed when fully stocked");
    let rejected = ledger
        .0
        .iter()
        .any(|e| matches!(e, EconomyEvent::OrderRejected { .. }));
    assert!(
        !rejected,
        "no OrderRejected when desired == 0 (fully stocked)"
    );
    // cursor still stamped even when skipping because stocked
    assert_eq!(
        input_pools.0[&actor].last_generated_tick,
        Some(1),
        "cursor stamped even when stocked"
    );
}

/// cash = 0 → affordable = 0 → OrderRejected with reason InsufficientFunds in ledger.
#[test]
fn input_order_rejected_without_funds() {
    let actor = EconomicActorId(9_003);
    let market = MarketId(3);
    let mut accounts = AccountBook::default();
    let inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    // cash = 0 (no deposit)
    // held = 0 → desired = 20
    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy());
    input_pools.0.insert(actor, input_pool(actor, market));

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        1,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "no bid placed when cash=0");
    let rejected = ledger.0.iter().any(|e| {
        matches!(
            e,
            EconomyEvent::OrderRejected { actor: a, reason, .. }
                if *a == actor && *reason == EconomyError::InsufficientFunds
        )
    });
    assert!(
        rejected,
        "OrderRejected(InsufficientFunds) must be in ledger"
    );

    // cursor stamped even after rejection
    assert_eq!(
        input_pools.0[&actor].last_generated_tick,
        Some(1),
        "cursor stamped even after OrderRejected"
    );
}

/// An `InputPool` entry without a matching `ProducerPolicies` entry is a config-revert
/// (#83 class). `run_generate_input_orders_at_tick` must fail fast with
/// `Err(EconomyError::InvalidOrder)` — same fail-fast doctrine as the dividend path.
#[test]
fn input_pool_without_policy_errors_loudly() {
    let actor = EconomicActorId(9_004);
    let market = MarketId(4);
    let mut accounts = AccountBook::default();
    let inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    accounts.deposit(actor, Money(1_000_000)).unwrap();
    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    // InputPool present for actor — but NO ProducerPolicies entry.
    input_pools.0.insert(actor, input_pool(actor, market));
    let policies = ProducerPolicies::default(); // intentionally empty

    let result = run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        1,
    );

    assert_eq!(
        result,
        Err(crate::economy::EconomyError::InvalidOrder),
        "InputPool without matching ProducerPolicy must fail fast (got {:?})",
        result
    );
}

/// Floor-to-zero participation bound on the ORDER side (producers.rs doc §"When
/// `max_price.0 <= 0` after the bound computation"): a positive-but-tiny output
/// reference price floors the bound to 0 — `floor(1 · 4_000/10_000) = 0` — which
/// must silently skip ordering for this interval: NO bid, NO `OrderRejected`
/// (the bound flooring is not an error; a zero reference price itself would be
/// `Err(ZeroPrice)`), the floored bound IS written back to the pool (telemetry
/// truth), and the cursor IS stamped so the next generation fires on schedule.
#[test]
fn tiny_reference_price_floors_bound_to_zero_skips_order_but_stamps_cursor() {
    let actor = EconomicActorId(9_006);
    let market = MarketId(6);
    let mut accounts = AccountBook::default();
    let inventory = InventoryBook::default(); // held = 0 → desired = 20 > 0
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default(); // labor_share_bps=6_000

    accounts.deposit(actor, Money(1_000_000)).unwrap();

    // Positive but tiny reference price: bound = floor(1·4_000/10_000)·10/10 = 0.
    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    let mut state = MarketGoodState::new(out_key);
    state.ewma_reference_price = Money(1);
    state.last_settlement_price = Money(1);
    market_goods.0.insert(out_key, state);

    policies.0.insert(actor, producer_policy());
    input_pools.0.insert(actor, input_pool(actor, market));

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        1,
    )
    .unwrap();

    assert!(
        orders.bids.is_empty(),
        "bound floored to 0 → no bid this interval (starved production recovers \
         next interval when the output price rises)"
    );
    assert!(
        ledger.0.is_empty(),
        "bound-zero is NOT an error: no OrderRejected, no event at all — got {:?}",
        ledger.0
    );
    let pool = input_pools.0[&actor];
    assert_eq!(
        pool.max_price,
        Money(0),
        "the floored bound is written back to the pool (the wire telemetry must \
         show the honest 0, and the dividend path's unpriced guard keys off it)"
    );
    assert_eq!(
        pool.last_generated_tick,
        Some(1),
        "cursor IS stamped on the bound-zero skip — the next generation must fire \
         on schedule, not retry every tick"
    );
}

/// Dormant-market skip — this pins the ORDER path only
/// (`run_generate_input_orders_at_tick`): the input pool must be left COMPLETELY
/// untouched by it — no bid, no event, cursor NOT stamped, bound NOT rewritten
/// (mirrors the consumer-pool dormant contract). Dormant pools are instead
/// expressed (and their bound/cursor written) by the MACRO-FLOW path
/// (`build_macro_buckets`' InputPools loop, macro_flow.rs) on the macro cadence —
/// see `build_macro_buckets_sources_dormant_input_pool_demand` in
/// tests/macro_flow.rs.
#[test]
fn input_order_dormant_market_leaves_pool_untouched() {
    let actor = EconomicActorId(9_007);
    let market = MarketId(7);
    let mut accounts = AccountBook::default();
    let inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    accounts.deposit(actor, Money(1_000_000)).unwrap();
    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy());
    let original = input_pool(actor, market);
    input_pools.0.insert(actor, original);

    let dormant: BTreeSet<MarketId> = [market].into_iter().collect();
    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &dormant,
        1,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "dormant market: no bid");
    assert!(ledger.0.is_empty(), "dormant market: no events");
    assert_eq!(
        input_pools.0[&actor], original,
        "dormant market: the pool must be byte-identical — cursor untouched (None) \
         and the bound NOT rewritten, so the pool fires immediately when the \
         market wakes"
    );
}

/// Interval-not-elapsed skip: with `interval_ticks = 5` and a cursor at tick 10, a
/// generation pass at tick 12 must leave the pool untouched (cursor stays Some(10),
/// bound not rewritten), place no bid and emit no event — the within-interval skip
/// is a true no-op, exactly like the consumer-pool counterpart.
#[test]
fn input_order_within_interval_leaves_pool_untouched() {
    let actor = EconomicActorId(9_008);
    let market = MarketId(8);
    let mut accounts = AccountBook::default();
    let inventory = InventoryBook::default();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    accounts.deposit(actor, Money(1_000_000)).unwrap();
    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy());
    let mut pool = input_pool(actor, market);
    pool.interval_ticks = 5;
    pool.last_generated_tick = Some(10);
    input_pools.0.insert(actor, pool);

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        12, // 12 − 10 = 2 < interval 5 → not yet due
        10,
        &BTreeSet::new(),
        1,
    )
    .unwrap();

    assert!(orders.bids.is_empty(), "within interval: no bid");
    assert!(ledger.0.is_empty(), "within interval: no events");
    assert_eq!(
        input_pools.0[&actor], pool,
        "within interval: the pool must be byte-identical — cursor stays Some(10) \
         and the bound is NOT rewritten"
    );
}

/// Partial affordability: `0 < affordable < desired` → the bid is capped to exactly
/// the affordable quantity (no OrderRejected — the rejection event is reserved for
/// affordable == 0). cash 3 at bound 400 affords floor(3·1_000/400) = 7 units of the
/// desired 20; the bid locks the order value floor(400·7/1_000) = 2.
#[test]
fn input_order_partial_affordability_caps_bid_to_affordable() {
    let actor = EconomicActorId(9_009);
    let market = MarketId(9);
    let mut accounts = AccountBook::default();
    let inventory = InventoryBook::default(); // held = 0 → desired = 2·10 = 20
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut input_pools = InputPools::default();
    let mut policies = ProducerPolicies::default();
    let mut market_goods = MarketGoods::default();
    let config = crate::economy::EconomyConfig::default();

    accounts.deposit(actor, Money(3)).unwrap(); // affords 7 of 20 at bound 400

    let out_key = MarketGoodKey {
        market,
        good: GOOD_TOOLS,
    };
    market_goods.0.insert(out_key, seeded_market_good(market));

    policies.0.insert(actor, producer_policy());
    input_pools.0.insert(actor, input_pool(actor, market));

    run_generate_input_orders_at_tick(
        &mut accounts,
        &mut orders,
        &inventory,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut input_pools,
        &policies,
        &market_goods,
        &config,
        1,
        10,
        &BTreeSet::new(),
        1,
    )
    .unwrap();

    assert_eq!(orders.bids.len(), 1, "exactly one (capped) bid placed");
    let bid = orders.bids.values().next().unwrap();
    assert_eq!(
        bid.qty_remaining,
        Quantity(7),
        "bid qty must be the affordable cap: floor(cash 3 · SCALE 1_000 / bound 400) \
         = 7 < desired 20"
    );
    assert_eq!(bid.max_price, Money(400), "bid priced at the bound");
    assert_eq!(
        accounts.account(actor).locked,
        Money(2),
        "the bid locks exactly the order value floor(400·7/1_000) = 2"
    );
    assert!(
        !ledger
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::OrderRejected { .. })),
        "partial affordability is NOT a rejection — OrderRejected is reserved for \
         affordable == 0"
    );
    assert_eq!(
        input_pools.0[&actor].last_generated_tick,
        Some(1),
        "cursor stamped after the capped bid"
    );
}
