//! Blocker-1: prove the abutopia world data makes residential-corridor pedestrians
//! bind home_market to a co-located consumption market (9002), so attribution can
//! route them. These tests load the REAL abutopia bundle.

use crate::base_world::BaseWorldBundle;
use crate::economy::{EconomyPlugin, seed_from_markets_layer};
use crate::mobility::market_binding::{assign_binding, markets_with_positions};
use crate::mobility::seed::from_base_world_bundle;

/// Build the full abutopia world (graph + NodeSpatialIndex via the mobility builder)
/// and seed the economy on top, so `markets_with_positions` returns the snapped
/// market node positions.
fn abutopia_world_with_economy() -> bevy_ecs::world::World {
    use crate::world::schedule::SimPlugin;
    let bundle = BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    let (mut world, mut schedule) =
        from_base_world_bundle(&bundle).expect("abutopia world builds from bundle");
    EconomyPlugin.install(&mut world, &mut schedule);
    seed_from_markets_layer(&mut world, &bundle.markets);
    world
}

/// Full chain on real data: spawn a citizen at the corridor (binds home=9002 against
/// the live economy), mark 9002's chunk observed, give 9002 realized consumption, run
/// attribution → the citizen is routed to 9002's node (routed > 0).
#[test]
fn corridor_citizen_is_routed_to_9002_when_observed_and_consuming() {
    use crate::economy::{
        MarketGoodKey, MarketGoodState, MarketGoods, MarketId, Markets, Quantity,
    };
    use crate::mobility::MarketBinding;
    use crate::mobility::resources::CitizenEconomicTargets;
    use crate::world::components::{ActiveChunk, ChunkCoordComp};

    let mut world = abutopia_world_with_economy();

    // 9002's snapped node + chunk (the market sits on the corridor).
    let node_9002 = world
        .resource::<Markets>()
        .0
        .get(&MarketId(9002))
        .expect("market 9002 seeded")
        .node_id;
    let pos = world
        .resource::<crate::routing::Graph>()
        .node(node_9002)
        .position;
    let chunk = crate::mobility::chunk_of(pos.0, pos.1, 32);

    // Spawn fresh citizens at the corridor using distinct ids (the bundle already
    // seeds agents named agent:walk:0..N; use a different prefix to avoid entity
    // collisions and ensure these are the only home_market=9002 candidates).
    let test_ids: Vec<crate::ids::AgentId> = (0..5)
        .map(|i| crate::ids::AgentId(format!("test:corridor:{i}")))
        .collect();
    for id in &test_ids {
        let rec = crate::mobility::AgentRecord::new_born_at(
            id.clone(),
            crate::mobility::AgentMobilityState::Walking {
                link_id: "link:walk:corridor:0".to_string(),
                progress: 0.5,
            },
            vec![],
            0.05,
            0,
        );
        crate::mobility::api::spawn_agent_from_record_at_position(&mut world, rec, (111.5, 64.51));
    }

    // Sanity-check: the freshly-spawned citizens must have bound home_market=9002.
    // Query by StableAgentId to target only our test agents.
    {
        use crate::mobility::components::{AgentMarker, StableAgentId};
        let mut q = world.query_filtered::<(&StableAgentId, &MarketBinding), bevy_ecs::prelude::With<AgentMarker>>();
        let my_bindings: Vec<u32> = q
            .iter(&world)
            .filter(|(id, _)| test_ids.contains(&id.0))
            .map(|(_, b)| b.home_market)
            .collect();
        assert_eq!(
            my_bindings.len(),
            5,
            "all 5 test corridor citizens must be present"
        );
        for home in &my_bindings {
            assert_eq!(
                *home, 9002,
                "spawned corridor citizen must bind home_market=9002; got {home}"
            );
        }
    }

    // Mark 9002's chunk observed.
    world.spawn((ChunkCoordComp(chunk), ActiveChunk));

    // Give 9002 realized consumption this tick (GOOD_FOOD and GOOD_TOOLS at 9002).
    for good in [crate::economy::GOOD_FOOD, crate::economy::GOOD_TOOLS] {
        let key = MarketGoodKey {
            market: MarketId(9002),
            good,
        };
        let mut goods = world.resource_mut::<MarketGoods>();
        let st = goods
            .0
            .entry(key)
            .or_insert_with(|| MarketGoodState::new(key));
        st.consumed_qty_last_tick = Quantity(30);
    }

    crate::economy::attribution::run_citizen_attribution_system(&mut world);

    let targets = world.resource::<CitizenEconomicTargets>().0.clone();
    assert!(
        !targets.is_empty(),
        "corridor citizens must be routed (routed>0); got 0"
    );
    for (_, node) in targets.iter() {
        assert_eq!(
            *node, node_9002,
            "routed citizens target market 9002's node"
        );
    }
    println!("routed={} all → node {:?}", targets.len(), node_9002);
}

/// Corridor:sidewalk:south spans tiles x≈106..117 at y=64.51. After re-anchoring 9002
/// onto the corridor, every pedestrian there must bind home_market = 9002 (nearest).
#[test]
fn corridor_pedestrians_bind_home_market_9002() {
    let world = abutopia_world_with_economy();
    let positions = markets_with_positions(&world);
    assert_eq!(
        positions.len(),
        4,
        "all four abutopia markets snapped to graph nodes"
    );

    for px in [106.0_f32, 111.5, 117.0] {
        let binding =
            assign_binding((px, 64.51), &positions).expect("binding exists with four live markets");
        assert_eq!(
            binding.home_market, 9002,
            "pedestrian at ({px}, 64.51) must bind home_market=9002; got {}",
            binding.home_market
        );
    }
}
