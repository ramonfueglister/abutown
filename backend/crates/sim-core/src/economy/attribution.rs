//! Conservation-exact attribution of the macro's realized consumption/wages onto
//! observed, market-bound citizens. READ-ONLY over economy quantities: it mints
//! and moves NO money, so the `#78` tick audit is unaffected. It only SELECTS
//! which citizens are economically targeted this tick and proves the partition
//! identity `attributed + unobserved == realized`.

/// One market's attribution outcome for a single channel (shopping OR wages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAttribution {
    /// Citizens selected to represent the realized activity, in deterministic order.
    pub attributed: Vec<crate::ids::AgentId>,
    /// `attributed.len() as i64 * per_unit` — the quantity the visible citizens depict.
    pub attributed_amount: i64,
    /// `realized - attributed_amount` — the part no visible citizen depicts.
    /// `>= 0` whenever `realized >= 0` (the only case in practice: realized is a
    /// consumed quantity or wage, both non-negative).
    pub unobserved_amount: i64,
}

/// Select up to `min(realized / per_unit, cap, candidates.len())` citizens from
/// `candidates` (already sorted deterministically by the caller, e.g. by AgentId),
/// each representing `per_unit` units. Pure; no RNG.
///
/// `realized` is the macro's realized quantity (consumed goods, or wage Money),
/// always `>= 0` in practice; `cap` is expected small (a per-market absolute cap).
/// Guarantees `attributed_amount + unobserved_amount == realized` exactly.
pub fn attribute_channel(
    realized: i64,
    per_unit: i64,
    cap: usize,
    candidates: &[crate::ids::AgentId],
) -> ChannelAttribution {
    debug_assert!(realized >= 0, "realized quantity/wage must be non-negative");
    let per_unit = per_unit.max(1);
    // `try_from` instead of `as usize`: saturate rather than silently truncate a
    // large i64 on a 32-bit target (the value is clamped by cap/len below anyway).
    let by_magnitude = usize::try_from((realized / per_unit).max(0)).unwrap_or(usize::MAX);
    let count = by_magnitude.min(cap).min(candidates.len());
    let attributed: Vec<crate::ids::AgentId> = candidates.iter().take(count).cloned().collect();
    let attributed_amount = (count as i64) * per_unit;
    let unobserved_amount = realized - attributed_amount;
    ChannelAttribution {
        attributed,
        attributed_amount,
        unobserved_amount,
    }
}

use bevy_ecs::world::World;

