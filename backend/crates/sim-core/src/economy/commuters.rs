//! Render-only projection of realized WAGES (twin of shoppers.rs / flow_shipments.rs): an
//! observed market that paid wages last tick spawns commuter trips that the
//! materialize system draws as pedestrians walking to the market — economically-real
//! labor income, realized as visible commuter flow (supply side). PURE VIEW — no
//! economic state, NOT persisted (ephemeral, regenerated on restart).

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::economy::{EconomyConfig, MarketId, Markets, WageTelemetry};
use crate::routing::NodeId;

/// Reserved actor-id offset for commuter-agents; distinct from shopper 2<<32.
pub const COMMUTER_ACTOR_OFFSET: u64 = 3 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommuterTrip {
    pub id: u64,
    pub market: MarketId,
    pub origin_node: NodeId,
    pub start_tick: u64,
    pub travel_ticks: u64,
}

impl CommuterTrip {
    pub fn progress(&self, tick: u64) -> f32 {
        let elapsed = tick.saturating_sub(self.start_tick);
        (elapsed as f32 / self.travel_ticks.max(1) as f32).clamp(0.0, 1.0)
    }
    pub fn arrived(&self, tick: u64) -> bool {
        tick.saturating_sub(self.start_tick) >= self.travel_ticks
    }
}

/// Active commuter trips, keyed by id.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CommuterTrips(pub BTreeMap<u64, CommuterTrip>);

/// Monotone id counter. EPHEMERAL — NOT persisted (resets to 0 on restore).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextCommuterId(pub u64);

impl NextCommuterId {
    #[allow(clippy::should_implement_trait)] // Not an Iterator; 'next' is an ID counter.
    pub fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

/// Visible walk time for a commuter over `dist` tiles, at the same tile/tick speed
/// the demo trader (and flow shipments) walk. Always `>= 1` so `progress` never
/// divides by zero and a zero-distance origin still takes one visible tick.
pub fn commuter_travel_ticks(dist: i64, config: &EconomyConfig) -> u64 {
    let speed = config.trader_tiles_per_tick.max(1);
    (dist.max(0) as u64).div_ceil(speed).max(1)
}

/// Pure capture core (no `World`): reconcile `CommuterTrips` against observed
/// markets' realized WAGES. For each observed market in `WageTelemetry` with
/// `wage > 0`, compute `target = min(wage / per_unit, cap)`,
/// count this market's current trips, and top up the shortfall with new trips
/// taking the first-N candidates from `origins`.
///
/// `origins(market_node)` MUST return candidate origin nodes that are already
/// SORTED deterministically (by `NodeId`), have the market node EXCLUDED, and have
/// a Walk route — paired with their Manhattan distance (tiles) to the market node.
///
/// Deterministic: BTreeMap/BTreeSet iteration + the sorted candidate order + the
/// monotone `NextCommuterId`. No RNG, no float. PURE VIEW — never touches
/// `AccountBook`/`InventoryBook`/`MarketGoods`.
#[allow(clippy::too_many_arguments)]
pub fn capture_commuter_trips(
    wage_telemetry: &WageTelemetry,
    observed: &BTreeSet<MarketId>,
    markets: &Markets,
    origins: impl Fn(NodeId) -> Vec<(NodeId, i64)>,
    config: &EconomyConfig,
    tick: u64,
    trips: &mut CommuterTrips,
    next: &mut NextCommuterId,
) {
    let per_unit = config.commuters_per_wage_unit.max(1);
    for (market, wage) in wage_telemetry.0.iter() {
        if !observed.contains(market) {
            continue;
        }
        if wage.0 <= 0 {
            continue;
        }
        let Some(site) = markets.0.get(market) else {
            continue;
        };
        let target = (wage.0 / per_unit).clamp(0, config.max_commuters_per_market as i64) as usize;
        let current = trips.0.values().filter(|t| t.market == *market).count();
        if current >= target {
            continue;
        }
        let shortfall = target - current;
        for (origin_node, dist) in origins(site.node_id).into_iter().take(shortfall) {
            let id = next.next();
            trips.0.insert(
                id,
                CommuterTrip {
                    id,
                    market: *market,
                    origin_node,
                    start_tick: tick,
                    travel_ticks: commuter_travel_ticks(dist, config),
                },
            );
        }
    }
}

/// Drop commuter trips that have arrived by `tick` AND whose agent is no longer
/// being rendered (so the ghost-free leave→despawn completes first). `rendering`
/// is the set of commuter ids still materialized.
pub fn expire_arrived_commuters(
    trips: &mut CommuterTrips,
    tick: u64,
    rendering: &std::collections::BTreeSet<u64>,
) {
    trips
        .0
        .retain(|id, t| !t.arrived(tick) || rendering.contains(id));
}
