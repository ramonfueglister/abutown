//! Per-capita overflow stress tests (slice 2e).
//!
//! Covers two concerns:
//!   1. **Ceiling fail-fast**: every scaled-multiply formula that uses `capita_factor`
//!      returns `Err(EconomyError::Overflow)` at the ceiling — NOT a panic, NOT a wrap.
//!   2. **Below-ceiling conservation**: at the realistic target (factor ~30, ~300
//!      citizens / baseline 10) a multi-tick full-plugin run stays byte-invariant on
//!      `total_money` every tick.
//!
//! Formulas under test (all via i128-intermediate + `i64::try_from`):
//!   - `target_spend` (`pools.rs`): `autonomous * capita_factor`
//!   - `generate_pool_orders_at_tick` (`pools.rs`): `offered_qty_per_tick * capita_factor`
//!   - `run_regen_at_tick` (`production.rs`): `qty_per_interval * capita_factor`
//!   - `run_production_at_tick` (`production.rs`): recipe `qty * capita_factor`
//!   - `capita_factor` (`capita.rs`): saturates at `i64::MAX` via `unwrap_or(i64::MAX)`
//!
//! Factor ~30 at ~300 citizens (baseline 10) is far below the i64 ceiling:
//!   max representable value ≈ 9.2 × 10^18; at factor 30 the largest scaled quantity
//!   in the demo seed is 10 * 30 = 300, trivially safe.

use std::collections::BTreeSet;

use crate::economy::capita::capita_factor;
use crate::economy::pools::target_spend;
use crate::economy::production::{
    EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit, RawDeposits,
    Recipe, run_production_at_tick, run_regen_at_tick,
};
use crate::economy::{
    AccountBook, DemandPool, DemandPools, DirtyMarketGoods, EconomicActorId, EconomyError,
    EconomyEvent, GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, HouseholdSector, InventoryBook, MarketGoodKey,
    MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, Money, NextOrderId, OrderBook,
    Quantity, SupplyPool, SupplyPools, TradeLedger, generate_pool_orders_at_tick,
};

// ── 1. capita_factor saturation ──────────────────────────────────────────────

/// `capita_factor(u64::MAX, 1)` must saturate to `i64::MAX`, NOT panic.
/// The raw i128 value `(u64::MAX as i128) / 1` = 1.844×10^19, which overflows i64.
/// The implementation uses `.unwrap_or(i64::MAX)` to clamp instead of panicking.
#[test]
fn capita_factor_u64_max_saturates_to_i64_max() {
    assert_eq!(
        capita_factor(u64::MAX, 1),
        i64::MAX,
        "u64::MAX / 1 overflows i64; must saturate to i64::MAX, not panic"
    );
}

/// Below the ceiling: a small count and a baseline that yield a factor of 30
/// stay comfortably representable.
#[test]
fn capita_factor_300_citizens_baseline_10_is_30() {
    assert_eq!(capita_factor(300, 10), 30);
}

// ── 2. target_spend ceiling ───────────────────────────────────────────────────

/// `autonomous(i64::MAX) * factor(2)` overflows i64 → `Err(Overflow)`, not a panic.
/// The i128 intermediate is 2 * i64::MAX ≈ 1.84×10^19, exceeds i64::MAX.
#[test]
fn target_spend_autonomous_overflow_returns_err() {
    assert_eq!(
        target_spend(Money(i64::MAX), 0, Money::ZERO, 2),
        Err(EconomyError::Overflow),
        "autonomous(i64::MAX) * factor(2) must return Err(Overflow), not panic"
    );
}

/// `autonomous(i64::MAX/2 + 1) * factor(2)` is the minimal overflow case (one past MAX/2).
#[test]
fn target_spend_just_above_ceiling_returns_overflow() {
    assert_eq!(
        target_spend(Money(i64::MAX / 2 + 1), 0, Money::ZERO, 2),
        Err(EconomyError::Overflow),
    );
}

/// Below the ceiling: realistic factor 30 on a small autonomous spend.
/// autonomous=1_000_000, factor=30, income=10_000, mpc=8_000.
/// scaled_autonomous = 30 × 1_000_000 = 30_000_000
/// induced            = floor(8_000 × 10_000 / 10_000) = 8_000
/// total              = 30_008_000
#[test]
fn target_spend_realistic_factor_30_is_ok() {
    assert_eq!(
        target_spend(Money(1_000_000), 8_000, Money(10_000), 30),
        Ok(Money(30_008_000)),
        "factor-30 realistic spend must succeed and equal 30_008_000"
    );
}

// ── 3. generate_pool_orders_at_tick — supply-side ceiling ────────────────────

