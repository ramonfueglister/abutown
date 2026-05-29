use crate::ids::{AgentId, VehicleId};
use crate::mobility::records::{AgentMobilityState, PlanStage, VehicleKind};
use abutown_protocol::DirectionDto;
use bevy_ecs::prelude::*;
use std::sync::Arc;

/// Marker component for pedestrian/agent entities.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentMarker;

/// Marker component for vehicles.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleMarker;

/// Current tile-space coordinate. Written by `compute_world_coord_system` each tick.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

/// Sprite-facing direction. Written by `compute_direction_system` each tick.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Direction(pub DirectionDto);

/// Pre-computed sprite catalog key. Deterministic per stable id, set at spawn time.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpriteKey(pub String);

/// Persistence handle for an agent. Matches `AgentId` 1:1.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub struct StableAgentId(pub AgentId);

/// Persistence handle for a vehicle. Matches `VehicleId` 1:1.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub struct StableVehicleId(pub VehicleId);

/// Wraps the existing `AgentMobilityState` enum (Walking, WaitingAtStop,
/// Boarding, InVehicle, Alighting, AtActivity). Stored on agents only.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct AgentMobilityStateComponent(pub AgentMobilityState);

/// MATSim-style activity plan plus current step cursor. Stored on agents only.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct WalkPlan {
    pub stages: Vec<PlanStage>,
    pub cursor: usize,
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct ActiveRoute {
    pub destination: crate::routing::NodeId,
    pub profile: crate::routing::RoutingProfileKey,
    pub steps: Vec<RouteStep>,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteStep {
    pub edge_id: crate::routing::EdgeId,
    pub mode: crate::routing::ModeState,
    pub canonical_edge_key: String,
    pub length: f32,
}

/// Per-tick walking distance in tile units. Stored on agents only.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct WalkSpeed(pub f32);

/// Vehicle class discriminator.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleKindComponent(pub VehicleKind);

/// Vehicle position along its current road traffic route edge. The wire shape
/// still uses `route_id: String` + `link_index: usize`; conversion happens at
/// the emission boundary through `TrafficRoutes::route(route_id)`.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct RoutePosition {
    pub route_id: crate::routing::TrafficRouteId,
    pub edge_index: usize,
    pub progress: f32,
    pub speed: f32,
}

/// Maximum passenger count. Stored on vehicles only.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity(pub u16);

/// Current passenger list. Stored on vehicles only.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Occupants(pub Vec<AgentId>);

/// Remaining ticks at the current stop (counts down). Stored on vehicles only.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DwellTicksRemaining(pub u16);

/// Tagging an agent whose walking progress saturated to 1.0 this tick.
/// Only agents with this marker are visited by `stop_arrival_system`,
/// which avoids iterating all 100k agents. Added by `walk_advance_system`
/// on saturation, removed by `stop_arrival_system` after the state
/// transition completes.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearStop;

/// Simulation tick at which this agent was born (spawned). Age is derived from
/// it via `SimClock`. Durable: mirrors `AgentRecord.birth_tick`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BirthTick(pub u64);

/// Cached resolved polyline for the link this entity currently traverses.
/// Refreshed by `update_link_polyline_cache_system` (runs first in Advance)
/// when the entity's link changes. Eliminates the per-tick HashMap chain
/// route/link lookup in compute_world_coord / compute_direction.
#[derive(Component, Debug, Clone)]
pub struct CurrentLinkPolyline {
    pub link_id: String,
    pub points: Arc<Vec<(f32, f32)>>,
}
