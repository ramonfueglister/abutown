use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::materialize::{MaterializedTraders, RenderActor, plan_render_mutations};
use crate::economy::{EconomicActorId, MarketId};
use crate::ids::ChunkCoord;
use crate::mobility::components::{Position, StableAgentId, TraderAgent};
use crate::mobility::resources::{AgentIdIndex, DirtyAgents};

fn observed_origin() -> BTreeSet<ChunkCoord> {
    [ChunkCoord { x: 0, y: 0 }].into_iter().collect()
}

#[test]
fn plan_render_mutations_drives_lifecycle_generically() {
    use crate::economy::materialize::TraderMutation;
    // A render-actor with no backing Trader (progress supplied directly).
    let actor = EconomicActorId(7);
    let polyline = vec![(1.0, 1.0), (5.0, 1.0)];
    let actors = [RenderActor {
        actor,
        polyline: &polyline,
        progress: 0.0,
        arrived: false,
    }];
    let materialized = MaterializedTraders::default();
    let observed = observed_origin();

    // First tick: observed + not materialized => Spawn at route start.
    let muts = plan_render_mutations(&actors, &materialized, &observed);
    assert_eq!(muts.len(), 1, "exactly one mutation");
    match &muts[0] {
        TraderMutation::Spawn { actor: a, x, y, .. } => {
            assert_eq!(*a, actor);
            assert_eq!((*x, *y), (1.0, 1.0), "progress 0 => route start");
        }
        _ => panic!("expected Spawn"),
    }

    // A materialized actor that is no longer in the actor list must be despawned
    // by the generic sweep (drives the shipment-expiry / trader-removal path).
    let mut materialized = MaterializedTraders::default();
    materialized.0.insert(
        EconomicActorId(99),
        crate::economy::materialize::MaterializedTrader {
            entity: Entity::PLACEHOLDER,
            observed: true,
        },
    );
    let muts = plan_render_mutations(&[], &materialized, &observed);
    assert_eq!(muts.len(), 1, "stale materialized actor swept");
    match &muts[0] {
        TraderMutation::Despawn { actor: a } => assert_eq!(a.0, 99),
        _ => panic!("expected Despawn of stale actor"),
    }
}

#[test]
fn materialize_does_not_touch_money_or_goods_with_active_shipment() {
    use crate::economy::shoppers::{SHOPPER_ACTOR_OFFSET, ShopperVisit, ShopperVisits};
    use crate::economy::{AccountBook, InventoryBook, Money};

    // Build a fully-routed world so materialize_traders_system can run end-to-end.
    let a = MarketId(10);
    let b = MarketId(11);
    let mut world = routed_shipment_world(a, b);

    // Give the shipment actor some economic state so we can verify it is never mutated.
    let shipment_actor_id = EconomicActorId(SHIPMENT_ACTOR_OFFSET);
    let shopper_actor_id = EconomicActorId(SHOPPER_ACTOR_OFFSET);
    let mut accounts = AccountBook::default();
    accounts.deposit(shipment_actor_id, Money(9_999)).unwrap();
    accounts.deposit(shopper_actor_id, Money(4_321)).unwrap();
    let mut inv = InventoryBook::default();
    inv.deposit(shipment_actor_id, GoodId(0), Quantity(7))
        .unwrap();
    inv.deposit(shopper_actor_id, GoodId(0), Quantity(11))
        .unwrap();
    world.insert_resource(accounts);
    world.insert_resource(inv);

    // Insert an active in-transit shipment.
    world.resource_mut::<FlowShipments>().0.insert(
        0,
        FlowShipment {
            id: 0,
            from_market: a,
            to_market: b,
            good: GoodId(0),
            qty: Quantity(50),
            start_tick: 0,
            travel_ticks: 10,
        },
    );
    // Insert an active shopper visit (walks from market_a's node to market_b's node).
    world.resource_mut::<ShopperVisits>().0.insert(
        0,
        ShopperVisit {
            id: 0,
            market: b,
            good: GoodId(0),
            origin_node: crate::routing::NodeId(0),
            start_tick: 0,
            travel_ticks: 10,
        },
    );

    // Run the materialize system several ticks; both the shipment and shopper render and expire.
    for t in 0u64..12 {
        world.insert_resource(Tick(t));
        materialize_traders_system(&mut world);
    }

    // Economic books must be byte-identical to what we inserted — pure render projection.
    assert_eq!(
        world
            .resource::<AccountBook>()
            .account(shipment_actor_id)
            .available,
        Money(9_999),
        "money untouched by shipment-materialize path"
    );
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(shipment_actor_id, GoodId(0))
            .available,
        Quantity(7),
        "goods untouched by shipment-materialize path"
    );
    // Shopper actor's economic books must also remain untouched — shopper path is read-only.
    assert_eq!(
        world
            .resource::<AccountBook>()
            .account(shopper_actor_id)
            .available,
        Money(4_321),
        "money untouched by shopper-materialize path"
    );
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(shopper_actor_id, GoodId(0))
            .available,
        Quantity(11),
        "goods untouched by shopper-materialize path"
    );
}

