use bevy_ecs::prelude::*;

use crate::base_world::HouseholdSpec;
use crate::economy::{DemandPools, EconomyConfig, MarketChunks, MarketId, Markets, SupplyPools};
use crate::ids::ChunkCoord;
use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};

fn node(id: u32, x: f32, y: f32) -> Node {
    Node {
        id: NodeId(id),
        position: (x, y),
        kind: NodeKind::Intersection,
        legacy_id: None,
    }
}

/// Build a fully-seeded world using the authored abutopia markets layer. Installs
/// the full `EconomyPlugin` scaffold (all resources) plus the four reference footway
/// nodes, then seeds via `seed_from_markets_layer`. Every economy test that needs a
/// ready-to-go economy should call this helper — it exercises the factory path end-to-end.
fn seed_world() -> World {
    use crate::world::schedule::SimPlugin;
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    crate::economy::EconomyPlugin.install(&mut world, &mut schedule);
    let nodes = vec![
        node(0, 8.0, 8.0),
        node(1, 72.0, 8.0),
        node(2, 8.0, 40.0),
        node(3, 72.0, 40.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));
    let bundle = crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads for seed_world()");
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);
    world
}

#[test]
fn factory_seeds_four_markets_and_two_distance_pairs() {
    let world = seed_world();

    assert_eq!(world.resource::<Markets>().0.len(), 4, "four city markets");
    assert_eq!(world.resource::<MarketChunks>().0.len(), 4, "all anchored");

    // The primary production/consumption pair still exists.
    let distances = world.resource::<crate::economy::MarketDistances>();
    assert!(
        distances
            .0
            .contains_key(&(MarketId(9_001), MarketId(9_002))),
        "primary city pair baked"
    );

    // The cross-town markets have distance entries in both directions.
    assert!(
        distances
            .0
            .contains_key(&(MarketId(9_003), MarketId(9_004))),
        "cross-town A->B distance baked"
    );
    assert!(
        distances
            .0
            .contains_key(&(MarketId(9_004), MarketId(9_003))),
        "cross-town B->A distance baked"
    );
}

#[test]
fn seed_adds_second_good_without_new_markets() {
    // After seeding, the live economy still has exactly 4 markets, and a
    // GOOD_FOOD supplier@m_a + consumer@m_b exists so the macro flow
    // produces a non-vacuous cross-market FOOD flow on the live stream.
    use crate::economy::GOOD_FOOD;
    let world = seed_world();
    assert_eq!(
        world.resource::<Markets>().0.len(),
        4,
        "still exactly 4 markets"
    );
    let has_food_supply = world
        .resource::<SupplyPools>()
        .0
        .values()
        .any(|p| p.good == GOOD_FOOD);
    let has_food_demand = world
        .resource::<DemandPools>()
        .0
        .values()
        .any(|p| p.good == GOOD_FOOD);
    assert!(
        has_food_supply && has_food_demand,
        "FOOD supplier@A + consumer@B added"
    );
}

