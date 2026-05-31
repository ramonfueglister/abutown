use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{
    AccountBook, DemandPool, DemandPools, DormantMarkets, EconomicActorId, EconomyConfig,
    GOOD_FOOD, InventoryBook, MarketChunks, MarketGoodKey, MarketGoodState, MarketGoods, MarketId,
    Money, Quantity, SupplyPool, SupplyPools, TradeLedger, WarmMarkets,
    refresh_dormant_markets_system, run_warm_market_flow_at_tick,
};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, AsleepChunk, ChunkCoordComp, WarmChunk};

fn dp(actor: u64, market: MarketId, qty: i64) -> DemandPool {
    DemandPool {
        actor: EconomicActorId(actor), market, good: GOOD_FOOD,
        desired_qty_per_tick: Quantity(qty), max_price: Money(10_000),
        urgency_bps: 0, elasticity_bps: 0, interval_ticks: 1, last_generated_tick: None,
    }
}
fn sp(actor: u64, market: MarketId, qty: i64) -> SupplyPool {
    SupplyPool {
        actor: EconomicActorId(actor), market, good: GOOD_FOOD,
        offered_qty_per_tick: Quantity(qty), min_price: Money(1),
        interval_ticks: 1, last_generated_tick: None,
    }
}
fn with_ref_price(market: MarketId, price: Money) -> MarketGoods {
    let key = MarketGoodKey { market, good: GOOD_FOOD };
    let mut mg = MarketGoods::default();
    let mut st = MarketGoodState::new(key);
    st.last_settlement_price = price;
    mg.0.insert(key, st);
    mg
}

#[test]
fn bridge_classifies_warm_dormant_and_active() {
    let mut world = World::new();
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), AsleepChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 1, y: 0 }), WarmChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 2, y: 0 }), ActiveChunk));

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(10), ChunkCoord { x: 0, y: 0 }); // asleep
    anchors.0.insert(MarketId(11), ChunkCoord { x: 1, y: 0 }); // warm
    anchors.0.insert(MarketId(12), ChunkCoord { x: 2, y: 0 }); // active
    world.insert_resource(anchors);
    world.insert_resource(DormantMarkets::default());
    world.insert_resource(WarmMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    let dormant = &world.resource::<DormantMarkets>().0;
    let warm = &world.resource::<WarmMarkets>().0;
    assert_eq!(*dormant, [MarketId(10), MarketId(11)].into_iter().collect::<BTreeSet<_>>());
    assert_eq!(*warm, [MarketId(11)].into_iter().collect::<BTreeSet<_>>());
}

#[test]
fn warm_flow_trades_min_at_reference_price() {
    let market = MarketId(1);
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(buyer, Money(1_000_000)).unwrap();
    inventory.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(buyer, dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(seller, sp(2, market, 60));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default();

    let money_before = accounts.total_money().unwrap();
    let goods_before = inventory.total_good(GOOD_FOOD).unwrap();

    // tick 0 is a multiple of interval (10).
    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger,
        &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();

    assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(60));
    assert_eq!(inventory.balance(seller, GOOD_FOOD).available, Quantity(940));
    // ref price 1000, scale 1000 -> 1 money per unit -> 60 moved.
    assert_eq!(accounts.account(seller).available, Money(60));
    assert_eq!(accounts.total_money().unwrap(), money_before);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), goods_before);
}

