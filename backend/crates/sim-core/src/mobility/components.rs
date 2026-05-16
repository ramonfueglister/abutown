use bevy_ecs::prelude::*;
use abutown_protocol::DirectionDto;
use crate::ids::{AgentId, RouteId, VehicleId};
use crate::mobility::records::{AgentMobilityState, PlanStage, VehicleKind};

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

/// Vehicle position along its current route link. Written by `vehicle_advance_system`.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct RoutePosition {
    pub route_id: RouteId,
    pub link_index: usize,
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

/// Sparse marker: present on any entity that mutated in the current tick.
/// Drained by `MobilityWorld::tick_mobility` for delta-building.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dirty;