#[test]
fn seed_adds_cross_town_markets_for_dormant_cross_flow() {
    // The bottom-edge markets (F_A @ chunk (0,1), F_B @ chunk (2,1)) keep a
    // standing GOOD_FOOD imbalance for recurring MacroFlow.
    use crate::economy::GOOD_FOOD;
    let world = seed_world();

    // The two cross-town markets exist.
    let markets = world.resource::<Markets>();
    assert!(
        markets.0.contains_key(&MarketId(9_003)),
        "cross-town market F_A seeded"
    );
    assert!(
        markets.0.contains_key(&MarketId(9_004)),
        "cross-town market F_B seeded"
    );

    // Both have entries in MarketChunks.
    let chunks = world.resource::<MarketChunks>();
    let chunk_fa = *chunks.0.get(&MarketId(9_003)).expect("F_A chunk");
    let chunk_fb = *chunks.0.get(&MarketId(9_004)).expect("F_B chunk");

    assert_eq!(chunk_fa, ChunkCoord { x: 0, y: 1 }, "F_A bottom-left chunk");
    assert_eq!(
        chunk_fb,
        ChunkCoord { x: 2, y: 1 },
        "F_B bottom-right chunk"
    );
    assert_ne!(chunk_fa, chunk_fb, "F_A and F_B in different chunks");

    // Supplier pool at F_A and consumer pool at F_B for GOOD_FOOD.
    let supply = world.resource::<SupplyPools>();
    let demand = world.resource::<DemandPools>();
    assert!(
        supply
            .0
            .values()
            .any(|p| p.market == MarketId(9_003) && p.good == GOOD_FOOD),
        "FOOD supplier at F_A"
    );
    assert!(
        demand
            .0
            .values()
            .any(|p| p.market == MarketId(9_004) && p.good == GOOD_FOOD),
        "FOOD consumer at F_B"
    );

    // MarketDistances entry in both directions.
    let distances = world.resource::<crate::economy::MarketDistances>();
    assert!(
        distances
            .0
            .get(&(MarketId(9_003), MarketId(9_004)))
            .copied()
            .unwrap_or(0)
            > 0,
        "F_A->F_B distance > 0"
    );
    assert!(
        distances
            .0
            .get(&(MarketId(9_004), MarketId(9_003)))
            .copied()
            .unwrap_or(0)
            > 0,
        "F_B->F_A distance > 0"
    );
}

#[test]
fn seed_installs_three_extractors_wood_and_two_food_but_never_lists_raw() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_WOOD, ProductionPools, RawDeposits,
    };
    use crate::economy::{
        GOOD_FOOD, GOOD_RAW, GOOD_WOOD, HouseholdSector, InventoryBook, MarketId,
    };

    let world = seed_world();

    // (market, output-good, sell-side min_price) expected for each extractor.
    // 8031 is NO LONGER an extractor (it is the buying TOOLS producer); the WOOD
    // extractor 8041 backs its input. WOOD is authored cheap (50) so the landed
    // cost 9003->9001 (50 + 5*59 = 345) stays under the TOOLS participation
    // bound (400) and the chain can trade from tick 1.
    let expected = [
        (EXTRACTOR_FOOD_A, MarketId(9_001), GOOD_FOOD, 500), // m_a
        (EXTRACTOR_FOOD_FA, MarketId(9_003), GOOD_FOOD, 500), // m_fa
        (EXTRACTOR_WOOD, MarketId(9_003), GOOD_WOOD, 50),    // m_fa
    ];
    for (actor, market, out_good, min_price) in expected {
        let dep = world.resource::<RawDeposits>().0[&actor];
        assert_eq!(dep.good, GOOD_RAW, "{actor:?} faucets RAW");
        assert_eq!(dep.qty_per_interval.0, 10, "{actor:?} faucet rate 10");
        assert_eq!(dep.interval_ticks, 1);

        let pool = world.resource::<ProductionPools>().0[&actor].clone();
        assert_eq!(
            pool.recipe.inputs,
            vec![(GOOD_RAW, dep.qty_per_interval)],
            "{actor:?} consumes RAW"
        );
        assert_eq!(pool.recipe.outputs.len(), 1);
        assert_eq!(
            pool.recipe.outputs[0].0, out_good,
            "{actor:?} outputs the right good"
        );
        assert_eq!(
            pool.recipe.outputs[0].1, dep.qty_per_interval,
            "{actor:?} produces 1:1 (output qty == faucet rate), else RAW unbounded/scarce"
        );

        let sp = world.resource::<SupplyPools>().0[&actor];
        assert_eq!(sp.good, out_good, "{actor:?} sells its output good");
        assert_eq!(sp.market, market, "{actor:?} sells at the right market");
        assert_eq!(sp.offered_qty_per_tick.0, 10);
        assert_eq!(
            sp.min_price.0, min_price,
            "{actor:?} opening min_price pinned"
        );

        assert!(
            world
                .resource::<InventoryBook>()
                .balance(actor, GOOD_RAW)
                .available
                .0
                > 0,
            "{actor:?} holds opening RAW so production fires on tick 0"
        );
        assert!(
            !world
                .resource::<HouseholdSector>()
                .pool_weights
                .contains_key(&actor),
            "{actor:?} is a firm, not a labor household"
        );
    }

    // GOOD_RAW is NEVER on any SupplyPool or DemandPool (structural non-tradability).
    assert!(
        world
            .resource::<SupplyPools>()
            .0
            .values()
            .all(|p| p.good != GOOD_RAW),
        "RAW never on a SupplyPool"
    );
    assert!(
        world
            .resource::<DemandPools>()
            .0
            .values()
            .all(|p| p.good != GOOD_RAW),
        "RAW never on a DemandPool"
    );
}