/// A supply pool with `offered_qty_per_tick = i64::MAX` and `capita_factor = 2`
/// would produce a scaled offer of 2 × i64::MAX which overflows i64.
/// The function must return `Err(EconomyError::Overflow)`, not panic.
#[test]
fn generate_pool_orders_supply_overflow_returns_err() {
    let actor = EconomicActorId(9_001);
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    // Inventory is irrelevant here: the scaled-offer multiply overflows (returns Err)
    // BEFORE the `min(available)` inventory clamp is ever reached. Present for realism only.
    inventory
        .deposit(actor, GOOD_TOOLS, Quantity(i64::MAX / 2))
        .unwrap();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    supply.0.insert(
        actor,
        SupplyPool {
            actor,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(i64::MAX),
            min_price: Money(1_000),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    let result = generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        0,
        5,
        &BTreeSet::new(),
        2, // capita_factor = 2 → i64::MAX * 2 overflows
    );
    assert_eq!(
        result,
        Err(EconomyError::Overflow),
        "offered_qty_per_tick(i64::MAX) × factor(2) must return Err(Overflow)"
    );
}

/// Below the ceiling: factor 30 with a normal offered quantity succeeds.
#[test]
fn generate_pool_orders_supply_realistic_factor_30_is_ok() {
    let actor = EconomicActorId(9_002);
    let market = MarketId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    inventory
        .deposit(actor, GOOD_TOOLS, Quantity(10_000))
        .unwrap();
    let mut orders = OrderBook::default();
    let mut ledger = TradeLedger::default();
    let mut dirty = DirtyMarketGoods::default();
    let mut next = NextOrderId::default();
    let mut demand = DemandPools::default();
    let mut supply = SupplyPools::default();
    supply.0.insert(
        actor,
        SupplyPool {
            actor,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    generate_pool_orders_at_tick(
        &mut accounts,
        &mut inventory,
        &mut orders,
        &mut ledger,
        &mut dirty,
        &mut next,
        &mut demand,
        &mut supply,
        0,
        5,
        &BTreeSet::new(),
        30, // factor 30 → scaled offer = 300, well below i64 ceiling
    )
    .expect("factor-30 supply order must succeed without overflow");

    // The ask was placed for min(300, available=10_000) = 300 units.
    let ask = orders.asks.values().next().expect("one ask placed");
    assert_eq!(ask.qty_remaining, Quantity(300));
}

// ── 4. run_regen_at_tick ceiling ─────────────────────────────────────────────

/// A deposit with `qty_per_interval = i64::MAX` and `capita_factor = 2` overflows.
/// The function must return `Err(EconomyError::Overflow)`, not panic.
#[test]
fn regen_at_tick_overflow_returns_err() {
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(i64::MAX),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );

    assert_eq!(
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0, 2),
        Err(EconomyError::Overflow),
        "qty_per_interval(i64::MAX) × factor(2) must return Err(Overflow)"
    );
}

/// Below the ceiling: factor 30 with the realistic seed qty (10) succeeds.
#[test]
fn regen_at_tick_realistic_factor_30_is_ok() {
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );

    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0, 30)
        .expect("factor-30 regen must succeed");

    // 10 × 30 = 300 RAW deposited.
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_RAW).available,
        Quantity(300)
    );
    assert!(ledger.0.contains(&EconomyEvent::Regenerated {
        actor: EXTRACTOR_TOOLS,
        good: GOOD_RAW,
        qty: Quantity(300),
    }));
}

// ── 5. run_production_at_tick ceiling ────────────────────────────────────────

/// A recipe input with `qty = i64::MAX` and `capita_factor = 2` produces a scaled qty
/// that overflows i64. The function must return `Err(EconomyError::Overflow)`.
#[test]
fn production_at_tick_input_overflow_returns_err() {
    let actor = EconomicActorId(9_003);
    let mut inv = InventoryBook::default();
    // Pre-load enough inventory so the can_produce check might try to scale.
    // We use i64::MAX / 2 (representable) for both goods.
    inv.deposit(actor, GOOD_RAW, Quantity(i64::MAX / 2))
        .unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = ProductionPools::default();
    prod.0.insert(
        actor,
        ProductionPool {
            actor,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(i64::MAX))],
                outputs: vec![(GOOD_TOOLS, Quantity(1))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    assert_eq!(
        run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0, 2),
        Err(EconomyError::Overflow),
        "recipe input qty(i64::MAX) × factor(2) must return Err(Overflow)"
    );
}

