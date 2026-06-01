use crate::economy::{GOOD_FOOD, MarketId, MarketSite, Money};

#[test]
fn value_types_are_serde_serializable() {
    let m = Money(1_234);
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), m);

    let site = MarketSite {
        id: MarketId(1),
        node_id: crate::routing::NodeId(7),
        name: "M1".to_string(),
    };
    let j = serde_json::to_string(&site).unwrap();
    let back: MarketSite = serde_json::from_str(&j).unwrap();
    assert_eq!(back, site);
    let _ = GOOD_FOOD;
}

use bevy_ecs::prelude::*;

use crate::economy::InventoryBalance;
use crate::economy::{
    AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, EconomyPersistSnapshot,
    EconomyPlugin, GOOD_TOOLS, InventoryBook, MarketChunks, MarketDistances, MarketGoodKey,
    MarketGoodState, MarketGoods, Markets, MoneyAccount, NextOrderId, OrderBook, OrderId,
    ProductionPool, ProductionPools, Quantity, Recipe, SupplyPool, SupplyPools, Trader,
    TraderState, Traders, apply_into_world, extract_from_world,
};
use crate::ids::ChunkCoord;
use crate::world::schedule::SimPlugin;

fn install_economy() -> World {
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    EconomyPlugin.install(&mut world, &mut schedule);
    world
}

fn seed(world: &mut World) {
    let a = EconomicActorId(1);
    let b = EconomicActorId(2);
    let m = crate::economy::MarketId(1);

    world.resource_mut::<AccountBook>().accounts.insert(
        a,
        MoneyAccount {
            available: crate::economy::Money(5_000),
            locked: crate::economy::Money(250),
        },
    );
    world.resource_mut::<InventoryBook>().balances.insert(
        (b, GOOD_FOOD),
        InventoryBalance {
            available: Quantity(40),
            locked: Quantity(5),
        },
    );

    world.resource_mut::<OrderBook>().bids.insert(
        OrderId(1),
        Bid {
            id: OrderId(1),
            owner: a,
            market: m,
            good: GOOD_FOOD,
            qty_remaining: Quantity(10),
            max_price: crate::economy::Money(1_200),
            cash_locked_remaining: crate::economy::Money(12),
            created_tick: 1,
            expires_tick: 100,
        },
    );
    world.resource_mut::<OrderBook>().asks.insert(
        OrderId(2),
        Ask {
            id: OrderId(2),
            owner: b,
            market: m,
            good: GOOD_FOOD,
            qty_remaining: Quantity(10),
            min_price: crate::economy::Money(1_000),
            goods_locked_remaining: Quantity(10),
            created_tick: 1,
            expires_tick: 100,
        },
    );
    world.resource_mut::<NextOrderId>().0 = 3;

    world.resource_mut::<Markets>().0.insert(
        m,
        crate::economy::MarketSite {
            id: m,
            node_id: crate::routing::NodeId(9),
            name: "M1".to_string(),
        },
    );
    let key = MarketGoodKey {
        market: m,
        good: GOOD_FOOD,
    };
    let mut gs = MarketGoodState::new(key);
    gs.last_settlement_price = crate::economy::Money(1_100);
    gs.last_cleared_tick = 7;
    world.resource_mut::<MarketGoods>().0.insert(key, gs);

    world.resource_mut::<DemandPools>().0.insert(
        a,
        DemandPool {
            actor: a,
            market: m,
            good: GOOD_FOOD,
            desired_qty_per_tick: Quantity(5),
            max_price: crate::economy::Money(1_300),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: Some(3),
        },
    );
    world.resource_mut::<SupplyPools>().0.insert(
        b,
        SupplyPool {
            actor: b,
            market: m,
            good: GOOD_FOOD,
            offered_qty_per_tick: Quantity(5),
            min_price: crate::economy::Money(900),
            interval_ticks: 1,
            last_generated_tick: Some(3),
        },
    );
    world.resource_mut::<ProductionPools>().0.insert(
        b,
        ProductionPool {
            actor: b,
            recipe: Recipe {
                inputs: vec![(GOOD_FOOD, Quantity(2))],
                outputs: vec![(GOOD_TOOLS, Quantity(1))],
            },
            interval_ticks: 4,
            last_generated_tick: None,
        },
    );
    world.resource_mut::<Traders>().0.insert(
        a,
        Trader {
            actor: a,
            good: GOOD_TOOLS,
            source: m,
            dest: crate::economy::MarketId(2),
            distance_tiles: 4,
            batch_qty: Quantity(100),
            buy_premium_bps: 500,
            sell_discount_bps: 500,
            order_ttl_ticks: 10,
            state: TraderState::Buying { order: None },
        },
    );
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m, ChunkCoord { x: 2, y: 3 });
    {
        let mut d = world.resource_mut::<crate::economy::MarketDistances>();
        d.0.insert((m, crate::economy::MarketId(2)), 4);
        d.0.insert((crate::economy::MarketId(2), m), 4);
    }
}

