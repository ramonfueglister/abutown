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