/// Authored `capita_baseline` from the markets layer reaches `EconomyConfig`.
///
/// Seeds a world from the abutopia bundle, but overrides `household.capita_baseline = 10`
/// before calling `seed_from_markets_layer`. After seeding,
/// `EconomyConfig::capita_baseline` must equal 10.
#[test]
fn authored_capita_baseline_reaches_economy_config() {
    use crate::world::schedule::SimPlugin;
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    crate::economy::EconomyPlugin.install(&mut world, &mut schedule);
    let nodes = vec![
        node(0, 8.0, 8.0),
        node(1, 72.0, 8.0),
        node(2, 8.0, 40.0),
        node(3, 72.0, 40.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));

    // Load the real abutopia bundle, then override capita_baseline to a non-default value.
    let mut bundle =
        crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
            .expect("abutopia bundle loads");
    bundle.markets.household.capita_baseline = 10;

    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);

    assert_eq!(
        world.resource::<EconomyConfig>().capita_baseline,
        10,
        "authored capita_baseline=10 must be written into EconomyConfig after seeding"
    );
}

/// Per-capita seed scaling: `opening_cash` and `opening_inventory` must both be
/// multiplied by `capita_factor(live_count, capita_baseline)` at seed time, so the
/// initial money/goods stock matches the scaled throughput that demand will generate.
///
/// With no live agents the factor is 1 → all existing tests stay byte-identical.
/// Here we set `capita_baseline = 5` and spawn 15 `AgentMarker` entities so the
/// factor is `floor(15 / 5) = 3`. Expected post-seed totals are 3× the authored values.
#[test]
fn seed_scales_opening_cash_and_inventory_by_capita_factor() {
    use crate::economy::{AccountBook, InventoryBook};
    use crate::mobility::components::AgentMarker;
    use crate::world::schedule::SimPlugin;

    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    crate::economy::EconomyPlugin.install(&mut world, &mut schedule);
    let nodes = vec![
        node(0, 8.0, 8.0),
        node(1, 72.0, 8.0),
        node(2, 8.0, 40.0),
        node(3, 72.0, 40.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));

    let mut bundle =
        crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
            .expect("abutopia bundle loads");

    // Override baseline to 5 so spawning 15 agents gives factor = floor(15/5) = 3.
    bundle.markets.household.capita_baseline = 5;

    // Spawn 15 AgentMarker entities BEFORE seeding (ordering mirrors PR #86 fix).
    for _ in 0..15 {
        world.spawn(AgentMarker);
    }

    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);

    // ── Cash: Σ(opening_cash × 3) across all demand AND producer specs ───────
    // Producers mint opening cash too (the only other seed-mint site); their
    // input orders run at capita_factor× throughput, so the mint scales the
    // same way as the demand actors'.
    let expected_cash: i64 = bundle
        .markets
        .demand
        .iter()
        .map(|d| d.opening_cash * 3)
        .chain(bundle.markets.producers.iter().map(|p| p.opening_cash * 3))
        .sum();
    let total = world
        .resource::<AccountBook>()
        .total_money()
        .expect("summable")
        .0;
    assert_eq!(
        total, expected_cash,
        "post-seed total_money == Σ(opening_cash × factor)"
    );

    // ── Inventory: Σ(opening_inventory × 3) across all supply specs ──────────
    let expected_inventory: i64 = bundle
        .markets
        .supply
        .iter()
        .map(|s| s.opening_inventory * 3)
        .sum();
    let inv: i64 = bundle
        .markets
        .supply
        .iter()
        .map(|s| {
            world
                .resource::<InventoryBook>()
                .balance(
                    crate::economy::EconomicActorId(s.actor),
                    crate::economy::GoodId(s.good),
                )
                .available
                .0
        })
        .sum();
    assert_eq!(
        inv, expected_inventory,
        "post-seed inventory == Σ(opening_inventory × factor)"
    );
}

