use crate::ids::{AgentId, ChunkCoord, LinkId, RouteId, StopId, VehicleId};
use crate::mobility::lod::{FlowCell, MobilityActivity};
use crate::mobility::records::{RouteRecord, StopRecord};
use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};

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

/// Per-chunk activity state. Driven by `classify_activity_system` each tick.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkActivities(pub HashMap<ChunkCoord, MobilityActivity>);

/// Per-chunk cooldown counter — decremented each tick, set to 30 on transition.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkActivityCooldowns(pub HashMap<ChunkCoord, u8>);

/// Per-chunk aggregate state for warm chunks.
#[derive(Resource, Debug, Default, Clone)]
pub struct FlowCells(pub HashMap<ChunkCoord, FlowCell>);

/// Per-chunk count of connected clients currently subscribed.
/// Updated by the WS task on chunk_subscribe / chunk_unsubscribe / disconnect.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkSubscribers(pub HashMap<ChunkCoord, u8>);

/// Per-chunk population: agents + vehicles + floor(flow_cell.population).
/// Rebuilt each tick by `track_chunk_populations_system`.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkPopulations(pub HashMap<ChunkCoord, u32>);

/// Per-chunk reverse-index of agent entities, rebuilt each tick by
/// `track_chunk_populations_system`. Lets `demote_active_to_warm_system`
/// despawn an entire chunk's residents in O(K) instead of scanning all
/// `N_agents` per transitioning chunk.
#[derive(Resource, Debug, Default, Clone)]
pub struct AgentsByChunk(pub HashMap<ChunkCoord, Vec<Entity>>);

/// Per-chunk reverse-index of vehicle entities; mirror of `AgentsByChunk`.
#[derive(Resource, Debug, Default, Clone)]
pub struct VehiclesByChunk(pub HashMap<ChunkCoord, Vec<Entity>>);

/// Transient list of activity transitions for promote/demote systems.
/// Cleared at start of each tick by `classify_activity_system`.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkTransitions(pub Vec<(ChunkCoord, MobilityActivity, MobilityActivity)>);

/// Per-agent record of the chunk that agent was in at the END of the
/// previous tick. Used by `tick_mobility` to compute `left_*` lists in the
/// new per-chunk delta — an agent whose previous chunk differs from its
/// current chunk is "leaving" the previous chunk and "arriving" in the
/// current chunk.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousAgentChunks(pub HashMap<AgentId, ChunkCoord>);

/// Mirror of `PreviousAgentChunks` for vehicles.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousVehicleChunks(pub HashMap<VehicleId, ChunkCoord>);

/// Stable AgentId → Entity. Spec §3 — exposed as a Resource so systems can
/// do O(1) lookups inside Queries instead of scanning all agents.
/// Maintained in lockstep with `MobilityWorld.by_agent_id` by the spawn
/// helpers and the post-tick sync in `MobilityWorld::tick_mobility`.
#[derive(Resource, Debug, Default, Clone)]
pub struct AgentIdIndex(pub HashMap<AgentId, Entity>);

/// Mirror of `AgentIdIndex` for vehicle entities.
#[derive(Resource, Debug, Default, Clone)]
pub struct VehicleIdIndex(pub HashMap<VehicleId, Entity>);
