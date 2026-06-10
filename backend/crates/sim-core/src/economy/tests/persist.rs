use crate::economy::{GOOD_FOOD, HouseholdSector, MarketId, MarketSite, Money};

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
    ProductionPool, ProductionPools, Quantity, Recipe, SupplyPool, SupplyPools, apply_into_world,
    extract_from_world,
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
            last_consumed_tick: None,
            income_last_tick: crate::economy::Money::ZERO,
            mpc_bps: 8_000,
            autonomous: crate::economy::Money(5_000),
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
    world
        .resource_mut::<MarketChunks>()
        .0
        .insert(m, ChunkCoord { x: 2, y: 3 });
    {
        let mut d = world.resource_mut::<crate::economy::MarketDistances>();
        d.0.insert((m, crate::economy::MarketId(2)), 4);
        d.0.insert((crate::economy::MarketId(2), m), 4);
    }
    world.insert_resource(HouseholdSector {
        population: 500_000,
        pool_weights: std::collections::BTreeMap::from([(a, 3), (b, 1)]),
    });
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
fn household_sector_round_trips() {
    let a = EconomicActorId(1);
    let b = EconomicActorId(2);
    let mut world = install_economy();
    seed(&mut world);
    // seed() already inserts HouseholdSector {population:500_000, pool_weights:{a:3, b:1}}.

    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();

    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);

    // The HouseholdSector resource round-trips exactly.
    let orig = world.resource::<HouseholdSector>();
    let restored = fresh.resource::<HouseholdSector>();
    assert_eq!(
        orig.population, restored.population,
        "population round-trips"
    );
    assert_eq!(
        orig.pool_weights, restored.pool_weights,
        "pool_weights round-trips"
    );

    // Snapshot identity (extract->serialize->deserialize->apply->extract is identity).
    assert_eq!(
        snap,
        extract_from_world(&fresh),
        "full snapshot identity with HouseholdSector"
    );
    // Spot-check the snapshot fields.
    assert_eq!(snap.household_sector.population, 500_000);
    assert!(
        snap.household_sector.pool_weights.contains(&(a, 3)),
        "a has weight 3"
    );
    assert!(
        snap.household_sector.pool_weights.contains(&(b, 1)),
        "b has weight 1"
    );
}

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
fn demand_pool_wage_fields_round_trip() {
    let mut world = install_economy();
    let actor = EconomicActorId(42);
    world.resource_mut::<DemandPools>().0.insert(
        actor,
        DemandPool {
            actor,
            market: crate::economy::MarketId(9_002),
            good: GOOD_TOOLS,
            desired_qty_per_tick: Quantity(7),
            max_price: crate::economy::Money(2_000),
            urgency_bps: 0,
            elasticity_bps: 0,
            interval_ticks: 1,
            last_generated_tick: Some(3),
            last_consumed_tick: Some(2),
            income_last_tick: crate::economy::Money(1_234),
            mpc_bps: 7_500,
            autonomous: crate::economy::Money(4_321),
        },
    );
    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    let restored = fresh.resource::<DemandPools>().0[&actor];
    assert_eq!(restored.income_last_tick, crate::economy::Money(1_234));
    assert_eq!(restored.mpc_bps, 7_500);
    assert_eq!(restored.autonomous, crate::economy::Money(4_321));
    assert_eq!(snap, extract_from_world(&fresh), "identity round-trip");
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

#[test]
fn raw_deposits_round_trip() {
    use crate::economy::GOOD_RAW;
    use crate::economy::production::{PRODUCER_TOOLS, RawDeposit, RawDeposits};

    let mut world = install_economy();
    world.resource_mut::<RawDeposits>().0.insert(
        PRODUCER_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(10),
            interval_ticks: 1,
            last_regen_tick: Some(42),
        },
    );

    let snap = extract_from_world(&world);
    assert_eq!(
        snap.raw_deposits,
        vec![(
            PRODUCER_TOOLS,
            RawDeposit {
                good: GOOD_RAW,
                qty_per_interval: Quantity(10),
                interval_ticks: 1,
                last_regen_tick: Some(42),
            }
        )],
        "raw_deposits extracted in sorted order"
    );

    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);

    let restored = fresh.resource::<RawDeposits>().0[&PRODUCER_TOOLS];
    assert_eq!(restored.last_regen_tick, Some(42));
    assert_eq!(restored.qty_per_interval, Quantity(10));
    assert_eq!(
        snap,
        extract_from_world(&fresh),
        "full snapshot identity with raw_deposits"
    );
}

