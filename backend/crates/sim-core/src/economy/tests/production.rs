use crate::economy::{
    EconomicActorId, EconomyEvent, GOOD_IRON, GOOD_TOOLS, GOOD_WOOD, InventoryBook, ProductionPool,
    ProductionPools, Quantity, Recipe, TradeLedger, run_production_at_tick,
};

fn tools_recipe() -> Recipe {
    Recipe {
        inputs: vec![(GOOD_WOOD, Quantity(2_000)), (GOOD_IRON, Quantity(1_000))],
        outputs: vec![(GOOD_TOOLS, Quantity(1_000))],
    }
}

fn seed(actor: EconomicActorId, interval: u64) -> ProductionPools {
    let mut p = ProductionPools::default();
    p.0.insert(
        actor,
        ProductionPool {
            actor,
            recipe: tools_recipe(),
            interval_ticks: interval,
            last_generated_tick: None,
        },
    );
    p
}

#[test]
fn production_consumes_inputs_and_produces_outputs() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 5, 1).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_IRON).available, Quantity(0));
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(1_000));
    assert!(ledger.0.contains(&EconomyEvent::Consumed {
        actor,
        good: GOOD_WOOD,
        qty: Quantity(2_000)
    }));
    assert!(ledger.0.contains(&EconomyEvent::Consumed {
        actor,
        good: GOOD_IRON,
        qty: Quantity(1_000)
    }));
    assert!(ledger.0.contains(&EconomyEvent::Produced {
        actor,
        good: GOOD_TOOLS,
        qty: Quantity(1_000)
    }));
}

#[test]
fn production_skips_when_inputs_insufficient() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap(); // no IRON
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 5, 1).unwrap();
    assert_eq!(inv.balance(actor, GOOD_WOOD).available, Quantity(2_000)); // unchanged
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(0));
    assert!(ledger.0.is_empty());
    assert_eq!(prod.0[&actor].last_generated_tick, Some(5)); // cadence still advances
}

#[test]
fn production_respects_interval() {
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(4_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(2_000)).unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 10);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0, 1).unwrap(); // produces (last=None)
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 3, 1).unwrap(); // interval not elapsed → skip
    assert_eq!(inv.balance(actor, GOOD_TOOLS).available, Quantity(1_000)); // only one batch
}

#[test]
fn production_conserves_money() {
    use crate::economy::{AccountBook, Money};
    let actor = EconomicActorId(1);
    let mut inv = InventoryBook::default();
    inv.deposit(actor, GOOD_WOOD, Quantity(2_000)).unwrap();
    inv.deposit(actor, GOOD_IRON, Quantity(1_000)).unwrap();
    let mut accounts = AccountBook::default();
    accounts.deposit(actor, Money(5_000)).unwrap();
    let before = accounts.total_money().unwrap();
    let mut ledger = TradeLedger::default();
    let mut prod = seed(actor, 1);
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 1, 1).unwrap();
    assert_eq!(accounts.total_money().unwrap(), before); // production never touches money
}

#[test]
fn production_is_deterministic() {
    let run = || {
        let a1 = EconomicActorId(2);
        let a2 = EconomicActorId(1);
        let mut inv = InventoryBook::default();
        for a in [a1, a2] {
            inv.deposit(a, GOOD_WOOD, Quantity(2_000)).unwrap();
            inv.deposit(a, GOOD_IRON, Quantity(1_000)).unwrap();
        }
        let mut ledger = TradeLedger::default();
        let mut prod = ProductionPools::default();
        for a in [a1, a2] {
            prod.0.insert(
                a,
                ProductionPool {
                    actor: a,
                    recipe: tools_recipe(),
                    interval_ticks: 1,
                    last_generated_tick: None,
                },
            );
        }
        run_production_at_tick(&mut inv, &mut ledger, &mut prod, 1, 1).unwrap();
        ledger.0
    };
    assert_eq!(run(), run());
}

