//! Render-only projection of the macro flow (#69): each accepted cross-market
//! `MacroFlow` edge becomes an in-transit `FlowShipment` that the materialize
//! system draws as a walking `TraderAgent`. Pure view — NO economic state, NOT
//! persisted (ephemeral, regenerated from the resumed flow on restart).

use bevy_ecs::prelude::*;
use std::collections::BTreeMap;

use crate::economy::{EconomyConfig, GoodId, MarketId, Quantity};

/// Reserved actor-id offset for shipment-traders so they never collide with
/// seeded economic actors (8001-8012) or the demo trader (8003).
pub const SHIPMENT_ACTOR_OFFSET: u64 = 1 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowShipment {
    pub id: u64,
    pub from_market: MarketId,
    pub to_market: MarketId,
    pub good: GoodId,
    pub qty: Quantity,
    pub start_tick: u64,
    pub travel_ticks: u64,
}

impl FlowShipment {
    /// Linear travel progress in [0,1] at `tick` (>= start_tick).
    pub fn progress(&self, tick: u64) -> f32 {
        let elapsed = tick.saturating_sub(self.start_tick);
        (elapsed as f32 / self.travel_ticks.max(1) as f32).clamp(0.0, 1.0)
    }
    /// True once the shipment has reached its destination.
    pub fn arrived(&self, tick: u64) -> bool {
        tick.saturating_sub(self.start_tick) >= self.travel_ticks
    }
}

/// Active in-transit shipments, keyed by id (deterministic counter).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct FlowShipments(pub BTreeMap<u64, FlowShipment>);

/// Monotone shipment-id counter. EPHEMERAL — NOT persisted (resets to 0 on
/// restore alongside the empty `FlowShipments`), unlike the persisted `NextOrderId`.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextShipmentId(pub u64);

impl NextShipmentId {
    #[allow(clippy::should_implement_trait)] // Not an Iterator; 'next' is an ID counter.
    pub fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

/// Visible travel time for a shipment over `dist` tiles, at the same tile/tick
/// speed the demo trader walks (so flow-traders pace identically). >= 1.
pub fn shipment_travel_ticks(dist: i64, config: &EconomyConfig) -> u64 {
    let speed = config.trader_tiles_per_tick.max(1);
    (dist.max(0) as u64).div_ceil(speed).max(1)
}

/// Drop shipments that have reached their destination by `tick`. Deterministic
/// (BTreeMap retain). Called once per tick by the materialize system.
pub fn expire_arrived(shipments: &mut FlowShipments, tick: u64) {
    shipments.0.retain(|_, s| !s.arrived(tick));
}