#[test]
fn warm_flow_conserves_with_two_sided_pro_rata() {
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
    accounts.deposit(EconomicActorId(2), Money(1_000_000)).unwrap();
    inventory.deposit(EconomicActorId(3), GOOD_FOOD, Quantity(1_000)).unwrap();
    inventory.deposit(EconomicActorId(4), GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(EconomicActorId(1), dp(1, market, 100));
    demand.0.insert(EconomicActorId(2), dp(2, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(EconomicActorId(3), sp(3, market, 50));
    supply.0.insert(EconomicActorId(4), sp(4, market, 50));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default();
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();

    // demand 200, supply 100 -> Q=100; buyers split 50/50, sellers 50/50.
    assert_eq!(inventory.balance(EconomicActorId(1), GOOD_FOOD).available, Quantity(50));
    assert_eq!(inventory.balance(EconomicActorId(2), GOOD_FOOD).available, Quantity(50));
    assert_eq!(accounts.total_money().unwrap(), m0);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
}

#[test]
fn warm_flow_only_fires_on_interval() {
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
    inventory.deposit(EconomicActorId(2), GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(EconomicActorId(1), dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(EconomicActorId(2), sp(2, market, 60));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default(); // interval 10

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 3,
    ).unwrap();
    assert_eq!(inventory.balance(EconomicActorId(1), GOOD_FOOD).available, Quantity(0));
}

#[test]
fn non_warm_market_does_not_flow() {
    let market = MarketId(1);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
    inventory.deposit(EconomicActorId(2), GOOD_FOOD, Quantity(1_000)).unwrap();
    let mut demand = DemandPools::default();
    demand.0.insert(EconomicActorId(1), dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(EconomicActorId(2), sp(2, market, 60));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = BTreeSet::new(); // market NOT warm (e.g. asleep)
    let cfg = EconomyConfig::default();

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();
    assert_eq!(inventory.balance(EconomicActorId(1), GOOD_FOOD).available, Quantity(0));
}

#[test]
fn warm_flow_caps_by_affordability_and_availability() {
    let market = MarketId(1);
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let mut accounts = AccountBook::default();
    let mut inventory = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    accounts.deposit(buyer, Money(30)).unwrap(); // affords 30 units at price 1000 (1/unit)
    inventory.deposit(seller, GOOD_FOOD, Quantity(20)).unwrap(); // only 20 in stock
    let mut demand = DemandPools::default();
    demand.0.insert(buyer, dp(1, market, 100));
    let mut supply = SupplyPools::default();
    supply.0.insert(seller, sp(2, market, 100));
    let mg = with_ref_price(market, Money(1_000));
    let warm: BTreeSet<MarketId> = [market].into_iter().collect();
    let cfg = EconomyConfig::default();
    let m0 = accounts.total_money().unwrap();
    let g0 = inventory.total_good(GOOD_FOOD).unwrap();

    run_warm_market_flow_at_tick(
        &mut accounts, &mut inventory, &mut ledger, &demand, &supply, &mg, &warm, &cfg, 0,
    ).unwrap();

    // min(affordable 30, stock 20) = 20 traded; conserved; no overdraw.
    assert_eq!(inventory.balance(buyer, GOOD_FOOD).available, Quantity(20));
    assert_eq!(accounts.total_money().unwrap(), m0);
    assert_eq!(inventory.total_good(GOOD_FOOD).unwrap(), g0);
    assert!(accounts.account(buyer).available.0 >= 0);
}

#[test]
fn warm_flow_is_deterministic() {
    let build = || {
        let market = MarketId(1);
        let mut accounts = AccountBook::default();
        let mut inventory = InventoryBook::default();
        let mut ledger = TradeLedger::default();
        accounts.deposit(EconomicActorId(1), Money(1_000_000)).unwrap();
        accounts.deposit(EconomicActorId(2), Money(1_000_000)).unwrap();
        inventory.deposit(EconomicActorId(3), GOOD_FOOD, Quantity(1_000)).unwrap();
        let mut demand = DemandPools::default();
        demand.0.insert(EconomicActorId(1), dp(1, market, 70));
        demand.0.insert(EconomicActorId(2), dp(2, market, 30));
        let mut supply = SupplyPools::default();
        supply.0.insert(EconomicActorId(3), sp(3, market, 90));
        let mg = with_ref_price(market, Money(1_000));
        let warm: BTreeSet<MarketId> = [market].into_iter().collect();
        run_warm_market_flow_at_tick(
            &mut accounts, &mut inventory, &mut ledger,
            &demand, &supply, &mg, &warm, &EconomyConfig::default(), 0,
        ).unwrap();
        ledger.0
    };
    assert_eq!(build(), build());
}

use crate::economy::EconomyPlugin;
use crate::mobility::resources::Tick;
use crate::world::schedule::SimPlugin;

#[test]
fn warm_market_flows_through_the_schedule_and_conserves() {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);

    let market = MarketId(1);
    let buyer = EconomicActorId(1);
    let seller = EconomicActorId(2);
    let coord = ChunkCoord { x: 9, y: 9 };
    world.spawn((ChunkCoordComp(coord), WarmChunk));
    {
        let mut acc = world.resource_mut::<AccountBook>();
        acc.deposit(buyer, Money(1_000_000)).unwrap();
    }
    {
        let mut inv = world.resource_mut::<InventoryBook>();
        inv.deposit(seller, GOOD_FOOD, Quantity(1_000)).unwrap();
    }
    {
        let mut mg = world.resource_mut::<MarketGoods>();
        let key = MarketGoodKey { market, good: GOOD_FOOD };
        let mut st = MarketGoodState::new(key);
        st.last_settlement_price = Money(1_000);
        mg.0.insert(key, st);
    }
    {
        let mut d = world.resource_mut::<DemandPools>();
        d.0.insert(buyer, dp(1, market, 50));
    }
    {
        let mut s = world.resource_mut::<SupplyPools>();
        s.0.insert(seller, sp(2, market, 50));
    }
    world.resource_mut::<MarketChunks>().0.insert(market, coord);
    world.insert_resource(Tick(0));

    let m0 = world.resource::<AccountBook>().total_money().unwrap();
    let g0 = world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap();

    // tick 0 fires the warm flow (multiple of 10).
    schedule.run(&mut world);

    assert_eq!(world.resource::<InventoryBook>().balance(buyer, GOOD_FOOD).available, Quantity(50));
    assert_eq!(world.resource::<AccountBook>().total_money().unwrap(), m0);
    assert_eq!(world.resource::<InventoryBook>().total_good(GOOD_FOOD).unwrap(), g0);
    assert!(world.contains_resource::<crate::economy::WarmMarkets>());
}
