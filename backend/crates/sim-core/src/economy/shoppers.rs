//! Render-only projection of aggregate DEMAND (twin of flow_shipments.rs): an
//! observed market with unmet demand spawns shopper visits that the materialize
//! system draws as pedestrians walking to the market. PURE VIEW — no economic
//! state, NOT persisted (ephemeral, regenerated from resumed demand on restart).

use bevy_ecs::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::economy::{EconomyConfig, GoodId, MarketGoods, MarketId, Markets};
use crate::routing::NodeId;

/// Reserved actor-id offset for shopper-agents; distinct from flow-traders' 1<<32.
pub const SHOPPER_ACTOR_OFFSET: u64 = 2 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopperVisit {
    pub id: u64,
    pub market: MarketId,
    pub good: GoodId,
    pub origin_node: NodeId,
    pub start_tick: u64,
    pub travel_ticks: u64,
}

impl ShopperVisit {
    pub fn progress(&self, tick: u64) -> f32 {
        let elapsed = tick.saturating_sub(self.start_tick);
        (elapsed as f32 / self.travel_ticks.max(1) as f32).clamp(0.0, 1.0)
    }
    pub fn arrived(&self, tick: u64) -> bool {
        tick.saturating_sub(self.start_tick) >= self.travel_ticks
    }
}

/// Active shopper visits, keyed by id.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ShopperVisits(pub BTreeMap<u64, ShopperVisit>);

/// Monotone id counter. EPHEMERAL — NOT persisted (resets to 0 on restore).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextShopperId(pub u64);

impl NextShopperId {
    #[allow(clippy::should_implement_trait)] // Not an Iterator; 'next' is an ID counter.
    pub fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

/// Visible walk time for a shopper over `dist` tiles, at the same tile/tick speed
/// the demo trader (and flow shipments) walk. Always `>= 1` so `progress` never
/// divides by zero and a zero-distance origin still takes one visible tick.
pub fn shopper_travel_ticks(dist: i64, config: &EconomyConfig) -> u64 {
    let speed = config.trader_tiles_per_tick.max(1);
    (dist.max(0) as u64).div_ceil(speed).max(1)
}

/// Pure capture core (no `World`): reconcile `ShopperVisits` against observed
/// markets' unmet demand. For each observed `(market, good)` in `MarketGoods` with
/// `unmet_demand_last_tick > 0`, compute `target = min(unmet / per_unit, cap)`,
/// count this market's current visits, and top up the shortfall with new visits
/// taking the first-N candidates from `origins`.
///
/// `origins(market_node)` MUST return candidate origin nodes that are already
/// SORTED deterministically (by `NodeId`), have the market node EXCLUDED, and have
/// a Walk route — paired with their Manhattan distance (tiles) to the market node.
/// (Spatial-index plus routability live in the system wrapper, keeping this core
/// pure and unit-testable.) If fewer than the shortfall valid candidates exist,
/// fewer visits are spawned.
///
/// Deterministic: BTreeMap/BTreeSet iteration + the sorted candidate order + the
/// monotone `NextShopperId`. No RNG, no float. PURE VIEW — never touches
/// `AccountBook`/`InventoryBook`/`MarketGoods` (reads `unmet_demand_last_tick` only).
#[allow(clippy::too_many_arguments)]
pub fn capture_shopper_visits(
    market_goods: &MarketGoods,
    observed: &BTreeSet<MarketId>,
    markets: &Markets,
    origins: impl Fn(NodeId) -> Vec<(NodeId, i64)>,
    config: &EconomyConfig,
    tick: u64,
    visits: &mut ShopperVisits,
    next: &mut NextShopperId,
) {
    let per_unit = config.shoppers_per_unit.max(1);
    for (key, state) in market_goods.0.iter() {
        if !observed.contains(&key.market) {
            continue;
        }
        let unmet = state.unmet_demand_last_tick.0;
        if unmet <= 0 {
            continue;
        }
        let Some(site) = markets.0.get(&key.market) else {
            continue;
        };
        let target = (unmet / per_unit).clamp(0, config.max_shoppers_per_market as i64) as usize;
        let current = visits.0.values().filter(|v| v.market == key.market).count();
        if current >= target {
            continue;
        }
        let shortfall = target - current;
        for (origin_node, dist) in origins(site.node_id).into_iter().take(shortfall) {
            let id = next.next();
            visits.0.insert(
                id,
                ShopperVisit {
                    id,
                    market: key.market,
                    good: key.good,
                    origin_node,
                    start_tick: tick,
                    travel_ticks: shopper_travel_ticks(dist, config),
                },
            );
        }
    }
}

/// Drop shopper visits that have arrived by `tick` AND whose agent is no longer
/// being rendered (so the ghost-free leave→despawn completes first). `rendering`
/// is the set of shopper ids still materialized.
pub fn expire_arrived_shoppers(
    visits: &mut ShopperVisits,
    tick: u64,
    rendering: &std::collections::BTreeSet<u64>,
) {
    visits
        .0
        .retain(|id, v| !v.arrived(tick) || rendering.contains(id));
}
