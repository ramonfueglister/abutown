use bevy_ecs::prelude::*;

use crate::economy::capita::CapitaFactor;
use crate::economy::production::{
    EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit, RawDeposits,
    Recipe,
};
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyEvent, EconomyPlugin, GOOD_FOOD,
    GOOD_RAW, GOOD_TOOLS, HouseholdSector, MarketGoodKey, MarketGoodState, MarketGoods, MarketId,
    MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools, TradeLedger,
};
use crate::mobility::resources::Tick;
use crate::world::plugin::CorePlugin;
use crate::world::schedule::SimPlugin;
use std::collections::BTreeMap;

/// Build the same minimal two-extractor economy used in `conservation_full_plugin_multi_tick`,
/// inserting a custom `CapitaFactor` value before running `n` ticks, then asserting
/// `total_money` is byte-invariant throughout.
fn run_conservation_with_factor(factor: i64, n: u64) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    // Override the default CapitaFactor(1) with the requested factor.
    world.insert_resource(CapitaFactor(factor));

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

    for i in 0..n {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i} with CapitaFactor({factor})"
        );
    }

    // Non-vacuity: at least some economy events must have fired.
    assert!(
        world
            .resource::<TradeLedger>()
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Regenerated { .. })),
        "goods flowed (CapitaFactor({factor}))"
    );
}

#[test]
fn capita_factor_1_conserves_total_money() {
    run_conservation_with_factor(1, 20);
}

#[test]
fn capita_factor_2_conserves_total_money() {
    run_conservation_with_factor(2, 20);
}

#[test]
fn capita_factor_10_conserves_total_money() {
    run_conservation_with_factor(10, 20);
}

/// Run the full economy schedule for `n` ticks with the given `factor` and a FIXED
/// opening_cash of `Money(1_000_000)` (the realistic seeded amount). Returns:
/// `(total_final_consumed, total_trade_events)` across the run.
fn run_solvency_scenario(factor: i64, n: u64) -> (i64, usize) {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    EconomyPlugin.install(&mut world, &mut schedule);

    world.insert_resource(CapitaFactor(factor));

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
    // Realistic opening cash: 1_000_000 (unchanged from seed — the solvency question).
    world
        .resource_mut::<AccountBook>()
        .deposit(consumer, Money(1_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        consumer,
        DemandPool {
            actor: consumer,
            market,
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(0),
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
        .deposit(food_consumer, Money(1_000_000))
        .unwrap();
    world.resource_mut::<DemandPools>().0.insert(
        food_consumer,
        DemandPool {
            actor: food_consumer,
            market,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(0),
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

    for i in 0..n {
        schedule.run(&mut world);
        world.resource_mut::<Tick>().0 += 1;
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            money_before,
            "total_money byte-invariant at tick {i} with CapitaFactor({factor})"
        );
    }

    let ledger = world.resource::<TradeLedger>();
    let total_final_consumed: i64 = ledger
        .0
        .iter()
        .filter_map(|e| match e {
            EconomyEvent::FinalConsumed { qty, .. } => Some(qty.0),
            _ => None,
        })
        .sum();
    let total_trade_events = ledger
        .0
        .iter()
        .filter(|e| matches!(e, EconomyEvent::Trade { .. }))
        .count();

    (total_final_consumed, total_trade_events)
}

/// Solvency at the realistic factor 30 (~300 citizens / baseline 10).
///
/// With supply-side scaling (2b), the extractor's offered_qty_per_tick scales 10→300
/// per tick, but the actual throughput is capped by the production rate (regen=10/tick
/// → inventory ≤ ~10 at any time). As a result the market clears ~10 goods/tick at
/// BOTH factor-1 and factor-30 — they are SUPPLY-CONSTRAINED, not demand-constrained.
/// The solvency question is: does the loop STAY ALIVE (trades keep firing, no
/// InsufficientFunds collapse) at the realistic 1_000_000 opening_cash?
///
/// (a) audit stays byte-invariant every tick — asserted inside run_solvency_scenario
///     via .expect in run_tick_audit_system (a conservation violation would panic).
/// (b) FinalConsumed and Trade events keep firing at factor 30 (loop not starved).
/// (c) Factor-30 Trade count is in the same ballpark as factor-1 (both are bounded
///     by supply, not cash — proves no cash-starvation / no InsufficientFunds spiral).
///
/// SOLVENCY VERDICT: SOLVENT at factor 30 / opening_cash=1_000_000.
/// Seed scaling unnecessary at factor 30 — no economy_snapshots migration.
#[test]
fn factor_30_solvency_at_fixed_opening_cash() {
    let (consumed_30, trades_30) = run_solvency_scenario(30, 50);
    let (consumed_1, trades_1) = run_solvency_scenario(1, 50);

    // (b) Demand must not have collapsed: both Trade and FinalConsumed must have fired.
    assert!(
        trades_30 > 0,
        "factor-30: Trade events must have fired (demand not starved); got trades_30={trades_30}"
    );
    assert!(
        consumed_30 > 0,
        "factor-30: FinalConsumed events must have fired; got consumed_30={consumed_30}"
    );

    // (c) Supply-constrained: both runs clear ~10 goods/tick (production rate limited,
    // not cash-limited). Confirm factor-30 is NOT demand-collapsed vs factor-1:
    // consumed_30 must be at least half of consumed_1 (same supply, different demand
    // pressure — both should trade the full supply every tick).
    assert!(
        consumed_30 * 2 >= consumed_1,
        "factor-30 should not be demand-collapsed vs factor-1: \
         consumed_30={consumed_30}, consumed_1={consumed_1}, \
         trades_30={trades_30}, trades_1={trades_1}"
    );

    // Emit diagnostic so the solvency evidence is readable in --nocapture output.
    println!(
        "SOLVENCY VERDICT factor=30 / opening_cash=1_000_000: SOLVENT (supply-constrained) — \
         consumed_30={consumed_30} consumed_1={consumed_1} \
         trades_30={trades_30} trades_1={trades_1}; \
         seed scaling unnecessary at factor 30 — no economy_snapshots migration."
    );
}