/// Exclusive system (EconomySet::Attribution). Reads realized consumption
/// (`MarketGoods.consumed_qty_last_tick`, valid after Consume) and wages
/// (`WageTelemetry`, valid after PayWages); restricts to observed markets (those
/// whose market node is in an Active/Hot chunk — identical test to the former
/// capture systems); selects the attributed cohort from observed, bound citizens;
/// and writes their economic target node into `CitizenEconomicTargets`. READ-ONLY
/// over economy state — mints and moves NO money (the `#78` audit is unaffected).
pub fn run_citizen_attribution_system(world: &mut World) {
    use crate::economy::{EconomyConfig, MarketGoods, Markets, WageTelemetry};
    use crate::ids::ChunkCoord;
    use crate::mobility::resources::CitizenEconomicTargets;
    use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};
    use bevy_ecs::prelude::{Or, With};
    use std::collections::{BTreeMap, BTreeSet};

    if world.get_resource::<Markets>().is_none()
        || world.get_resource::<crate::routing::Graph>().is_none()
        || world.get_resource::<CitizenEconomicTargets>().is_none()
    {
        return;
    }

    // (1) Observed chunks — query borrow released after collect.
    let observed_chunks: BTreeSet<ChunkCoord> = {
        let mut q =
            world.query_filtered::<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>();
        q.iter(world).map(|c| c.0).collect()
    };

    // (2) Observed markets + target nodes + realized telemetry + config — immutable
    //     resource borrows, cloned into owned locals before release.
    let (observed_markets, market_nodes, consumed_by_market, wage_by_market, config) = {
        let graph = world.resource::<crate::routing::Graph>();
        let markets = world.resource::<Markets>();
        let observed_markets: BTreeSet<u32> = markets
            .0
            .iter()
            .filter(|(_, site)| {
                let pos = graph.node(site.node_id).position;
                observed_chunks.contains(&crate::mobility::chunk_of(pos.0, pos.1, 32))
            })
            .map(|(id, _)| id.0)
            .collect();
        let market_nodes: BTreeMap<u32, crate::routing::NodeId> = markets
            .0
            .iter()
            .map(|(id, site)| (id.0, site.node_id))
            .collect();
        let mut consumed_by_market: BTreeMap<u32, i64> = BTreeMap::new();
        for (key, st) in world.resource::<MarketGoods>().0.iter() {
            if observed_markets.contains(&key.market.0) {
                *consumed_by_market.entry(key.market.0).or_default() += st.consumed_qty_last_tick.0;
            }
        }
        let wage_by_market: BTreeMap<u32, i64> = world
            .resource::<WageTelemetry>()
            .0
            .iter()
            .filter(|(m, _)| observed_markets.contains(&m.0))
            .map(|(m, w)| (m.0, w.0))
            .collect();
        let config = *world.resource::<EconomyConfig>();
        (
            observed_markets,
            market_nodes,
            consumed_by_market,
            wage_by_market,
            config,
        )
    };

    if observed_markets.is_empty() {
        world.resource_mut::<CitizenEconomicTargets>().0.clear();
        return;
    }

    // (3) Candidate citizens per market — query borrow released after collect.
    let (shop_candidates, work_candidates) = {
        let mut shop: BTreeMap<u32, Vec<crate::ids::AgentId>> = BTreeMap::new();
        let mut work: BTreeMap<u32, Vec<crate::ids::AgentId>> = BTreeMap::new();
        let mut q = world.query_filtered::<(
            &crate::mobility::components::StableAgentId,
            &crate::mobility::MarketBinding,
        ), With<crate::mobility::components::AgentMarker>>();
        for (id, binding) in q.iter(world) {
            if observed_markets.contains(&binding.home_market) {
                shop.entry(binding.home_market)
                    .or_default()
                    .push(id.0.clone());
            }
            if observed_markets.contains(&binding.work_market) {
                work.entry(binding.work_market)
                    .or_default()
                    .push(id.0.clone());
            }
        }
        for v in shop.values_mut() {
            v.sort();
        }
        for v in work.values_mut() {
            v.sort();
        }
        (shop, work)
    };

    // (4) Compute targets — pure, no world borrow.
    let mut targets: BTreeMap<crate::ids::AgentId, crate::routing::NodeId> = BTreeMap::new();
    for (market_id, realized) in consumed_by_market {
        let Some(&node) = market_nodes.get(&market_id) else {
            continue;
        };
        let cands = shop_candidates.get(&market_id).cloned().unwrap_or_default();
        let res = attribute_channel(
            realized,
            config.shoppers_per_unit,
            config.max_shoppers_per_market,
            &cands,
        );
        for id in res.attributed {
            targets.insert(id, node);
        }
    }
    for (market_id, realized) in wage_by_market {
        let Some(&node) = market_nodes.get(&market_id) else {
            continue;
        };
        let cands = work_candidates.get(&market_id).cloned().unwrap_or_default();
        let res = attribute_channel(
            realized,
            config.commuters_per_wage_unit,
            config.max_commuters_per_market,
            &cands,
        );
        for id in res.attributed {
            targets.entry(id).or_insert(node);
        }
    }

    // (5) Write.
    world.resource_mut::<CitizenEconomicTargets>().0 = targets;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::AgentId;

    fn ids(n: usize) -> Vec<AgentId> {
        (0..n).map(|i| AgentId(format!("agent:walk:{i}"))).collect()
    }

    #[test]
    fn count_is_min_of_magnitude_cap_and_candidates() {
        // realized 9, per_unit 3 → magnitude 3; cap 4; candidates 10 → count 3.
        let c = attribute_channel(9, 3, 4, &ids(10));
        assert_eq!(c.attributed.len(), 3);
        assert_eq!(c.attributed_amount, 9);
        assert_eq!(c.unobserved_amount, 0);
    }

    #[test]
    fn cap_bounds_the_cohort_and_leaves_unobserved_remainder() {
        // realized 100, per_unit 3 → magnitude 33; cap 4 → count 4; 4*3=12 attributed.
        let c = attribute_channel(100, 3, 4, &ids(10));
        assert_eq!(
            c.attributed.len(),
            4,
            "absolute cap, never scales with population"
        );
        assert_eq!(c.attributed_amount, 12);
        assert_eq!(c.unobserved_amount, 88);
        assert_eq!(
            c.attributed_amount + c.unobserved_amount,
            100,
            "conservation identity"
        );
    }

    #[test]
    fn fewer_candidates_than_magnitude_caps_at_candidates() {
        // realized 9, per_unit 3 → magnitude 3, but only 2 observed citizens bound here.
        let c = attribute_channel(9, 3, 4, &ids(2));
        assert_eq!(c.attributed.len(), 2);
        assert_eq!(c.attributed_amount, 6);
        assert_eq!(c.unobserved_amount, 3);
        assert_eq!(c.attributed_amount + c.unobserved_amount, 9);
    }

    #[test]
    fn zero_realized_attributes_nobody() {
        let c = attribute_channel(0, 3, 4, &ids(10));
        assert!(c.attributed.is_empty());
        assert_eq!(c.attributed_amount, 0);
        assert_eq!(c.unobserved_amount, 0);
    }

    #[test]
    fn zero_per_unit_is_treated_as_one() {
        // per_unit <= 0 is clamped to 1 → count = min(6/1, 4, 10) = 4; identity holds.
        let c = attribute_channel(6, 0, 4, &ids(10));
        assert_eq!(c.attributed.len(), 4);
        assert_eq!(c.attributed_amount, 4);
        assert_eq!(c.attributed_amount + c.unobserved_amount, 6);
    }

    #[test]
    fn selection_is_deterministic_prefix() {
        let c = attribute_channel(9, 3, 4, &ids(10));
        assert_eq!(
            c.attributed,
            vec![
                AgentId("agent:walk:0".into()),
                AgentId("agent:walk:1".into()),
                AgentId("agent:walk:2".into())
            ],
        );
    }

    // ---- exclusive-system test -------------------------------------------------

    use crate::economy::{
        EconomyConfig, GoodId, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite,
        Markets, Quantity, WageTelemetry,
    };
    use crate::mobility::MarketBinding;
    use crate::mobility::components::{AgentMarker, StableAgentId};
    use crate::mobility::resources::CitizenEconomicTargets;
    use crate::routing::{Graph, Node, NodeId, NodeKind};
    use crate::world::components::{ActiveChunk, ChunkCoordComp};
    use bevy_ecs::world::World;

    /// Mirrors `economy/tests/materialize.rs::routed_shipment_world`'s observed-chunk
    /// setup: one market node at (1,1) inside chunk (0,0), with chunk (0,0) marked
    /// Active. Trimmed to exactly the resources `run_citizen_attribution_system`
    /// reads (no HpaIndex / spatial — those are the capture systems' concern).
    #[test]
    fn system_attributes_min_of_magnitude_cap_and_candidates() {
        let mut world = World::new();

        // Routing graph: a single market node at (1,1) — inside chunk (0,0).
        let graph = Graph::new(
            vec![Node {
                id: NodeId(0),
                position: (1.0, 1.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            }],
            vec![],
        );
        world.insert_resource(graph);

        // One market anchored to that node.
        let market_id = MarketId(9001);
        let mut markets = Markets::default();
        markets.0.insert(
            market_id,
            MarketSite {
                id: market_id,
                node_id: NodeId(0),
                name: "M".to_string(),
            },
        );
        world.insert_resource(markets);

        // Realized consumption of 9 units at (market, good).
        let mut goods = MarketGoods::default();
        let key = MarketGoodKey {
            market: market_id,
            good: GoodId(0),
        };
        let mut state = MarketGoodState::new(key);
        state.consumed_qty_last_tick = Quantity(9);
        goods.0.insert(key, state);
        world.insert_resource(goods);

        world.insert_resource(EconomyConfig::default());
        world.insert_resource(WageTelemetry::default());
        world.insert_resource(CitizenEconomicTargets::default());

        // Chunk (0,0) is observed (Active). The market node sits inside it.
        world.spawn((
            ChunkCoordComp(crate::ids::ChunkCoord { x: 0, y: 0 }),
            ActiveChunk,
        ));

        // 5 citizens, all bound (home == work == this market).
        for i in 0..5 {
            world.spawn((
                AgentMarker,
                StableAgentId(AgentId(format!("agent:walk:{i}"))),
                MarketBinding {
                    home_market: market_id.0,
                    work_market: market_id.0,
                },
            ));
        }

        run_citizen_attribution_system(&mut world);

        // min(9/3, 4, 5) = 3 attributed, each mapped to the market's node.
        let targets = &world.resource::<CitizenEconomicTargets>().0;
        assert_eq!(
            targets.len(),
            3,
            "min(magnitude 3, cap 4, candidates 5) == 3"
        );
        for (_, node) in targets.iter() {
            assert_eq!(
                *node,
                NodeId(0),
                "each attributed citizen targets the market node"
            );
        }
    }
}
