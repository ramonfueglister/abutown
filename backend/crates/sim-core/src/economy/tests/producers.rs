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
    MarketGoodState {
        key,
        last_settlement_price: Money(1_000),
        ewma_reference_price: Money(1_000),
        traded_qty_last_tick: Quantity(0),
        unmet_demand_last_tick: Quantity(0),
        unsold_supply_last_tick: Quantity(0),
        consumed_qty_last_tick: Quantity::ZERO,
        dirty: false,
        last_cleared_tick: 0,
    }
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
    );

    assert_eq!(
        result,
        Err(crate::economy::EconomyError::InvalidOrder),
        "InputPool without matching ProducerPolicy must fail fast (got {:?})",
        result
    );
}
