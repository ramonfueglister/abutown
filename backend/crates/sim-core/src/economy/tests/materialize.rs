use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::materialize::{
    MaterializedTraders, RenderActor, apply_mutations, plan_mutations, plan_render_mutations,
};
use crate::economy::{
    EconomicActorId, EconomyConfig, GOOD_TOOLS, MarketId, Quantity, Trader, TraderState, Traders,
};
use crate::ids::ChunkCoord;
use crate::mobility::components::{Position, StableAgentId, TraderAgent};
use crate::mobility::resources::{AgentIdIndex, DirtyAgents};

fn seed(state: TraderState) -> (World, EconomicActorId) {
    let mut world = World::new();
    world.insert_resource(AgentIdIndex::default());
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(MaterializedTraders::default());
    world.insert_resource(EconomyConfig::default());
    let actor = EconomicActorId(1);
    let mut traders = Traders::default();
    traders.0.insert(
        actor,
        Trader {
            actor,
            good: GOOD_TOOLS,
            source: MarketId(1),
            dest: MarketId(2),
            distance_tiles: 4,
            batch_qty: Quantity(1),
            buy_premium_bps: 0,
            sell_discount_bps: 0,
            order_ttl_ticks: 10,
            state,
        },
    );
    world.insert_resource(traders);
    (world, actor)
}

fn run(
    world: &mut World,
    routes: &BTreeMap<EconomicActorId, Vec<(f32, f32)>>,
    observed: &BTreeSet<ChunkCoord>,
) {
    let muts = {
        let traders = world.resource::<Traders>();
        let config = world.resource::<EconomyConfig>();
        let materialized = world.resource::<MaterializedTraders>();
        plan_mutations(traders, config, materialized, routes, observed)
    };
    apply_mutations(world, 0, muts);
}

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
fn materialize_spawns_trader_agent_at_route_start_and_feeds_delta() {
    let (mut world, actor) = seed(TraderState::Buying { order: None });
    let routes: BTreeMap<EconomicActorId, Vec<(f32, f32)>> =
        [(actor, vec![(1.0, 1.0), (5.0, 1.0)])]
            .into_iter()
            .collect();
    run(&mut world, &routes, &observed_origin());

    let mut q = world.query_filtered::<(Entity, &Position, &StableAgentId), With<TraderAgent>>();
    let hits: Vec<(Entity, (f32, f32), String)> = q
        .iter(&world)
        .map(|(e, p, s)| (e, (p.x, p.y), s.0.0.clone()))
        .collect();
    assert_eq!(hits.len(), 1, "exactly one trader-agent");
    assert_eq!(hits[0].1, (1.0, 1.0), "Buying => progress 0 => route start");
    assert!(hits[0].2.starts_with("trader:"), "namespaced stable id");
    assert!(
        world.resource::<DirtyAgents>().0.contains(&hits[0].0),
        "trader-agent fed into DirtyAgents (the per-tick delta path)"
    );
    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&actor)
    );
}

#[test]
fn materialize_despawns_when_trader_removed_from_economy() {
    let (mut world, actor) = seed(TraderState::Buying { order: None });
    let routes: BTreeMap<EconomicActorId, Vec<(f32, f32)>> =
        [(actor, vec![(1.0, 1.0)])].into_iter().collect();
    run(&mut world, &routes, &observed_origin());
    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&actor)
    );

    world.resource_mut::<Traders>().0.remove(&actor);
    run(&mut world, &BTreeMap::new(), &observed_origin());

    let mut q = world.query_filtered::<Entity, With<TraderAgent>>();
    assert_eq!(q.iter(&world).count(), 0, "trader-agent despawned");
    assert!(
        !world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&actor)
    );
    assert!(
        !world
            .resource::<AgentIdIndex>()
            .0
            .keys()
            .any(|k| k.0.starts_with("trader:")),
        "dropped from AgentIdIndex"
    );
}

#[test]
fn materialize_despawns_when_trader_leaves_observed_chunks() {
    let (mut world, actor) = seed(TraderState::Buying { order: None });
    let routes: BTreeMap<EconomicActorId, Vec<(f32, f32)>> =
        [(actor, vec![(1.0, 1.0)])].into_iter().collect();
    let none: BTreeSet<ChunkCoord> = BTreeSet::new();

    // Observed -> spawned.
    run(&mut world, &routes, &observed_origin());
    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&actor)
    );

    // First unobserved tick: kept ALIVE + marked dirty at its position so the
    // delta emits left_agents for the chunk it left (ghost-free client removal).
    run(&mut world, &routes, &none);
    let entity = world
        .resource::<MaterializedTraders>()
        .0
        .get(&actor)
        .map(|m| m.entity)
        .expect("kept alive one tick to emit the leave");
    assert!(
        world.resource::<DirtyAgents>().0.contains(&entity),
        "marked dirty on the leaving tick"
    );

    // Second unobserved tick: despawned (LOD).
    run(&mut world, &routes, &none);
    let count = world
        .query_filtered::<Entity, With<TraderAgent>>()
        .iter(&world)
        .count();
    assert_eq!(
        count, 0,
        "despawned once unobserved (after the leave emitted)"
    );
    assert!(
        !world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&actor)
    );
}

#[test]
fn materialize_does_not_touch_money_or_goods() {
    use crate::economy::{AccountBook, InventoryBook, Money};
    let (mut world, actor) = seed(TraderState::ToDest { remaining: 2 });
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(10_000)).unwrap();
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_TOOLS, Quantity(5)).unwrap();
    world.insert_resource(accounts);
    world.insert_resource(inv);

    let routes: BTreeMap<EconomicActorId, Vec<(f32, f32)>> =
        [(actor, vec![(1.0, 1.0), (9.0, 1.0)])]
            .into_iter()
            .collect();
    for _ in 0..5 {
        run(&mut world, &routes, &observed_origin());
    }

    assert_eq!(
        world.resource::<AccountBook>().account(actor).available,
        Money(10_000),
        "money untouched (render-only)"
    );
    assert_eq!(
        world
            .resource::<InventoryBook>()
            .balance(actor, GOOD_TOOLS)
            .available,
        Quantity(5),
        "goods untouched (render-only)"
    );
}

use crate::economy::flow_shipments::SHIPMENT_ACTOR_OFFSET;
use crate::economy::materialize::materialize_traders_system;
use crate::economy::{FlowShipment, FlowShipments, GoodId, MarketSite, Markets};
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

    world.insert_resource(Traders::default());
    world.insert_resource(EconomyConfig::default());
    world.insert_resource(MaterializedTraders::default());
    world.insert_resource(FlowShipments::default());
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
    // tick (spec lines 25/63/94, mirroring
    // materialize_despawns_when_trader_leaves_observed_chunks).
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