use crate::economy::flow_shipments::SHIPMENT_ACTOR_OFFSET;
use crate::economy::materialize::materialize_traders_system;
use crate::economy::{FlowShipment, FlowShipments, GoodId, MarketSite, Markets, Quantity};
use crate::mobility::components::SpriteKey;
use crate::mobility::resources::Tick;
use crate::routing::{
    Edge, EdgeId, EdgeKind, FlowFieldCache, Graph, HpaConfig, HpaIndex, Node, NodeId, NodeKind,
    NodeSpatialIndex,
};
use crate::world::components::{ActiveChunk, ChunkCoordComp};

/// Build a fully routed world (Graph + HpaIndex + FlowFieldCache + the render
/// resources the materialize system reads) with two markets anchored to two
/// footway nodes that share chunk (0,0). One straight Footway joins them so a
/// flow shipment's whole route — and every mid-flight position — lands in the
/// observed chunk. No demo trader is seeded.
fn routed_shipment_world(market_a: MarketId, market_b: MarketId) -> World {
    let mut world = World::new();
    let graph = Graph::new(
        vec![
            Node {
                id: NodeId(0),
                position: (1.0, 1.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (20.0, 1.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ],
        vec![Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(1.0, 1.0), (20.0, 1.0)],
            length: 19.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("walk:ab".into()),
        }],
    );
    let hpa = HpaIndex::build(&graph, HpaConfig::default()).expect("HPA should build");
    let spatial = NodeSpatialIndex::from_nodes(graph.nodes());
    world.insert_resource(graph);
    world.insert_resource(hpa);
    world.insert_resource(spatial);
    world.insert_resource(FlowFieldCache::default());

    let mut markets = Markets::default();
    markets.0.insert(
        market_a,
        MarketSite {
            id: market_a,
            node_id: NodeId(0),
            name: "A".to_string(),
        },
    );
    markets.0.insert(
        market_b,
        MarketSite {
            id: market_b,
            node_id: NodeId(1),
            name: "B".to_string(),
        },
    );
    world.insert_resource(markets);

    world.insert_resource(MaterializedTraders::default());
    world.insert_resource(FlowShipments::default());
    world.insert_resource(crate::economy::shoppers::ShopperVisits::default());
    world.insert_resource(crate::economy::commuters::CommuterTrips::default());
    world.insert_resource(AgentIdIndex::default());
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(Tick(0));

    // Chunk (0,0) is observed (Active). The whole route sits inside it.
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), ActiveChunk));
    world
}

