//! Data-driven economy seeder. Reconstructs the live economy from an authored
//! `MarketLayer` (`data/worlds/.../layers/markets.json`) instead of the hardcoded
//! constants from the removed `seed` module. Stock/account/pool state is seeded
//! once and then persists; authored geometry/config such as market anchors and
//! producer policies is re-applied on every boot.

use bevy_ecs::prelude::*;

use crate::base_world::MarketLayer;
use crate::economy::producers::{InputPool, InputPools, ProducerPolicies, ProducerPolicy};
use crate::economy::production::{
    ProductionPool, ProductionPools, RawDeposit, RawDeposits, Recipe,
};
use crate::economy::transport::manhattan_tiles;
use crate::economy::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, GOOD_RAW, GoodId, HOUSEHOLD_SECTOR,
    HouseholdSector, InventoryBook, MarketChunks, MarketDistances, MarketGoodKey, MarketGoodState,
    MarketGoods, MarketId, MarketSite, Markets, Money, Quantity, SupplyPool, SupplyPools,
};
use crate::routing::{Graph, NodeSpatialIndex};

type ResolvedMarketSite = (MarketId, String, crate::routing::NodeId);

/// Reconstruct the economy from an authored market layer. Requires `Graph` +
/// `NodeSpatialIndex` (snaps each market anchor to the nearest real footway node,
/// exactly like the legacy seed — no coordinate is baked into the graph). Idempotent:
/// no-ops if the world already has an economy. No-ops if the graph is too small to
/// host distinct reachable nodes for every market (mirrors the legacy "graph too
/// small" early-return).
pub fn seed_from_markets_layer(world: &mut World, layer: &MarketLayer) {
    // Authored data is validated on EVERY boot (both the fresh-seed and the hydrate
    // path read the layer), failing loud on authoring errors — the file's pattern.
    validate_producer_specs(layer);

    // Authored economy CONFIG is re-applied on every boot, BEFORE the idempotent
    // state-seed guard below. `capita_baseline` lives in `EconomyConfig`, which is rebuilt
    // from defaults each start and is NOT part of the economy snapshot — so unless it is
    // re-read from the layer here, a hydrated world (non-empty `Markets` → early return)
    // would silently fall back to the default identity baseline, turning the per-capita
    // ramp OFF on the first restart after seeding. Applying it here makes "edit
    // markets.json + restart" actually retune a persisted world, and keeps config
    // tracking the authored data rather than a stale snapshot-era default.
    if let Some(mut cfg) = world.get_resource_mut::<crate::economy::EconomyConfig>() {
        cfg.capita_baseline = layer.household.capita_baseline;
    }

    let Some(resolved) = resolve_market_sites(world, layer) else {
        return;
    };

    // Idempotent STOCK/POOL bootstrap: seed pools/accounts only when the world has no
    // economy yet. Market geometry is authored config and is re-applied above the
    // hydrate branch so persisted simulations follow map authoring changes without
    // deleting balances.
    if !world.resource::<Markets>().0.is_empty() {
        apply_market_sites_and_distances(world, layer, &resolved);
        // HYDRATE path: `ProducerPolicies` is authored CONFIG (NOT persisted) — rebuild
        // it from the layer unconditionally, same #83 lesson as `capita_baseline` above:
        // without this, a restart would silently revert every producer to the #75
        // defaults (θ=100%, no working-capital buffer). Then enforce the keyset
        // invariant upfront against the snapshot-hydrated `InputPools` (review I1).
        apply_producer_policies(world, layer);
        assert_producer_keysets_match(world);
        return;
    }

    // ── 1-2) Markets + distances: authored geometry/config, re-applied on every boot. ──
    apply_market_sites_and_distances(world, layer, &resolved);

    // ── Per-capita seed scaling (spec 2026-06-06 §2b). Demand will run at
    // capita_factor× throughput, so the seeded money/goods stock must be
    // factor× too — money is still minted exactly once, only more of it.
    // With no live agents (unit tests, economy-only worlds) the factor is 1
    // and seeding stays byte-identical to the unscaled economy.
    // Extractor opening RAW stock stays 1× deliberately: it is a 1-tick buffer;
    // the regeneration faucet itself is capita-scaled at runtime (production.rs).
    let seed_factor: i64 = {
        let live = crate::economy::capita::live_agent_count(world);
        let baseline = world
            .resource::<crate::economy::systems::EconomyConfig>()
            .capita_baseline;
        crate::economy::capita::capita_factor(live, baseline)
    };

    // ── 3) Supply pools: opening inventory + a standing SupplyPool. ──
    for spec in &layer.supply {
        let actor = EconomicActorId(spec.actor);
        let good = GoodId(spec.good);
        world
            .resource_mut::<InventoryBook>()
            .deposit(
                actor,
                good,
                Quantity(
                    i64::try_from((spec.opening_inventory as i128) * (seed_factor as i128))
                        .expect("seed: opening_inventory × capita factor overflows i64"),
                ),
            )
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
            .deposit(
                actor,
                Money(
                    i64::try_from((spec.opening_cash as i128) * (seed_factor as i128))
                        .expect("seed: opening_cash × capita factor overflows i64"),
                ),
            )
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

    // ── 5b) Producers: buying firms — five pieces per spec. ──
    // ProductionPool (Leontief recipe in→out), InputPool (derived input demand at the
    // home market; `max_price` starts at ZERO and is discovered by the order-generation
    // pass), sell-side SupplyPool (identical shape to the extractor's), and the
    // opening-cash mint (the demand-actor seed-mint pattern — the ONLY permitted
    // non-transfer money creation; the #78 audit baselines AFTER seeding). NO RawDeposit
    // and NO opening input inventory: a producer buys its input on the market.
    // `ProducerPolicies` (authored config) is applied below for BOTH paths.
    for spec in &layer.producers {
        let actor = EconomicActorId(spec.actor);
        let in_good = GoodId(spec.in_good);
        let out_good = GoodId(spec.out_good);
        world
            .resource_mut::<AccountBook>()
            .deposit(
                actor,
                Money(
                    i64::try_from((spec.opening_cash as i128) * (seed_factor as i128))
                        .expect("seed: producer opening_cash × capita factor overflows i64"),
                ),
            )
            .expect("seed: producer opening cash");
        world.resource_mut::<ProductionPools>().0.insert(
            actor,
            ProductionPool {
                actor,
                recipe: Recipe {
                    inputs: vec![(in_good, Quantity(spec.in_qty))],
                    outputs: vec![(out_good, Quantity(spec.out_qty))],
                },
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
        world.resource_mut::<InputPools>().0.insert(
            actor,
            InputPool {
                actor,
                market: MarketId(spec.market),
                good: in_good,
                in_qty: Quantity(spec.in_qty),
                out_qty: Quantity(spec.out_qty),
                out_good,
                interval_ticks: 1,
                last_generated_tick: None,
                max_price: Money::ZERO,
            },
        );
        world.resource_mut::<SupplyPools>().0.insert(
            actor,
            SupplyPool {
                actor,
                market: MarketId(spec.market),
                good: out_good,
                offered_qty_per_tick: Quantity(spec.qty),
                min_price: Money(spec.min_price),
                interval_ticks: 1,
                last_generated_tick: None,
            },
        );
    }
    apply_producer_policies(world, layer);
    assert_producer_keysets_match(world);

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

/// Snap authored market anchors to real graph nodes. Resolve every node first;
/// if any anchor fails to snap, or two markets snap to the same node, return
/// `None` without mutating the world (matches the legacy fresh-seed no-op).
fn resolve_market_sites(world: &World, layer: &MarketLayer) -> Option<Vec<ResolvedMarketSite>> {
    let spatial = world.resource::<NodeSpatialIndex>();
    let mut seen_nodes = std::collections::HashSet::new();
    let mut resolved = Vec::with_capacity(layer.markets.len());
    for spec in &layer.markets {
        let node = spatial.nearest((spec.anchor[0], spec.anchor[1]))?;
        if !seen_nodes.insert(node) {
            return None;
        }
        resolved.push((MarketId(spec.id), spec.name.clone(), node));
    }
    Some(resolved)
}

fn apply_market_sites_and_distances(
    world: &mut World,
    layer: &MarketLayer,
    resolved: &[ResolvedMarketSite],
) {
    {
        let mut markets = world.resource_mut::<Markets>();
        for (id, name, node) in resolved {
            markets.0.insert(
                *id,
                MarketSite {
                    id: *id,
                    node_id: *node,
                    name: name.clone(),
                },
            );
        }
    }

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
    {
        let mut anchors = world.resource_mut::<MarketChunks>();
        for (id, chunk) in chunks {
            anchors.0.insert(id, chunk);
        }
    }

    let pairs: Vec<((MarketId, MarketId), i64)> = {
        let graph = world.resource::<Graph>();
        layer
            .distances
            .iter()
            .map(|d| {
                let m_from = MarketId(d.from);
                let m_to = MarketId(d.to);
                let node_from = market_node(resolved, m_from);
                let node_to = market_node(resolved, m_to);
                let dist = manhattan_tiles(graph, node_from, node_to);
                ((m_from, m_to), dist)
            })
            .collect()
    };
    {
        let mut distances = world.resource_mut::<MarketDistances>();
        distances.0.clear();
        for ((from, to), dist) in pairs {
            distances.0.insert((from, to), dist);
            distances.0.insert((to, from), dist);
        }
    }
}

/// Validate every authored `ProducerSpec` — fail-loud on authoring errors (the
/// file's pattern: `assert!`/`panic!`, mirroring the household-sector asserts).
/// Runs on EVERY boot, before either the fresh-seed or the hydrate path.
fn validate_producer_specs(layer: &MarketLayer) {
    let demand_actors: std::collections::BTreeSet<u64> =
        layer.demand.iter().map(|d| d.actor).collect();
    let mut seen_producers = std::collections::BTreeSet::new();
    for spec in &layer.producers {
        let actor = spec.actor;
        assert!(
            layer.markets.iter().any(|m| m.id == spec.market),
            "seed: producer {actor}: market {} not in layer.markets",
            spec.market
        );
        assert!(
            GoodId(spec.in_good) != GOOD_RAW,
            "seed: producer {actor}: in_good must not be GOOD_RAW({}) — RAW is \
             structurally non-tradable (extractor faucets only), the chain would be \
             dead at seed",
            GOOD_RAW.0
        );
        assert!(
            spec.theta_bps <= 10_000,
            "seed: producer {actor}: theta_bps {} > 10_000 (ProducerPolicy promises \
             0..=10_000)",
            spec.theta_bps
        );
        assert!(
            spec.batches_target >= 1,
            "seed: producer {actor}: batches_target must be >= 1"
        );
        assert!(
            spec.in_qty > 0 && spec.out_qty > 0,
            "seed: producer {actor}: in_qty/out_qty must be > 0 (got {}/{})",
            spec.in_qty,
            spec.out_qty
        );
        assert!(
            spec.qty > 0 && spec.min_price > 0,
            "seed: producer {actor}: sell-side qty/min_price must be > 0 (got {}/{})",
            spec.qty,
            spec.min_price
        );
        assert!(
            spec.opening_cash >= 0,
            "seed: producer {actor}: opening_cash must be >= 0"
        );
        // Role disjointness: the demand and input order-generation passes in
        // systems.rs are independent ONLY because no actor is in both maps.
        assert!(
            !demand_actors.contains(&actor),
            "seed: actor {actor} is in BOTH DemandSpec and ProducerSpec roles — the \
             independent demand/input order passes require disjoint actor sets"
        );
        assert!(
            !layer.supply.iter().any(|s| s.actor == actor)
                && !layer.extractors.iter().any(|e| e.actor == actor),
            "seed: actor {actor} is also a SupplySpec/ExtractorSpec actor — the \
             producer seed would silently overwrite its pools"
        );
        assert!(
            seen_producers.insert(actor),
            "seed: duplicate producer actor {actor}"
        );
        // The chain must be alive from tick 0: SOMETHING must output `in_good`
        // (a finite supplier, an extractor, or another producer — any market;
        // macro_flow moves it between markets).
        let has_input_supply = layer.supply.iter().any(|s| s.good == spec.in_good)
            || layer.extractors.iter().any(|e| e.out_good == spec.in_good)
            || layer
                .producers
                .iter()
                .any(|p| p.actor != actor && p.out_good == spec.in_good);
        assert!(
            has_input_supply,
            "seed: producer {actor}: no supply path for in_good {} — no SupplySpec, \
             ExtractorSpec, or other ProducerSpec outputs it (chain dead at seed)",
            spec.in_good
        );
    }
}

/// Rebuild `ProducerPolicies` from the authored layer — UNCONDITIONAL overwrite
/// (authored is truth). Policies are config, not state: never persisted, re-applied
/// on every boot exactly like `capita_baseline` (the #83 lesson).
fn apply_producer_policies(world: &mut World, layer: &MarketLayer) {
    let mut policies = ProducerPolicies::default();
    for spec in &layer.producers {
        policies.0.insert(
            EconomicActorId(spec.actor),
            ProducerPolicy {
                theta_bps: spec.theta_bps,
                batches_target: spec.batches_target,
            },
        );
    }
    world.insert_resource(policies);
}

/// Review I1: `ProducerPolicies` and `InputPools` are ONLY ever valid together —
/// enforce keys(policies) == keys(input_pools) upfront (after seed AND after
/// re-apply), not lazily in the dividend/order paths. A mismatch means the layer's
/// `producers` diverged from a persisted snapshot's `InputPools` (#83-class config
/// revert): fix the layer, or `DELETE FROM economy_snapshots` for an intentional
/// producer add/remove.
fn assert_producer_keysets_match(world: &World) {
    let policy_keys: Vec<EconomicActorId> = world
        .resource::<ProducerPolicies>()
        .0
        .keys()
        .copied()
        .collect();
    let pool_keys: Vec<EconomicActorId> =
        world.resource::<InputPools>().0.keys().copied().collect();
    assert_eq!(
        policy_keys, pool_keys,
        "seed: keys(ProducerPolicies) != keys(InputPools) — the authored producers \
         diverged from the persisted InputPools (one-sided state is a #83-class \
         config revert; fix markets.json or DELETE FROM economy_snapshots)"
    );
}

/// Resolve a market id to its snapped footway node within this seed pass.
/// The id MUST exist (it came from `layer.markets`, validated by the loader);
/// fail loud on the impossible — a distance spec referencing an unknown market
/// is an authoring error, not a runtime state to silently tolerate.
fn market_node(resolved: &[ResolvedMarketSite], id: MarketId) -> crate::routing::NodeId {
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
                position: (8.0, 8.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (72.0, 8.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (8.0, 40.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(3),
                position: (72.0, 40.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(4),
                position: (40.0, 8.0),
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
                (9001, "Central Works"),
                (9002, "Market Square"),
                (9003, "Harbor Depot"),
                (9004, "Homes Quarter"),
            ],
            "exactly four markets with authored names, ascending ids"
        );
    }

    /// `MarketDistances` contains exactly the six directed edges of the three authored
    /// pairs (9001↔9002, 9003↔9004, and 9001↔9003 — the WOOD route) — no self-loops.
    #[test]
    fn markets_layer_distances_are_exactly_three_directed_pairs() {
        let world = seeded_world();
        let distances = world.resource::<MarketDistances>();
        let mut keys: Vec<(u32, u32)> = distances.0.keys().map(|(a, b)| (a.0, b.0)).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                (9001, 9002),
                (9001, 9003),
                (9002, 9001),
                (9003, 9001),
                (9003, 9004),
                (9004, 9003),
            ],
            "distances: only the three authored pairs in both directions"
        );
        // The WOOD route 9003→9001 runs up the west edge: |8-8| + |40-8| = 32.
        // macro_flow prunes cross-edges without a distance entry ("no known route"),
        // so this edge is what makes the input chain physically possible.
        assert_eq!(
            distances.0[&(MarketId(9003), MarketId(9001))],
            32,
            "WOOD route distance baked from the snapped anchors"
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
        // Supply actors: 8001/8011/8021 (finite suppliers) + 8031 (TOOLS producer,
        // sell side) + 8032/8033 (FOOD extractors) + 8041 (WOOD extractor)
        assert_eq!(
            actor_ids,
            vec![8001, 8011, 8021, 8031, 8032, 8033, 8041],
            "supply actor ids"
        );
        for (actor, pool) in &supply.0 {
            assert_eq!(
                pool.offered_qty_per_tick,
                Quantity(10),
                "actor {actor:?}: offered_qty_per_tick == 10"
            );
            // WOOD (8041) is authored cheap so the chain can trade from tick 1:
            // the TOOLS participation bound is 400 and transport 9003→9001 adds
            // 295/unit, so the WOOD reservation must sit well below 105.
            let expected_min = if actor.0 == 8041 { 50 } else { 500 };
            assert_eq!(
                pool.min_price,
                Money(expected_min),
                "actor {actor:?}: min_price == {expected_min}"
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

    /// `ProductionPools` contains exactly the three extractors plus the TOOLS producer
    /// (no finite suppliers).
    #[test]
    fn markets_layer_production_pools_contain_extractors_and_producer() {
        use crate::economy::production::ProductionPools;
        let world = seeded_world();
        let prod = world.resource::<ProductionPools>();
        let mut actor_ids: Vec<u64> = prod.0.keys().map(|a| a.0).collect();
        actor_ids.sort();
        assert_eq!(
            actor_ids,
            vec![8031, 8032, 8033, 8041],
            "production pools: the TOOLS producer + the three extractor actors"
        );
        for (actor, pool) in &prod.0 {
            assert_eq!(
                pool.interval_ticks, 1,
                "actor {actor:?}: interval_ticks == 1"
            );
        }
    }

    /// `RawDeposits` contains exactly the three extractors with qty==10 and interval==1
    /// — and NOT the producer (8031 buys WOOD; it has no faucet).
    #[test]
    fn markets_layer_raw_deposits_match_spec() {
        use crate::economy::production::RawDeposits;
        let world = seeded_world();
        let deps = world.resource::<RawDeposits>();
        let mut actor_ids: Vec<u64> = deps.0.keys().map(|a| a.0).collect();
        actor_ids.sort();
        assert_eq!(
            actor_ids,
            vec![8032, 8033, 8041],
            "raw deposits: exactly the three extractor actors (producer 8031 excluded)"
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

    /// Load the authored abutopia layer (for mutation-based validation tests).
    fn abutopia_layer() -> MarketLayer {
        crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
            .expect("abutopia bundle loads")
            .markets
            .clone()
    }

    /// A standalone, valid producer spec on a fresh actor id (8_051) used by the
    /// validation tests, so they are independent of what abutopia authors.
    fn test_producer_spec() -> crate::base_world::ProducerSpec {
        crate::base_world::ProducerSpec {
            actor: 8_051,
            market: 9001,
            in_good: 2, // WOOD — supplied by extractor 8041 (out_good == 2)
            in_qty: 10,
            out_good: 4,
            out_qty: 10,
            qty: 10,
            min_price: 500,
            theta_bps: 8_000,
            batches_target: 2,
            opening_cash: 1_000_000,
        }
    }

    /// True iff seeding the mutated layer panics (the file's validation pattern is
    /// fail-loud `assert!`/`panic!`, not `Result`).
    fn seed_panics(mutate: impl FnOnce(&mut MarketLayer)) -> bool {
        let mut layer = abutopia_layer();
        mutate(&mut layer);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut world = unseeded_world();
            seed_from_markets_layer(&mut world, &layer);
        }))
        .is_err()
    }

    /// ProducerSpec 8031 seeds ALL FIVE pieces: ProductionPool (WOOD→TOOLS recipe),
    /// InputPool (max_price ZERO, interval 1, denormalized in/out quantities),
    /// ProducerPolicy, sell-side SupplyPool, and the opening-cash mint — and NO
    /// RawDeposit (a producer buys its input; it is not a faucet).
    #[test]
    fn producer_seed_creates_all_five_pieces() {
        use crate::economy::producers::{InputPools, ProducerPolicies, ProducerPolicy};
        use crate::economy::production::{ProductionPools, RawDeposits};
        use crate::economy::{AccountBook, GOOD_TOOLS, GOOD_WOOD};

        let world = seeded_world();
        let actor = EconomicActorId(8031);

        // 1) ProductionPool: Leontief recipe in→out.
        let prod = world.resource::<ProductionPools>().0[&actor].clone();
        assert_eq!(prod.recipe.inputs, vec![(GOOD_WOOD, Quantity(10))]);
        assert_eq!(prod.recipe.outputs, vec![(GOOD_TOOLS, Quantity(10))]);
        assert_eq!(prod.interval_ticks, 1);
        assert_eq!(prod.last_generated_tick, None);

        // 2) InputPool: derived demand for WOOD at the home market, bound undiscovered.
        let pool = world.resource::<InputPools>().0[&actor];
        assert_eq!(pool.actor, actor);
        assert_eq!(pool.market, MarketId(9001));
        assert_eq!(pool.good, GOOD_WOOD);
        assert_eq!(pool.in_qty, Quantity(10));
        assert_eq!(pool.out_qty, Quantity(10));
        assert_eq!(pool.out_good, GOOD_TOOLS);
        assert_eq!(pool.interval_ticks, 1);
        assert_eq!(pool.last_generated_tick, None);
        assert_eq!(pool.max_price, Money::ZERO, "bound discovered at runtime");

        // 3) ProducerPolicy from the authored theta/batches.
        assert_eq!(
            world.resource::<ProducerPolicies>().0[&actor],
            ProducerPolicy {
                theta_bps: 8_000,
                batches_target: 2,
            }
        );

        // 4) Sell-side SupplyPool, exactly like an extractor's.
        let sp = world.resource::<SupplyPools>().0[&actor];
        assert_eq!(sp.market, MarketId(9001));
        assert_eq!(sp.good, GOOD_TOOLS);
        assert_eq!(sp.offered_qty_per_tick, Quantity(10));
        assert_eq!(sp.min_price, Money(500));
        assert_eq!(sp.interval_ticks, 1);

        // 5) Opening cash minted (the demand-actor seed-mint pattern).
        assert_eq!(
            world.resource::<AccountBook>().account(actor).available,
            Money(1_000_000)
        );

        // NOT a faucet: no RawDeposit for the producer.
        assert!(
            !world.resource::<RawDeposits>().0.contains_key(&actor),
            "a producer buys its input — it must NOT get a RAW faucet"
        );
    }

    /// `in_good == GOOD_RAW` is rejected: RAW is structurally non-tradable, so a
    /// producer could never buy it on a market — the chain would be dead at seed.
    #[test]
    fn producer_validation_rejects_raw_input() {
        assert!(seed_panics(|layer| {
            let mut spec = test_producer_spec();
            spec.in_good = 5; // GOOD_RAW
            layer.producers.push(spec);
        }));
    }

    /// Non-positive quantities/prices, zero batches, and theta above 100% are all
    /// authoring errors that must fail the seed loudly.
    #[test]
    fn producer_validation_rejects_bad_numbers() {
        type SpecMutation = Box<dyn Fn(&mut crate::base_world::ProducerSpec)>;
        let cases: Vec<(&str, SpecMutation)> = vec![
            ("batches_target == 0", Box::new(|s| s.batches_target = 0)),
            ("theta_bps > 10_000", Box::new(|s| s.theta_bps = 10_001)),
            ("in_qty <= 0", Box::new(|s| s.in_qty = 0)),
            ("out_qty <= 0", Box::new(|s| s.out_qty = -1)),
            ("qty <= 0", Box::new(|s| s.qty = 0)),
            ("min_price <= 0", Box::new(|s| s.min_price = 0)),
            ("opening_cash < 0", Box::new(|s| s.opening_cash = -1)),
            ("unknown market", Box::new(|s| s.market = 9_999)),
        ];
        for (name, mutate_spec) in cases {
            assert!(
                seed_panics(|layer| {
                    let mut spec = test_producer_spec();
                    mutate_spec(&mut spec);
                    layer.producers.push(spec);
                }),
                "seed must reject producer with {name}"
            );
        }
    }

    /// A producer whose `in_good` no supplier/extractor/other-producer outputs is a
    /// dead chain from tick 0 — rejected at seed.
    #[test]
    fn producer_validation_requires_input_supply_path() {
        assert!(seed_panics(|layer| {
            let mut spec = test_producer_spec();
            spec.in_good = 3; // GOOD_IRON — nothing in abutopia outputs IRON
            layer.producers.push(spec);
        }));
    }

    /// The same actor in BOTH consumer (DemandSpec) and producer (ProducerSpec) roles
    /// is rejected: disjointness is the unchecked assumption behind the independent
    /// demand/input order-generation passes in systems.rs.
    #[test]
    fn seed_rejects_actor_in_both_consumer_and_producer_roles() {
        assert!(seed_panics(|layer| {
            let mut spec = test_producer_spec();
            spec.actor = 8002; // authored TOOLS consumer
            layer.producers.push(spec);
        }));
    }

    /// Re-Apply: ProducerPolicies are rebuilt from the LAYER on every boot, even when
    /// the economy state is hydrated from a snapshot (the capita_baseline / #83 lesson —
    /// config must not silently revert on restart).
    #[test]
    fn producer_policies_reapplied_over_persisted_state() {
        use crate::economy::persist::{apply_into_world, extract_from_world};
        use crate::economy::producers::ProducerPolicies;

        // Phase 1 — fresh seed from the authored layer.
        let mut layer = abutopia_layer();
        let mut world = unseeded_world();
        seed_from_markets_layer(&mut world, &layer);
        assert_eq!(
            world.resource::<ProducerPolicies>().0[&EconomicActorId(8031)].theta_bps,
            8_000,
            "fresh seed applies the authored theta"
        );
        let markets_after_first = world.resource::<Markets>().0.len();

        // Phase 2 — persistence round trip into a fresh world (the hydrate path),
        // then re-seed with a RETUNED layer: authored policy must win.
        let snap = extract_from_world(&world);
        let mut hydrated = unseeded_world();
        apply_into_world(&mut hydrated, &snap);
        layer.producers[0].theta_bps = 1_234;
        layer.producers[0].batches_target = 7;
        seed_from_markets_layer(&mut hydrated, &layer);

        let policy = hydrated.resource::<ProducerPolicies>().0[&EconomicActorId(8031)];
        assert_eq!(policy.theta_bps, 1_234, "re-applied from the retuned layer");
        assert_eq!(
            policy.batches_target, 7,
            "re-applied from the retuned layer"
        );
        assert_eq!(
            hydrated.resource::<Markets>().0.len(),
            markets_after_first,
            "state-seed stays idempotent on the hydrate path"
        );
    }

    /// Re-Apply: authored market anchors are geometry/config, not economic stock.
    /// A hydrated economy must pick up retuned map positions without deleting
    /// persisted balances, otherwise edited world data leaves the live backend on
    /// stale market nodes.
    #[test]
    fn market_sites_and_distances_reapplied_over_persisted_state() {
        use crate::economy::persist::{apply_into_world, extract_from_world};

        let mut layer = abutopia_layer();
        let mut world = unseeded_world();
        seed_from_markets_layer(&mut world, &layer);
        let snap = extract_from_world(&world);

        let mut hydrated = unseeded_world();
        apply_into_world(&mut hydrated, &snap);
        layer.markets[0].anchor = [40.0, 8.0];
        seed_from_markets_layer(&mut hydrated, &layer);

        let central = hydrated.resource::<Markets>().0[&MarketId(9001)].clone();
        assert_eq!(
            central.node_id,
            NodeId(4),
            "hydrated market node must follow the retuned authored anchor"
        );
        assert_eq!(
            hydrated.resource::<MarketDistances>().0[&(MarketId(9001), MarketId(9002))],
            32,
            "distances must be recomputed from re-applied market sites"
        );
    }

    /// REVIEW I1: after seed AND after re-apply, keys(ProducerPolicies) ==
    /// keys(InputPools) — enforced upfront, not lazily in the dividend path. Removing
    /// a producer from the layer over a persisted snapshot is a loud failure.
    #[test]
    fn seed_and_reapply_enforce_policy_pool_keyset_equality() {
        use crate::economy::persist::{apply_into_world, extract_from_world};
        use crate::economy::producers::{InputPools, ProducerPolicies};

        // After a fresh seed: key sets are equal and non-empty.
        let world = seeded_world();
        let policy_keys: Vec<EconomicActorId> = world
            .resource::<ProducerPolicies>()
            .0
            .keys()
            .copied()
            .collect();
        let pool_keys: Vec<EconomicActorId> =
            world.resource::<InputPools>().0.keys().copied().collect();
        assert_eq!(policy_keys, pool_keys, "seed: key sets equal");
        assert_eq!(
            policy_keys,
            vec![EconomicActorId(8031)],
            "the authored producer is present"
        );

        // After re-apply over persisted state with the producer REMOVED from the
        // layer: keys(ProducerPolicies)={} vs keys(InputPools)={8031} → loud panic.
        let snap = extract_from_world(&world);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut hydrated = unseeded_world();
            apply_into_world(&mut hydrated, &snap);
            let mut layer = abutopia_layer();
            layer.producers.clear();
            seed_from_markets_layer(&mut hydrated, &layer);
        }))
        .is_err();
        assert!(
            panicked,
            "re-apply must enforce keys(ProducerPolicies) == keys(InputPools) upfront"
        );
    }

    /// Opening `MarketGoodState` for the six authored (market,good) pairs matches
    /// the authored prices (`ewma_reference_price` and `last_settlement_price`).
    #[test]
    fn markets_layer_opening_prices_match_spec() {
        use crate::economy::{GoodId, MarketGoodKey, MarketGoods};
        let world = seeded_world();
        let goods = world.resource::<MarketGoods>();
        let cases: &[(u32, u16, i64)] = &[
            (9002, 4, 1000), // Market Square, TOOLS
            (9002, 1, 1000), // Market Square, FOOD
            (9004, 1, 1000), // Homes Quarter, FOOD
            // Producer chain: TOOLS at the producer's home market (the participation
            // bound's reference price — without it the input-order pass is ZeroPrice
            // at tick 1; 1000 equals macro_flow's prior fallback, so it is inert for
            // the pre-chain dynamics). WOOD authored cheap at the source (50) and at
            // the convergence-side sink (380 < bound 400, > 50+295 landed cost).
            (9001, 4, 1000), // Central Works, TOOLS
            (9003, 2, 50),   // Harbor Depot, WOOD (source)
            (9001, 2, 380),  // Central Works, WOOD (sink)
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
