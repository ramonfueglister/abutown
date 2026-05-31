use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::materialize::{MaterializedTraders, apply_mutations, plan_mutations};
use crate::economy::{
    EconomicActorId, EconomyConfig, GOOD_TOOLS, MarketId, Quantity, Trader, TraderState, Traders,
};
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

fn run(world: &mut World, routes: &BTreeMap<EconomicActorId, Vec<(f32, f32)>>) {
    let muts = {
        let traders = world.resource::<Traders>();
        let config = world.resource::<EconomyConfig>();
        let materialized = world.resource::<MaterializedTraders>();
        plan_mutations(traders, config, materialized, routes)
    };
    apply_mutations(world, 0, muts);
}

#[test]
fn materialize_spawns_trader_agent_at_route_start_and_feeds_delta() {
    let (mut world, actor) = seed(TraderState::Buying { order: None });
    let routes: BTreeMap<EconomicActorId, Vec<(f32, f32)>> =
        [(actor, vec![(1.0, 1.0), (5.0, 1.0)])]
            .into_iter()
            .collect();
    run(&mut world, &routes);

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
    run(&mut world, &routes);
    assert!(
        world
            .resource::<MaterializedTraders>()
            .0
            .contains_key(&actor)
    );

    // Trader leaves the economy -> its render agent is despawned and dropped.
    world.resource_mut::<Traders>().0.remove(&actor);
    run(&mut world, &BTreeMap::new());

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
        run(&mut world, &routes);
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