#[test]
fn materialize_renders_flow_shipment_then_despawns_on_arrival() {
    let a = MarketId(1);
    let b = MarketId(2);
    let mut world = routed_shipment_world(a, b);
    let shipment_actor = EconomicActorId(SHIPMENT_ACTOR_OFFSET);

    // One in-transit shipment A->B, 10 ticks, starting at tick 0.
    world.resource_mut::<FlowShipments>().0.insert(
        0,
        FlowShipment {
            id: 0,
            from_market: a,
            to_market: b,
            good: GoodId(0),
            qty: Quantity(10),
            start_tick: 0,
            travel_ticks: 10,
        },
    );

    // Mid-flight (tick 5 => progress 0.5): a trader-agent must materialize at the
    // shipment's progressed position inside the observed chunk.
    world.insert_resource(Tick(5));
    materialize_traders_system(&mut world);

    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&shipment_actor),
        "shipment-trader materialized while in an observed chunk"
    );
    let mut q =
        world.query_filtered::<(&Position, &StableAgentId, &SpriteKey), With<TraderAgent>>();
    let hits: Vec<((f32, f32), String, String)> = q
        .iter(&world)
        .map(|(p, s, sk)| ((p.x, p.y), s.0.0.clone(), sk.0.clone()))
        .collect();
    assert_eq!(hits.len(), 1, "exactly one shipment-trader agent");
    assert!(hits[0].1.starts_with("trader:"), "namespaced stable id");
    assert!(hits[0].2.starts_with("trader:"), "trader sprite variant");
    // Progress 0.5 along [(1,1)->(20,1)] => ~(10.5, 1.0), well inside chunk (0,0).
    assert!(
        (hits[0].0.0 - 10.5).abs() < 0.5 && (hits[0].0.1 - 1.0).abs() < 0.01,
        "rendered at the progressed position, got {:?}",
        hits[0].0
    );

    // Capture the live render-entity so we can assert it is freshly dirtied on the
    // arrival (leave) tick. Drain DirtyAgents first so the assertion below proves a
    // NEW dirty was emitted on the arrival tick — not a stale one from the spawn.
    let entity = world
        .resource::<MaterializedTraders>()
        .0
        .get(&shipment_actor)
        .map(|m| m.entity)
        .expect("materialized mid-flight");
    world.resource_mut::<DirtyAgents>().0.clear();

    // Arrival tick (tick 10 => progress 1.0, arrived): the destination lies inside
    // the observed chunk (the player is watching goods arrive), so the generic
    // sweep would despawn abruptly without a leave -> client ghost. Instead the
    // arrived shipment is routed through the SAME ghost-free leaving->despawn path
    // as an LOD demotion: on this tick the agent is marked dirty (the leave) so
    // tick_mobility can broadcast its removal, and the shipment is kept one extra
    // tick (spec lines 25/63/94, mirroring the ghost-free leave->despawn design).
    world.insert_resource(Tick(10));
    materialize_traders_system(&mut world);
    assert!(
        world.resource::<DirtyAgents>().0.contains(&entity),
        "arrived shipment-trader marked dirty on the leave tick (ghost-free removal)"
    );
    assert!(
        !world.resource::<FlowShipments>().0.is_empty(),
        "arrived shipment kept one extra tick so the leave is emitted before despawn"
    );
    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&shipment_actor),
        "agent still alive on the arrival (leave) tick"
    );
    assert_eq!(
        world
            .query_filtered::<Entity, With<TraderAgent>>()
            .iter(&world)
            .count(),
        1,
        "one trader-agent still alive on the leave tick"
    );

    // Next tick: the leave was already emitted, so the agent is despawned (LOD)
    // and only now is the shipment dropped from FlowShipments.
    world.insert_resource(Tick(11));
    materialize_traders_system(&mut world);
    assert!(
        !world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&shipment_actor),
        "shipment-trader despawned the tick after the leave"
    );
    assert!(
        world.resource::<FlowShipments>().0.is_empty(),
        "arrived shipment dropped once its trader finished the leave->despawn path"
    );
    assert_eq!(
        world
            .query_filtered::<Entity, With<TraderAgent>>()
            .iter(&world)
            .count(),
        0,
        "no trader-agent remains after arrival"
    );
}

