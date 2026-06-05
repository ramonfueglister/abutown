//! Data-driven economy seeder. Reconstructs the live economy from an authored
//! `MarketLayer` (`data/worlds/.../layers/markets.json`) instead of the hardcoded
//! constants in `seed::seed_demo_economy`. The output is byte-for-byte identical
//! to the legacy seed (proven by `tests/seed.rs::layer_seed_matches_legacy_seed_
//! byte_for_byte`) — this is a pure source swap (constants → data), no behaviour
//! change. Seeded ONCE on fresh-world creation; the economy then persists, so a
//! hydrated world finds non-empty `Markets` and this no-ops.

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