#[test]
fn good_raw_is_the_next_free_good_id_and_distinct() {
    use crate::economy::{GOOD_FOOD, GOOD_IRON, GOOD_RAW, GOOD_TOOLS, GOOD_WOOD, GoodId};
    assert_eq!(GOOD_RAW, GoodId(5));
    for g in [GOOD_FOOD, GOOD_WOOD, GOOD_IRON, GOOD_TOOLS] {
        assert_ne!(
            g, GOOD_RAW,
            "GOOD_RAW must not collide with a tradable good"
        );
    }
}

#[test]
fn regenerated_event_type_tag_is_stable() {
    use crate::economy::{EconomicActorId, EconomyEvent, GOOD_RAW, Quantity};
    let e = EconomyEvent::Regenerated {
        actor: EconomicActorId(8_031),
        good: GOOD_RAW,
        qty: Quantity(100),
    };
    assert_eq!(e.event_type(), "regenerated");
}

#[test]
fn regen_deposits_faucet_on_interval_and_stamps_cursor() {
    use crate::economy::production::{EXTRACTOR_TOOLS, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{EconomyEvent, GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );

    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 5, 1).unwrap();

    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_RAW).available,
        Quantity(100)
    );
    assert_eq!(deposits.0[&EXTRACTOR_TOOLS].last_regen_tick, Some(5));
    assert!(ledger.0.contains(&EconomyEvent::Regenerated {
        actor: EXTRACTOR_TOOLS,
        good: GOOD_RAW,
        qty: Quantity(100),
    }));
}

#[test]
fn regen_skips_within_interval_but_does_not_advance_cursor_on_skip() {
    use crate::economy::production::{EXTRACTOR_TOOLS, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 10,
            last_regen_tick: None,
        },
    );
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0, 1).unwrap(); // fires (last=None)
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 3, 1).unwrap(); // interval not elapsed
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_RAW).available,
        Quantity(100),
        "only one deposit within the interval"
    );
    // On a skip the gate returns BEFORE stamping, so the cursor stays at the firing tick.
    assert_eq!(deposits.0[&EXTRACTOR_TOOLS].last_regen_tick, Some(0));
}

#[test]
fn regen_is_flow_capped_not_capacity_capped() {
    use crate::economy::production::{EXTRACTOR_TOOLS, RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    // No recipe consuming RAW here: deposits stack unboundedly per interval (faucet,
    // not a level-capped reservoir). The recipe is what bounds RAW in the live loop.
    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    deposits.0.insert(
        EXTRACTOR_TOOLS,
        RawDeposit {
            good: GOOD_RAW,
            qty_per_interval: Quantity(100),
            interval_ticks: 1,
            last_regen_tick: None,
        },
    );
    for t in 0..3 {
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, t, 1).unwrap();
    }
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_RAW).available,
        Quantity(300)
    );
}

#[test]
fn regen_is_deterministic_keys_first() {
    use crate::economy::production::{RawDeposit, RawDeposits, run_regen_at_tick};
    use crate::economy::{EconomicActorId, GOOD_RAW, InventoryBook, Quantity, TradeLedger};
    use std::collections::BTreeMap;

    let run = || {
        let mut inv = InventoryBook::default();
        let mut ledger = TradeLedger::default();
        let mut deposits = RawDeposits(BTreeMap::new());
        // Insert out of ascending order to prove keys-first iteration.
        for a in [EconomicActorId(9), EconomicActorId(2)] {
            deposits.0.insert(
                a,
                RawDeposit {
                    good: GOOD_RAW,
                    qty_per_interval: Quantity(50),
                    interval_ticks: 1,
                    last_regen_tick: None,
                },
            );
        }
        run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 1, 1).unwrap();
        ledger.0
    };
    assert_eq!(run(), run());
}