#[test]
fn economy_snapshot_round_trips() {
    let mut world = install_economy();
    seed(&mut world);

    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();

    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    let snap2 = extract_from_world(&fresh);

    assert_eq!(
        snap, snap2,
        "extract->serialize->deserialize->apply->extract is identity"
    );
    assert_eq!(
        snap.market_distances,
        vec![
            (
                (crate::economy::MarketId(1), crate::economy::MarketId(2)),
                4
            ),
            (
                (crate::economy::MarketId(2), crate::economy::MarketId(1)),
                4
            ),
        ],
        "directed distances persist in sorted BTreeMap order"
    );
}

#[test]
fn economy_snapshot_is_byte_stable() {
    let mut world = install_economy();
    seed(&mut world);
    let a = serde_json::to_vec(&extract_from_world(&world)).unwrap();
    let b = serde_json::to_vec(&extract_from_world(&world)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn market_distances_round_trips() {
    let mut world = install_economy();
    seed(&mut world);
    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert_eq!(
        world.resource::<MarketDistances>().0,
        fresh.resource::<MarketDistances>().0,
        "market_distances survive extract->serialize->apply"
    );
    // And the whole snapshot remains an identity round-trip (covers the new field).
    assert_eq!(snap, extract_from_world(&fresh));
}

#[test]
fn empty_economy_round_trips() {
    let world = install_economy();
    let snap = extract_from_world(&world);
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &snap);
    assert_eq!(snap, extract_from_world(&fresh));
}

use crate::economy::EconomySnapshotProvider;
use crate::world::persistence::SnapshotProvider;

#[test]
fn provider_collects_single_economy_item() {
    let mut world = install_economy();
    seed(&mut world);

    let provider = EconomySnapshotProvider {
        world_id: "w1".to_string(),
    };
    assert_eq!(provider.name(), "economy");
    assert_eq!(provider.schema_version(), 1);

    let items = provider.collect(&world);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key.kind, "economy");
    assert_eq!(items[0].key.identifier, "full");
    assert_eq!(items[0].key.world_id, "w1");
    assert_eq!(items[0].schema_version, 1);
    assert!(!items[0].payload.is_empty());

    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&items[0].payload).unwrap();
    assert_eq!(decoded, extract_from_world(&world));
}

#[test]
fn ledger_tail_is_capped_and_round_trips() {
    use crate::economy::{EconomyEvent, GoodId, PERSISTED_LEDGER_TAIL, Quantity, TradeLedger};

    let mut world = install_economy();
    let overflow = PERSISTED_LEDGER_TAIL as u64 + 50;
    {
        let mut ledger = world.resource_mut::<TradeLedger>();
        for i in 0..overflow {
            ledger.0.push(EconomyEvent::Produced {
                actor: EconomicActorId(1),
                good: GoodId(1),
                qty: Quantity(i as i64),
            });
        }
    }

    let snap = extract_from_world(&world);
    assert_eq!(
        snap.ledger_tail.len(),
        PERSISTED_LEDGER_TAIL,
        "tail is capped"
    );
    // The newest event is preserved; the oldest 50 are dropped.
    assert_eq!(
        snap.ledger_tail.last(),
        world.resource::<TradeLedger>().0.last()
    );
    assert_eq!(
        snap.ledger_tail.first(),
        Some(&EconomyEvent::Produced {
            actor: EconomicActorId(1),
            good: GoodId(1),
            qty: Quantity(50),
        })
    );

    // Serialize round-trip + apply restores the tail verbatim.
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert_eq!(fresh.resource::<TradeLedger>().0, snap.ledger_tail);
}