/// A recipe output with `qty = i64::MAX` and `capita_factor = 2` also overflows.
/// We trigger this by providing sufficient inputs so the input check passes, but
/// the output scale overflows.
#[test]
fn production_at_tick_output_overflow_returns_err() {
    let actor = EconomicActorId(9_004);
    let mut inv = InventoryBook::default();
    // Input qty=1 scaled by factor=2 is 2 — safe; but output qty=i64::MAX scaled by 2 overflows.
    inv.deposit(actor, GOOD_RAW, Quantity(2)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = ProductionPools::default();
    prod.0.insert(
        actor,
        ProductionPool {
            actor,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(1))],
                outputs: vec![(GOOD_TOOLS, Quantity(i64::MAX))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    assert_eq!(
        run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0, 2),
        Err(EconomyError::Overflow),
        "recipe output qty(i64::MAX) × factor(2) must return Err(Overflow)"
    );
}

/// Below the ceiling: factor 30 with the realistic seed recipe (10 RAW → 10 TOOLS) succeeds.
#[test]
fn production_at_tick_realistic_factor_30_is_ok() {
    let actor = EconomicActorId(9_005);
    let mut inv = InventoryBook::default();
    // Scaled input = 10 × 30 = 300 RAW needed.
    inv.deposit(actor, GOOD_RAW, Quantity(300)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = ProductionPools::default();
    prod.0.insert(
        actor,
        ProductionPool {
            actor,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_TOOLS, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );

    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0, 30)
        .expect("factor-30 production must succeed");

    // 300 RAW consumed → 300 TOOLS produced.
    assert_eq!(inv.balance(actor, GOOD_RAW).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(300));
}

// ── 6. System-level conservation at factor ~30 ───────────────────────────────
//
// Reuses the same full-plugin harness as `run_conservation_with_factor` in
// `capita.rs`. Runs 50 ticks at factor 30 (~300 citizens, baseline 10) and
// asserts `total_money` is byte-invariant on every tick — proving that the
// realistic per-capita scale is comfortably below any overflow ceiling AND
// conserves money end-to-end.
//
// SAFETY MARGIN NOTE: at factor 30 the largest scaled quantity in the demo
// seed is 10 × 30 = 300 (well within i64's ~9.2×10^18 range). The realistic
// ceiling is ~9.2×10^18 / 300 ≈ 3×10^16× above any value this fixture can produce.
#[test]
fn conservation_holds_at_factor_30_for_50_ticks() {
    use crate::economy::systems::EconomyConfig;
    use crate::economy::{AccountBook, EconomyPlugin, TradeLedger};
    use crate::mobility::components::AgentMarker;
    use crate::mobility::resources::Tick;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use std::collections::BTreeMap;

    // factor 30: spawn 30 AgentMarker entities with capita_baseline=1.
    let factor: i64 = 30;
    let mut world = bevy_ecs::world::World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.insert_resource(EconomyConfig {
        capita_baseline: 1,
        ..EconomyConfig::default()
    });
    for _ in 0..factor {
        world.spawn(AgentMarker);
    }

    let consumer = EconomicActorId(8_002);
    let market = MarketId(1);

    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_TOOLS,
        ProductionPool {
            actor: EXTRACTOR_TOOLS,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_TOOLS, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_TOOLS,
        SupplyPool {
            actor: EXTRACTOR_TOOLS,
            market,
            good: GOOD_TOOLS,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(10_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market,
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        },
    );
    let food_consumer = EconomicActorId(8_012);
    world.resource_mut::<RawDeposits>().0.insert(
        EXTRACTOR_FOOD_A,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        EXTRACTOR_FOOD_A,
        ProductionPool {
            actor: EXTRACTOR_FOOD_A,
            recipe: Recipe {
                inputs: vec![(GOOD_RAW, Quantity(10))],
                outputs: vec![(GOOD_FOOD, Quantity(10))],
            },
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        EXTRACTOR_FOOD_A,
        SupplyPool {
            actor: EXTRACTOR_FOOD_A,
            market,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(10),
            min_price: Money(500),
            interval_ticks: 1,
            last_generated_tick: None,
        },
    );
    world
        .resource_mut::<AccountBook>()
        .deposit(food_consumer, Money(10_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(10),
            max_price: Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: None,
            last_consumed_tick: None,
            income_last_tick: Money::ZERO,
            mpc_bps: 8_000,
            autonomous: Money(5_000),
        },
    );
    world.resource_mut::<Markets>().0.insert(
        market,
        MarketSite {
            id: market,
            node_id: crate::routing::NodeId(0),
            name: "M1".to_string(),
        },
    );
    world.insert_resource(HouseholdSector {
        population: 1_000_000,
        pool_weights: BTreeMap::from([(consumer, 1_i64), (food_consumer, 1_i64)]),
    });
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_TOOLS,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }
    {
        let key = MarketGoodKey {
            market,
            good: GOOD_FOOD,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.ewma_reference_price = Money(1_000);
        st.last_settlement_price = Money(1_000);
    }

    let money_before = world.resource::<AccountBook>().total_money().unwrap();

    // 50 ticks — byte-invariant money assertion mirrors the #78 tick audit.
    // If any conservation violation occurs the audit system panics (fail-fast).
    for i in 0..50_u64 {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i} with CapitaFactor({factor}); \
             factor-30 max scaled qty ≈ 300, far below i64::MAX (~9.2×10^18)"
        );
    }

    // Non-vacuity: at least some Regenerated events must have fired (production flowed).
    assert!(
        world
            .resource::<TradeLedger>()
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Regenerated { .. })),
        "factor-30 run must have generated goods (non-vacuous)"
    );

    println!(
        "CONSERVATION VERIFIED: factor=30, 50 ticks, total_money byte-invariant throughout. \
         Max scaled qty ≈ 10 × 30 = 300 ≪ i64::MAX ≈ 9.2×10^18. \
         Safety margin: ~3×10^16×."
    );
}