#[test]
fn regen_rate_covers_aggregate_tools_demand_at_seed() {
    // §15.2 Sizing-Sim: the EXTRACTOR_TOOLS's faucet (and its same-rate recipe + SupplyPool)
    // MUST cover aggregate per-tick TOOLS demand at seed prices, else static prices leave
    // chronic TOOLS scarcity (§8). Build the live demo world, measure aggregate TOOLS
    // demand, and assert the seeded faucet rate >= that demand.
    use crate::economy::production::{EXTRACTOR_TOOLS, RawDeposits};
    use crate::economy::{DemandPools, GOOD_TOOLS};

    // Build the world inline with the same minimal spatial scaffold the seeder needs,
    // keeping this test self-contained and independent of the seed test module.
    let mut world = bevy_ecs::world::World::new();
    {
        use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
        let node = |id: u32, x: f32, y: f32| Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        };
        let nodes = vec![
            node(0, 2.0, 3.0),
            node(1, 111.5, 64.51),
            node(2, 16.0, 48.0),
            node(3, 208.0, 48.0),
        ];
        world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
        world.insert_resource(Graph::new(nodes, vec![]));
        world.insert_resource(crate::economy::Markets::default());
        world.insert_resource(crate::economy::MarketChunks::default());
        world.insert_resource(crate::economy::AccountBook::default());
        world.insert_resource(crate::economy::InventoryBook::default());
        world.insert_resource(crate::economy::SupplyPools::default());
        world.insert_resource(crate::economy::DemandPools::default());
        world.insert_resource(crate::economy::MarketDistances::default());
        world.insert_resource(crate::economy::MarketGoods::default());
        world.insert_resource(crate::economy::production::ProductionPools::default());
        world.insert_resource(crate::economy::production::RawDeposits::default());
        // EconomyConfig is required by seed_from_markets_layer (capita_baseline lookup).
        world.insert_resource(crate::economy::EconomyConfig::default());
    }
    let bundle = crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);

    let aggregate_tools_demand: i64 = world
        .resource::<DemandPools>()
        .0
        .values()
        .filter(|p| p.good == GOOD_TOOLS)
        .map(|p| p.desired_qty_per_tick.0)
        .sum();
    assert_eq!(
        aggregate_tools_demand, 10,
        "seed has exactly one TOOLS consumer @ 10/tick (sizing baseline)"
    );

    let faucet = world.resource::<RawDeposits>().0[&EXTRACTOR_TOOLS];
    assert!(
        faucet.qty_per_interval.0 >= aggregate_tools_demand && faucet.interval_ticks == 1,
        "EXTRACTOR_TOOLS faucet rate ({} per {} tick(s)) must cover aggregate TOOLS demand ({}/tick) \
         at seed prices, else chronic scarcity (§8/§15.2)",
        faucet.qty_per_interval.0,
        faucet.interval_ticks,
        aggregate_tools_demand
    );
}

#[test]
fn food_extractor_ids_are_free_and_distinct() {
    use crate::economy::EconomicActorId;
    use crate::economy::production::{EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS};
    // Distinct from the TOOLS extractor and from each other.
    assert_ne!(EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS);
    assert_ne!(EXTRACTOR_FOOD_FA, EXTRACTOR_TOOLS);
    assert_ne!(EXTRACTOR_FOOD_A, EXTRACTOR_FOOD_FA);
    // Distinct from every seeded actor id (8_001..8_022, 8_031).
    for seeded in [8_001u64, 8_002, 8_011, 8_012, 8_021, 8_022, 8_031] {
        assert_ne!(EXTRACTOR_FOOD_A, EconomicActorId(seeded));
        assert_ne!(EXTRACTOR_FOOD_FA, EconomicActorId(seeded));
    }
}

