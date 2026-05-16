use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};
use crate::ids::{LinkId, RouteId, StopId};
use crate::mobility::records::{RouteRecord, StopRecord};

/// Monotonic simulation tick counter. Incremented by `tick_increment_system`.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tick(pub u64);

/// Route table: keyed by RouteId, value is the full route definition.
#[derive(Resource, Debug, Default, Clone)]
pub struct Routes(pub HashMap<RouteId, RouteRecord>);

/// Stop table: keyed by StopId, value is the full stop definition.
#[derive(Resource, Debug, Default, Clone)]
pub struct Stops(pub HashMap<StopId, StopRecord>);

/// Per-link polyline geometry. Read by `compute_world_coord_system` and the
/// advance systems to compute distances.
#[derive(Resource, Debug, Default, Clone)]
pub struct LinkPolylines(pub HashMap<LinkId, Vec<(f32, f32)>>);

/// Entities marked dirty by advance systems this tick. Read & drained by
/// `MobilityWorld::tick_mobility` to build the per-tick delta.
#[derive(Resource, Debug, Default, Clone)]
pub struct DirtyAgents(pub HashSet<Entity>);

/// Companion to `DirtyAgents` for vehicle entities.
#[derive(Resource, Debug, Default, Clone)]
pub struct DirtyVehicles(pub HashSet<Entity>);