#[test]
fn materialize_renders_shopper_then_despawns_on_arrival() {
    use crate::economy::shoppers::{SHOPPER_ACTOR_OFFSET, ShopperVisit, ShopperVisits};

    // Reuse the #70 routed-world fixture: two markets on a single footway, both in
    // chunk (0,0). The shopper walks from the origin footway node (node 0) TO the
    // market (node 1); the whole route — and every mid-flight position — sits in the
    // observed chunk.
    let a = MarketId(1);
    let b = MarketId(2);
    let mut world = routed_shipment_world(a, b);
    let shopper_actor = EconomicActorId(SHOPPER_ACTOR_OFFSET);

    // One active visit (id 0) walking origin=node0 -> market_b=node1, 10 ticks.
    world.resource_mut::<ShopperVisits>().0.insert(
        0,
        ShopperVisit {
            id: 0,
            market: b,
            good: GoodId(0),
            origin_node: NodeId(0),
            start_tick: 0,
            travel_ticks: 10,
        },
    );

    // Mid-flight (tick 5 => progress 0.5): a shopper-agent materializes at the
    // visit's progressed position inside the observed chunk.
    world.insert_resource(Tick(5));
    materialize_traders_system(&mut world);

    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&shopper_actor),
        "shopper materialized while in an observed chunk"
    );
    let mut q =
        world.query_filtered::<(&Position, &StableAgentId, &SpriteKey), With<TraderAgent>>();
    let hits: Vec<((f32, f32), String, String)> = q
        .iter(&world)
        .map(|(p, s, sk)| ((p.x, p.y), s.0.0.clone(), sk.0.clone()))
        .collect();
    assert_eq!(hits.len(), 1, "exactly one shopper agent");
    assert!(
        hits[0].1.starts_with("shopper:"),
        "shopper-namespaced stable id, got {:?}",
        hits[0].1
    );
    assert!(
        hits[0].2.starts_with("shopper:"),
        "shopper sprite variant, got {:?}",
        hits[0].2
    );
    // Progress 0.5 along [(1,1)->(20,1)] => ~(10.5, 1.0), well inside chunk (0,0).
    assert!(
        (hits[0].0.0 - 10.5).abs() < 0.5 && (hits[0].0.1 - 1.0).abs() < 0.01,
        "rendered at the progressed position, got {:?}",
        hits[0].0
    );

    // The shopper visit must remain mid-flight (not yet arrived).
    assert!(
        !world.resource::<ShopperVisits>().0.is_empty(),
        "active shopper visit retained mid-flight"
    );

    let entity = world
        .resource::<MaterializedTraders>()
        .0
        .get(&shopper_actor)
        .map(|m| m.entity)
        .expect("materialized mid-flight");
    world.resource_mut::<DirtyAgents>().0.clear();

    // Arrival tick (tick 10 => progress 1.0, arrived): the destination sits in the
    // observed chunk, so the arrived shopper is routed through the SAME ghost-free
    // leave->despawn path. On this tick the agent is dirtied (the leave) and kept;
    // the visit is retained one extra tick.
    world.insert_resource(Tick(10));
    materialize_traders_system(&mut world);
    assert!(
        world.resource::<DirtyAgents>().0.contains(&entity),
        "arrived shopper marked dirty on the leave tick (ghost-free removal)"
    );
    assert!(
        !world.resource::<ShopperVisits>().0.is_empty(),
        "arrived shopper visit kept one extra tick so the leave is emitted before despawn"
    );
    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&shopper_actor),
        "agent still alive on the arrival (leave) tick"
    );

    // Next tick: the leave was emitted, so the agent is despawned (LOD) and only now
    // is the visit dropped from ShopperVisits.
    world.insert_resource(Tick(11));
    materialize_traders_system(&mut world);
    assert!(
        !world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&shopper_actor),
        "shopper despawned the tick after the leave"
    );
    assert!(
        world.resource::<ShopperVisits>().0.is_empty(),
        "arrived shopper visit dropped once its agent finished the leave->despawn path"
    );
    assert_eq!(
        world
            .query_filtered::<Entity, With<TraderAgent>>()
            .iter(&world)
            .count(),
        0,
        "no shopper-agent remains after arrival"
    );
}
