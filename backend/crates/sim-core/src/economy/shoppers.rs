//! Render-only projection of aggregate DEMAND (twin of flow_shipments.rs): an
//! observed market with unmet demand spawns shopper visits that the materialize
//! system draws as pedestrians walking to the market. PURE VIEW — no economic
//! state, NOT persisted (ephemeral, regenerated from resumed demand on restart).

use bevy_ecs::prelude::*;
use std::collections::BTreeMap;

use crate::economy::{GoodId, MarketId};
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
