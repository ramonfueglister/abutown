//! Data-driven economy seeder. Reconstructs the live economy from an authored
//! `MarketLayer` (`data/worlds/.../layers/markets.json`) instead of the hardcoded
//! constants from the removed `seed` module. Seeded ONCE on fresh-world creation;
//! the economy then persists, so a hydrated world finds non-empty `Markets` and
//! this no-ops.

use bevy_ecs::prelude::*;

use crate::base_world::MarketLayer;
use crate::economy::production::{
    ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
};
use crate::economy::transport::manhattan_tiles;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, GoodId, HOUSEHOLD_SECTOR,
    HouseholdSector, InventoryBook, MarketChunks, MarketDistances, MarketGoodKey, MarketGoodState,
    MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools,
};
use crate::routing::{Graph, NodeSpatialIndex};

/// Reconstruct the economy from an authored market layer. Requires `Graph` +
/// `NodeSpatialIndex` (snaps each market anchor to the nearest real footway node,
/// exactly like the legacy seed — no coordinate is baked into the graph). Idempotent:
/// no-ops if the world already has an economy. No-ops if the graph is too small to
/// host distinct reachable nodes for every market (mirrors the legacy "graph too
/// small" early-return).
pub fn seed_from_markets_layer(world: &mut World, layer: &MarketLayer) {
    // Idempotent bootstrap: seed only when the world has no economy yet. Once seeded
    // it persists, so subsequent hydrates find markets and skip (no double-seed guard,
    // no heal-on-restore shim).
    if !world.resource::<Markets>().0.is_empty() {
        return;
    }

    // ── 1) Markets: snap each anchor → nearest footway node; insert MarketSite + MarketChunks. ──
    // Resolve every node first; if any anchor fails to snap, or two markets snap to the
    // same node, return early WITHOUT mutating the world (matches the legacy no-op).
    let mut resolved: Vec<(MarketId, &str, crate::routing::NodeId)> =
        Vec::with_capacity(layer.markets.len());
    {
        let spatial = world.resource::<NodeSpatialIndex>();
        let mut seen_nodes = std::collections::HashSet::new();
        for spec in &layer.markets {
            let node = match spatial.nearest((spec.anchor[0], spec.anchor[1])) {
                Some(n) if seen_nodes.insert(n) => n,
                // None → graph too small; duplicate node → two markets collide. Either
                // way: graph cannot host this layer's distinct markets — no-op.
                _ => return,
            };
            resolved.push((MarketId(spec.id), spec.name.as_str(), node));
        }
    }
    {
        let mut markets = world.resource_mut::<Markets>();
        for (id, name, node) in &resolved {
            markets.0.insert(
                *id,
                MarketSite {
                    id: *id,
                    node_id: *node,
                    name: (*name).to_string(),
                },
            );
        }
    }
    {
        let chunks: Vec<(MarketId, crate::ids::ChunkCoord)> = {
            let graph = world.resource::<Graph>();
            resolved
                .iter()
                .map(|(id, _, node)| {
                    let pos = graph.node(*node).position;
                    (*id, crate::mobility::chunk_of(pos.0, pos.1, 32))
                })
                .collect()
        };
        let mut anchors = world.resource_mut::<MarketChunks>();
        for (id, chunk) in chunks {
            anchors.0.insert(id, chunk);
        }
    }

    // ── 2) Market distances: bake BOTH directions for each authored pair. ──
    {
        let pairs: Vec<((MarketId, MarketId), i64)> = {
            let graph = world.resource::<Graph>();
            layer
                .distances
                .iter()
                .map(|d| {
                    let m_from = MarketId(d.from);
                    let m_to = MarketId(d.to);
                    let node_from = market_node(&resolved, m_from);
                    let node_to = market_node(&resolved, m_to);
                    let dist = manhattan_tiles(graph, node_from, node_to);
                    ((m_from, m_to), dist)
                })
                .collect()
        };
        let mut distances = world.resource_mut::<MarketDistances>();
        for ((from, to), dist) in pairs {
            distances.0.insert((from, to), dist);
            distances.0.insert((to, from), dist);
        }
    }

    // ── 3) Supply pools: opening inventory + a standing SupplyPool. ──
    for spec in &layer.supply {
        let actor = EconomicActorId(spec.actor);
        let good = GoodId(spec.good);
        world
            .resource_mut::<InventoryBook>()
            .deposit(actor, good, Quantity(spec.opening_inventory))
            .expect("seed: supplier goods");
        world.resource_mut::<SupplyPools>().0.insert(
            actor,
            SupplyPool {
                actor,
                market: MarketId(spec.market),
                good,
                offered_qty_per_tick: Quantity(spec.qty),
                min_price: Money(spec.min_price),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }

    // ── 4) Demand pools: opening cash + a standing DemandPool. ──
    for spec in &layer.demand {
        let actor = EconomicActorId(spec.actor);
        world
            .resource_mut::<AccountBook>()
            .deposit(actor, Money(spec.opening_cash))
            .expect("seed: consumer cash");
        world.resource_mut::<DemandPools>().0.insert(
            actor,
            DemandPool {
                actor,
                market: MarketId(spec.market),
                good: GoodId(spec.good),
                desired_qty_per_tick: Quantity(spec.qty),
                max_price: Money(spec.max_price),
                urgency_bps: 0,
                elasticity_bps: 0,
                interval_ticks: 1,
                last_generated_tick: None,
                last_consumed_tick: None,
                income_last_tick: Money::ZERO,
                mpc_bps: spec.mpc_bps,
                autonomous: Money(spec.autonomous),
            },
        );
    }

    // ── 5) Extractors: a RAW faucet + a RAW→out recipe + a SupplyPool for the output. ──
    // RAW is NEVER placed on a pool/market beyond the RawDeposit faucet.
    for spec in &layer.extractors {
        let actor = EconomicActorId(spec.actor);
        let in_good = GoodId(spec.in_good);
        let out_good = GoodId(spec.out_good);
        let qty = Quantity(spec.qty);
        world
            .resource_mut::<InventoryBook>()
            .deposit(actor, in_good, qty)
            .expect("seed: extractor opening raw stock");
        world.resource_mut::<RawDeposits>().0.insert(
            actor,
            RawDeposit {
                good: in_good,
                qty_per_interval: qty,
                interval_ticks: 1,
                last_regen_tick: None,
            },
        );
        world.resource_mut::<ProductionPools>().0.insert(
            actor,
            ProductionPool {
                actor,
                recipe: Recipe {
                    inputs: vec![(in_good, qty)],
                    outputs: vec![(out_good, qty)],
                },
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
        world.resource_mut::<SupplyPools>().0.insert(
            actor,
            SupplyPool {
                actor,
                market: MarketId(spec.market),
                good: out_good,
                offered_qty_per_tick: qty,
                min_price: Money(spec.min_price),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }

    // ── 6) SFC household sector: equal weight over EVERY DemandSpec actor. ──
    {
        const _: () = assert!(HOUSEHOLD_SECTOR.0 == u64::MAX - 1);
        let mut weights = std::collections::BTreeMap::new();
        for spec in &layer.demand {
            weights.insert(EconomicActorId(spec.actor), 1_i64);
        }
        assert!(
            weights.values().any(|w| *w > 0),
            "seed: HouseholdSector must have at least one positive pool weight"
        );
        assert!(
            !weights.contains_key(&HOUSEHOLD_SECTOR),
            "HOUSEHOLD_SECTOR must not collide with a seeded actor id"
        );
        world.insert_resource(HouseholdSector {
            population: layer.household.population,
            pool_weights: weights,
        });
    }

    // ── 7) Opening reference prices for each authored (market, good). ──
    // A legitimate market opening price (data), NOT a runtime fallback: set
    // `ewma_reference_price`/`last_settlement_price` ONLY when currently <= 0.
    {
        let mut goods = world.resource_mut::<MarketGoods>();
        for spec in &layer.opening_prices {
            let key = MarketGoodKey {
                market: MarketId(spec.market),
                good: GoodId(spec.good),
            };
            let state = goods
                .0
                .entry(key)
                .or_insert_with(|| MarketGoodState::new(key));
            if state.ewma_reference_price.0 <= 0 {
                state.ewma_reference_price = Money(spec.price);
            }
            if state.last_settlement_price.0 <= 0 {
                state.last_settlement_price = Money(spec.price);
            }
        }
    }
}

/// Resolve a market id to its snapped footway node within this seed pass.
/// The id MUST exist (it came from `layer.markets`, validated by the loader);
/// fail loud on the impossible — a distance spec referencing an unknown market
/// is an authoring error, not a runtime state to silently tolerate.
fn market_node(
    resolved: &[(MarketId, &str, crate::routing::NodeId)],
    id: MarketId,
) -> crate::routing::NodeId {
    resolved
        .iter()
        .find(|(m, _, _)| *m == id)
        .map(|(_, _, node)| *node)
        .unwrap_or_else(|| panic!("seed: distance references unknown market {id:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};

    /// Build an unseeded world with the four reference footway nodes that the
    /// abutopia markets layer expects, plus the full EconomyPlugin install so
    /// every resource `seed_from_markets_layer` reads or inserts exists.
    fn unseeded_world() -> World {
        use crate::world::schedule::SimPlugin;
        let mut world = World::new();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        crate::economy::EconomyPlugin.install(&mut world, &mut schedule);
        let nodes = vec![
            Node {
                id: NodeId(0),
                position: (2.0, 3.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (13.0, 3.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (16.0, 48.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(3),
                position: (208.0, 48.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ];
        world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
        world.insert_resource(Graph::new(nodes, vec![]));
        world
    }

    fn seeded_world() -> World {
        let bundle =
            crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
                .expect("abutopia bundle loads");
        let mut world = unseeded_world();
        seed_from_markets_layer(&mut world, &bundle.markets);
        world
    }

    /// The factory seeds exactly markets {9001,9002,9003,9004} with the authored names.
    #[test]
    fn markets_layer_seeds_exactly_four_named_markets() {
        let world = seeded_world();
        let markets = world.resource::<Markets>();
        let mut pairs: Vec<(u32, &str)> = markets
            .0
            .iter()
            .map(|(id, site)| (id.0, site.name.as_str()))
            .collect();
        pairs.sort_by_key(|(id, _)| *id);
        assert_eq!(
            pairs,
            vec![
                (9001, "Demo A"),
                (9002, "Demo B"),
                (9003, "Flow Demo A"),
                (9004, "Flow Demo B"),
            ],
            "exactly four markets with authored names, ascending ids"
        );
    }

    /// `MarketDistances` contains exactly the four directed edges (no diagonal, no self-loops).
    #[test]
    fn markets_layer_distances_are_exactly_two_directed_pairs() {
        let world = seeded_world();
        let distances = world.resource::<MarketDistances>();
        let mut keys: Vec<(u32, u32)> = distances.0.keys().map(|(a, b)| (a.0, b.0)).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![(9001, 9002), (9002, 9001), (9003, 9004), (9004, 9003)],
            "distances: only the two authored pairs in both directions, no diagonal"
        );
    }

    /// Supply actor ids, pool qty, min_price, and interval_ticks match the spec.
    #[test]
    fn markets_layer_supply_pools_match_spec() {
        use crate::economy::SupplyPools;
        let world = seeded_world();
        let supply = world.resource::<SupplyPools>();
        let mut actor_ids: Vec<u64> = supply.0.keys().map(|a| a.0).collect();
        actor_ids.sort();
        // Supply actors: 8001/8011/8021 (finite suppliers) + 8031/8032/8033 (extractors)
        assert_eq!(
            actor_ids,
            vec![8001, 8011, 8021, 8031, 8032, 8033],
            "supply actor ids"
        );
        for (actor, pool) in &supply.0 {
            assert_eq!(
                pool.offered_qty_per_tick,
                Quantity(10),
                "actor {actor:?}: offered_qty_per_tick == 10"
            );
            assert_eq!(
                pool.min_price,
                Money(500),
                "actor {actor:?}: min_price == 500"
            );
            assert_eq!(
                pool.interval_ticks, 1,
                "actor {actor:?}: interval_ticks == 1"
            );
        }
    }

    /// Demand actor ids, pool qty, max_price, and interval_ticks match the spec.
    #[test]
    fn markets_layer_demand_pools_match_spec() {
        use crate::economy::DemandPools;
        let world = seeded_world();
        let demand = world.resource::<DemandPools>();
        let mut actor_ids: Vec<u64> = demand.0.keys().map(|a| a.0).collect();
        actor_ids.sort();
        // Demand actors: 8002 (TOOLS consumer), 8012 (FOOD consumer), 8022 (flow FOOD consumer)
        assert_eq!(actor_ids, vec![8002, 8012, 8022], "demand actor ids");
        for (actor, pool) in &demand.0 {
            assert_eq!(
                pool.desired_qty_per_tick,
                Quantity(10),
                "actor {actor:?}: desired_qty_per_tick == 10"
            );
            assert_eq!(
                pool.max_price,
                Money(2000),
                "actor {actor:?}: max_price == 2000"
            );
            assert_eq!(
                pool.interval_ticks, 1,
                "actor {actor:?}: interval_ticks == 1"
            );
        }
    }

    /// `ProductionPools` contains exactly the three extractors (no finite suppliers).
    #[test]
    fn markets_layer_production_pools_contain_only_extractors() {
        use crate::economy::production::ProductionPools;
        let world = seeded_world();
        let prod = world.resource::<ProductionPools>();
        let mut actor_ids: Vec<u64> = prod.0.keys().map(|a| a.0).collect();
        actor_ids.sort();
        assert_eq!(
            actor_ids,
            vec![8031, 8032, 8033],
            "production pools: only the three extractor actors"
        );
        for (actor, pool) in &prod.0 {
            assert_eq!(
                pool.interval_ticks, 1,
                "actor {actor:?}: interval_ticks == 1"
            );
        }
    }

    /// `RawDeposits` contains exactly the three extractors with qty==10 and interval==1.
    #[test]
    fn markets_layer_raw_deposits_match_spec() {
        use crate::economy::production::RawDeposits;
        let world = seeded_world();
        let deps = world.resource::<RawDeposits>();
        let mut actor_ids: Vec<u64> = deps.0.keys().map(|a| a.0).collect();
        actor_ids.sort();
        assert_eq!(
            actor_ids,
            vec![8031, 8032, 8033],
            "raw deposits: exactly the three extractor actors"
        );
        for (actor, dep) in &deps.0 {
            assert_eq!(
                dep.qty_per_interval,
                Quantity(10),
                "actor {actor:?}: qty_per_interval == 10"
            );
            assert_eq!(
                dep.interval_ticks, 1,
                "actor {actor:?}: interval_ticks == 1"
            );
        }
    }

    /// `HouseholdSector.pool_weights` keys are exactly {8002,8012,8022}, all weight 1,
    /// extractors absent, HOUSEHOLD_SECTOR absent.
    #[test]
    fn markets_layer_household_sector_weights_match_spec() {
        use crate::economy::{HOUSEHOLD_SECTOR, HouseholdSector};
        let world = seeded_world();
        let hs = world.resource::<HouseholdSector>();
        let mut keys: Vec<u64> = hs.pool_weights.keys().map(|a| a.0).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![8002, 8012, 8022],
            "pool_weights keys are exactly the three consumer actors"
        );
        for (actor, &weight) in &hs.pool_weights {
            assert_eq!(weight, 1, "actor {actor:?}: weight == 1");
        }
        // Extractors must NOT appear in pool_weights (firms, not households).
        for extractor in [8031u64, 8032, 8033] {
            assert!(
                !hs.pool_weights.contains_key(&EconomicActorId(extractor)),
                "extractor {extractor} must not be in pool_weights"
            );
        }
        // HOUSEHOLD_SECTOR sentinel must not collide.
        assert!(
            !hs.pool_weights.contains_key(&HOUSEHOLD_SECTOR),
            "HOUSEHOLD_SECTOR sentinel must not appear in pool_weights"
        );
    }

    /// Opening `MarketGoodState` for the three authored (market,good) pairs has
    /// `ewma_reference_price == Money(1000)` and `last_settlement_price == Money(1000)`.
    #[test]
    fn markets_layer_opening_prices_match_spec() {
        use crate::economy::{GoodId, MarketGoodKey, MarketGoods};
        let world = seeded_world();
        let goods = world.resource::<MarketGoods>();
        let cases: &[(u32, u16, i64)] = &[
            (9002, 4, 1000), // Demo B, TOOLS
            (9002, 1, 1000), // Demo B, FOOD
            (9004, 1, 1000), // Flow Demo B, FOOD
        ];
        for &(market, good, price) in cases {
            let key = MarketGoodKey {
                market: MarketId(market),
                good: GoodId(good),
            };
            let state = goods
                .0
                .get(&key)
                .unwrap_or_else(|| panic!("MarketGoodState missing for ({market},{good})"));
            assert_eq!(
                state.ewma_reference_price,
                Money(price),
                "({market},{good}): ewma_reference_price"
            );
            assert_eq!(
                state.last_settlement_price,
                Money(price),
                "({market},{good}): last_settlement_price"
            );
        }
    }
}