#[test]
fn two_extractors_make_distinct_goods_and_each_balances_its_own_raw() {
    use crate::economy::production::{
        EXTRACTOR_FOOD_A, EXTRACTOR_TOOLS, ProductionPool, ProductionPools, RawDeposit,
        RawDeposits, Recipe, run_production_at_tick, run_regen_at_tick,
    };
    use crate::economy::{
        EconomyEvent, GOOD_FOOD, GOOD_RAW, GOOD_TOOLS, InventoryBook, Quantity, TradeLedger,
    };
    use std::collections::BTreeMap;

    let mut inv = InventoryBook::default();
    let mut ledger = TradeLedger::default();
    let mut deposits = RawDeposits(BTreeMap::new());
    let mut prod = ProductionPools::default();
    for (actor, out) in [(EXTRACTOR_TOOLS, GOOD_TOOLS), (EXTRACTOR_FOOD_A, GOOD_FOOD)] {
        deposits.0.insert(
            actor,
            RawDeposit {
                good: GOOD_RAW,
                qty_per_interval: Quantity(10),
                interval_ticks: 1,
                last_regen_tick: None,
            },
        );
        prod.0.insert(
            actor,
            ProductionPool {
                actor,
                recipe: Recipe {
                    inputs: vec![(GOOD_RAW, Quantity(10))],
                    outputs: vec![(out, Quantity(10))],
                },
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }

    // One tick: regen (deposits RAW) then production (consumes RAW, emits goods).
    run_regen_at_tick(&mut inv, &mut ledger, &mut deposits, 0, 1).unwrap();
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0, 1).unwrap();

    // Each extractor produced its OWN good...
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_TOOLS).available,
        Quantity(10)
    );
    assert_eq!(
        inv.balance(EXTRACTOR_FOOD_A, GOOD_FOOD).available,
        Quantity(10)
    );
    // ...and the FOOD extractor made NO tools, the TOOLS extractor made NO food.
    assert_eq!(
        inv.balance(EXTRACTOR_FOOD_A, GOOD_TOOLS).available,
        Quantity(0)
    );
    assert_eq!(
        inv.balance(EXTRACTOR_TOOLS, GOOD_FOOD).available,
        Quantity(0)
    );

    // Per-actor RAW balance: each regenerated 10 and consumed 10 -> net 0 on hand.
    for actor in [EXTRACTOR_TOOLS, EXTRACTOR_FOOD_A] {
        let regen: i64 = ledger
            .0
            .iter()
            .filter_map(|e| match e {
                EconomyEvent::Regenerated {
                    actor: a,
                    good: GOOD_RAW,
                    qty,
                } if *a == actor => Some(qty.0),
                _ => None,
            })
            .sum();
        let consumed: i64 = ledger
            .0
            .iter()
            .filter_map(|e| match e {
                EconomyEvent::Consumed {
                    actor: a,
                    good: GOOD_RAW,
                    qty,
                } if *a == actor => Some(qty.0),
                _ => None,
            })
            .sum();
        assert_eq!(
            regen, consumed,
            "actor {actor:?} RAW regenerated == consumed"
        );
        assert_eq!(
            inv.balance(actor, GOOD_RAW).available,
            Quantity(0),
            "actor {actor:?} RAW on-hand 0"
        );
    }

    // Throttle: with RAW exhausted (consumed this tick) and no regen on a within-interval
    // call, no further FOOD/TOOLS is produced.
    run_production_at_tick(&mut inv, &mut ledger, &mut prod, 0, 1).unwrap(); // same tick, interval not elapsed
    assert_eq!(
        inv.balance(EXTRACTOR_FOOD_A, GOOD_FOOD).available,
        Quantity(10),
        "no double-produce"
    );
}

