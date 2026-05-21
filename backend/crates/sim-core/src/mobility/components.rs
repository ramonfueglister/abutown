use crate::ids::{AgentId, LinkId, VehicleId};
use crate::mobility::records::{AgentMobilityState, PlanStage, VehicleKind};
use abutown_protocol::DirectionDto;
use bevy_ecs::prelude::*;
use std::sync::Arc;

/// Marker component for pedestrian/agent entities.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentMarker;

/// Marker component for vehicles (cars + trams).
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

/// Per-tick walking distance in tile units. Stored on agents only.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct WalkSpeed(pub f32);

/// Vehicle class discriminator (car vs tram).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleKindComponent(pub VehicleKind);

/// Vehicle position along its current transit line edge. Written by
/// `vehicle_advance_system`. Phase 8b T10 migrated this from string-keyed
/// `RouteId` + `link_index` to integer-keyed `LineId` + `edge_index`. The
/// wire shape (`VehicleRecord`, `VehicleMobilityDto`) still uses the legacy
/// `route_id: String` + `link_index: usize`; conversion happens at the
/// emission boundary via `TransitLines::line(line_id).legacy_route_id`.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct RoutePosition {
    pub line_id: crate::routing::LineId,
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

/// Cached resolved polyline for the link this entity currently traverses.
/// Refreshed by `update_link_polyline_cache_system` (runs first in Advance)
/// when the entity's link changes. Eliminates the per-tick HashMap chain
/// (RouteId → RouteRecord → LinkId → polyline) in compute_world_coord /
/// compute_direction.
#[derive(Component, Debug, Clone)]
pub struct CurrentLinkPolyline {
    pub link_id: LinkId,
    pub points: Arc<Vec<(f32, f32)>>,
}