/// Mirrors `raw_deposits_round_trip` for the new non-default `input_pools` field:
/// the `last_generated_tick` cursor and the discovered `max_price` survive the
/// extract → JSON → apply round trip byte-stably.
#[test]
fn input_pools_round_trip() {
    use crate::economy::GOOD_WOOD;
    use crate::economy::producers::{InputPool, InputPools};
    use crate::economy::production::PRODUCER_TOOLS;

    let mut world = install_economy();
    world.resource_mut::<InputPools>().0.insert(
        PRODUCER_TOOLS,
        InputPool {
            actor: PRODUCER_TOOLS,
            market: MarketId(9001),
            good: GOOD_WOOD,
            in_qty: Quantity(10),
            out_qty: Quantity(10),
            out_good: GOOD_TOOLS,
            interval_ticks: 1,
            last_generated_tick: Some(42),
            max_price: Money(400),
        },
    );

    let snap = extract_from_world(&world);
    assert_eq!(
        snap.input_pools.len(),
        1,
        "input_pools extracted (sorted order)"
    );
    assert_eq!(snap.input_pools[0].0, PRODUCER_TOOLS);

    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);

    let restored = fresh.resource::<InputPools>().0[&PRODUCER_TOOLS];
    assert_eq!(restored.last_generated_tick, Some(42), "cursor survives");
    assert_eq!(restored.max_price, Money(400), "discovered bound survives");
    assert_eq!(restored.good, GOOD_WOOD);
    assert_eq!(restored.out_good, GOOD_TOOLS);
    assert_eq!(
        snap,
        extract_from_world(&fresh),
        "full snapshot identity with input_pools"
    );
}

#[test]
fn three_extractor_raw_deposits_round_trip() {
    use crate::economy::GOOD_RAW;
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, PRODUCER_TOOLS, RawDeposit, RawDeposits,
    };

    let mut world = install_economy();
    for (actor, cursor) in [
        (PRODUCER_TOOLS, Some(7u64)),
        (EXTRACTOR_FOOD_A, Some(11)),
        (EXTRACTOR_FOOD_FA, None),
    ] {
        world.resource_mut::<RawDeposits>().0.insert(
            actor,
            RawDeposit {
                good: GOOD_RAW,
                qty_per_interval: Quantity(10),
                interval_ticks: 1,
                last_regen_tick: cursor,
            },
        );
    }

    let snap = extract_from_world(&world);
    assert_eq!(
        snap.raw_deposits.len(),
        3,
        "all three extractor deposits persist"
    );
    // Sorted by EconomicActorId (8_031, 8_032, 8_033).
    assert_eq!(snap.raw_deposits[0].0, PRODUCER_TOOLS);
    assert_eq!(snap.raw_deposits[1].0, EXTRACTOR_FOOD_A);
    assert_eq!(snap.raw_deposits[2].0, EXTRACTOR_FOOD_FA);

    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert_eq!(fresh.resource::<RawDeposits>().0.len(), 3);
    assert_eq!(
        fresh.resource::<RawDeposits>().0[&EXTRACTOR_FOOD_A].last_regen_tick,
        Some(11)
    );
    assert_eq!(
        snap,
        extract_from_world(&fresh),
        "full snapshot identity with three raw_deposits"
    );
}

#[test]
fn snapshot_without_raw_deposits_field_fails_to_deserialize() {
    // No serde-default: a JSON object missing `raw_deposits` MUST fail (forces the one-time
    // DELETE FROM economy_snapshots before deploy; no silent legacy shim).
    let json = r#"{"accounts":[],"inventory":[],"bids":[],"asks":[],"next_order_id":0,
        "markets":[],"market_goods":[],"demand_pools":[],"supply_pools":[],
        "production_pools":[],"market_chunks":[],"ledger_tail":[],"market_distances":[],
        "household_sector":{"population":0,"pool_weights":[]}}"#;
    let res: Result<EconomyPersistSnapshot, _> = serde_json::from_str(json);
    assert!(
        res.is_err(),
        "missing raw_deposits must fail (no serde-default)"
    );
}

#[test]
fn tick_audit_event_round_trips_in_ledger_tail() {
    use crate::economy::{EconomyEvent, EconomyPersistSnapshot, Money, TradeLedger};
    let mut world = install_economy();
    world
        .resource_mut::<TradeLedger>()
        .0
        .push(EconomyEvent::TickAudit {
            tick: 5,
            total_money: Money(99_999),
        });
    let snap = extract_from_world(&world);
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    let mut fresh = install_economy();
    apply_into_world(&mut fresh, &decoded);
    assert!(
        fresh.resource::<TradeLedger>().0.iter().any(|e| matches!(
            e,
            EconomyEvent::TickAudit {
                tick: 5,
                total_money: Money(99_999)
            }
        )),
        "TickAudit survives the ledger_tail round-trip"
    );
    assert_eq!(
        snap,
        extract_from_world(&fresh),
        "full snapshot identity with a TickAudit event"
    );
}