#[test]
fn faucet_rate_covers_routed_demand_per_consumer_pool_at_seed() {
    use crate::economy::production::{ProductionPools, RawDeposits};
    use crate::economy::{DemandPools, GoodId, MarketDistances, MarketId, SupplyPools};

    // Helper: for a given world's pools/deposits, for every consumer DemandPool compute the
    // continuous faucet supply of its good reachable from its demand-market and assert >= demand.
    // "Reachable" = same market, or a supply market s with a MarketDistances entry (s, d).
    // Returns the list of (consumer_market, good, demand, reachable_faucet) for inspection.
    fn check(
        demand: &DemandPools,
        supply: &SupplyPools,
        deposits: &RawDeposits,
        production: &ProductionPools,
        distances: &MarketDistances,
    ) -> Vec<(MarketId, GoodId, i64, i64)> {
        let mut rows = Vec::new();
        for (&actor, dp) in demand.0.iter() {
            let d_market = dp.market;
            let g = dp.good;
            let need = dp.desired_qty_per_tick.0;
            let mut reachable_faucet: i64 = 0;
            for (&s_actor, sp) in supply.0.iter() {
                if sp.good != g {
                    continue;
                }
                // Only count CONTINUOUS supply (an extractor faucet), not finite endowment.
                let Some(dep) = deposits.0.get(&s_actor) else {
                    continue;
                };
                // The faucet feeds a recipe whose output is this good at this supply pool.
                let produces_g = production
                    .0
                    .get(&s_actor)
                    .map(|p| p.recipe.outputs.iter().any(|(og, _)| *og == g))
                    .unwrap_or(false);
                if !produces_g {
                    continue;
                }
                let s_market = sp.market;
                let reaches =
                    s_market == d_market || distances.0.contains_key(&(s_market, d_market));
                if reaches {
                    reachable_faucet += dep.qty_per_interval.0 / dep.interval_ticks.max(1) as i64;
                }
            }
            let _ = actor;
            rows.push((d_market, g, need, reachable_faucet));
        }
        rows
    }

    // Build the live seed world (same builder as the sizing test above).
    let mut world = bevy_ecs::world::World::new();
    {
        use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
        let node = |id: u32, x: f32, y: f32| Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        };
        let nodes = vec![
            node(0, 2.0, 3.0),
            node(1, 111.5, 64.51),
            node(2, 16.0, 48.0),
            node(3, 208.0, 48.0),
        ];
        world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
        world.insert_resource(Graph::new(nodes, vec![]));
        world.insert_resource(crate::economy::Markets::default());
        world.insert_resource(crate::economy::MarketChunks::default());
        world.insert_resource(crate::economy::AccountBook::default());
        world.insert_resource(crate::economy::InventoryBook::default());
        world.insert_resource(SupplyPools::default());
        world.insert_resource(DemandPools::default());
        world.insert_resource(MarketDistances::default());
        world.insert_resource(crate::economy::MarketGoods::default());
        world.insert_resource(ProductionPools::default());
        world.insert_resource(RawDeposits::default());
        // EconomyConfig is required by seed_from_markets_layer (capita_baseline lookup).
        world.insert_resource(crate::economy::EconomyConfig::default());
    }
    let bundle = crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);

    let rows = check(
        world.resource::<DemandPools>(),
        world.resource::<SupplyPools>(),
        world.resource::<RawDeposits>(),
        world.resource::<ProductionPools>(),
        world.resource::<MarketDistances>(),
    );
    assert!(
        !rows.is_empty(),
        "non-vacuous: there are consumer pools to check"
    );
    for (market, good, need, faucet) in &rows {
        assert!(
            faucet >= need,
            "consumer @ {market:?} for {good:?} demands {need}/tick but reachable continuous faucet is {faucet}/tick",
        );
    }

    // NEGATIVE CONTROL — prove the invariant is not vacuous: move BOTH FOOD faucets to m_a.
    // Then 8_022's demand @ m_fb (reachable only from m_fa) loses its faucet and the check must
    // report a shortfall (reachable_faucet == 0 for that FOOD pool).
    {
        use crate::economy::production::EXTRACTOR_FOOD_FA;
        let mut sp = world.resource_mut::<SupplyPools>();
        sp.0.get_mut(&EXTRACTOR_FOOD_FA).unwrap().market = MarketId(9_001); // move m_fa -> m_a
    }
    let broken = check(
        world.resource::<DemandPools>(),
        world.resource::<SupplyPools>(),
        world.resource::<RawDeposits>(),
        world.resource::<ProductionPools>(),
        world.resource::<MarketDistances>(),
    );
    use crate::economy::GOOD_FOOD;
    let fb_food = broken
        .iter()
        .find(|(m, g, _, _)| *m == MarketId(9_004) && *g == GOOD_FOOD)
        .expect("there is a FOOD consumer @ m_fb");
    assert!(
        fb_food.2 > fb_food.3,
        "negative control: FOOD @ m_fb demand {} must now EXCEED reachable faucet {} (proves the check binds on routing)",
        fb_food.2,
        fb_food.3,
    );
}