/// Serde-default for `HouseholdSpec.capita_baseline` is 1_000_000 (identity).
///
/// A JSON household object that omits `capita_baseline` must deserialize with the
/// identity default so that worlds not yet updated to author this field keep the
/// same behaviour (no per-capita scaling).
#[test]
fn household_spec_serde_default_capita_baseline_is_identity() {
    let spec: HouseholdSpec =
        serde_json::from_str(r#"{"population":1000000}"#).expect("deserializes without field");
    assert_eq!(
        spec.capita_baseline, 1_000_000,
        "omitted capita_baseline must default to 1_000_000 (identity)"
    );
}

/// Authored `capita_baseline` is re-applied on EVERY call to `seed_from_markets_layer`,
/// even when the economy STATE is already populated (the hydrate-from-snapshot path).
///
/// Regression for the bug where the ramp reverted to the identity default on the first
/// restart: `EconomyConfig` is rebuilt from defaults each boot and is NOT part of the
/// economy snapshot, and the config write used to sit BEHIND the idempotent state-seed
/// guard — so a hydrated world (non-empty `Markets` → early return) silently kept the
/// default 1_000_000 baseline, turning the per-capita ramp off after one session.
#[test]
fn capita_baseline_reapplies_on_hydrate_even_when_state_already_seeded() {
    use crate::world::schedule::SimPlugin;
    let mut world = World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    crate::economy::EconomyPlugin.install(&mut world, &mut schedule);
    let nodes = vec![
        node(0, 8.0, 8.0),
        node(1, 72.0, 8.0),
        node(2, 8.0, 40.0),
        node(3, 72.0, 40.0),
    ];
    world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
    world.insert_resource(Graph::new(nodes, vec![]));

    let mut bundle =
        crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
            .expect("abutopia bundle loads");

    // Phase 1 — fresh world: author an explicit baseline and confirm the fresh seed
    // applies it (and populates economy STATE). Set it explicitly rather than relying on
    // whatever abutopia's markets.json happens to author, so this test stays independent
    // of the live world-data value.
    bundle.markets.household.capita_baseline = 1_000_000;
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);
    let markets_after_first = world.resource::<Markets>().0.len();
    assert_eq!(markets_after_first, 4, "fresh seed populates the 4 markets");
    assert_eq!(
        world.resource::<EconomyConfig>().capita_baseline,
        1_000_000,
        "fresh seed applies the authored baseline"
    );

    // Phase 2 — simulate a restart where markets.json now authors a DIFFERENT baseline,
    // on a world whose economy STATE is already populated (as after a hydrate from a
    // snapshot). The idempotent state-seed guard returns early, but config must re-apply.
    bundle.markets.household.capita_baseline = 10;
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);

    // Config tracked the authored value despite the state-seed being skipped...
    assert_eq!(
        world.resource::<EconomyConfig>().capita_baseline,
        10,
        "capita_baseline must be re-applied from the layer on the hydrate path \
         (state already seeded, so the idempotent guard returns before state-seeding)"
    );
    // ...and the idempotent state-seed did NOT double-seed.
    assert_eq!(
        world.resource::<Markets>().0.len(),
        markets_after_first,
        "state-seed stays idempotent: no extra markets on the second call"
    );
}
