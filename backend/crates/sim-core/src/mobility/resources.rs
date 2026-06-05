use crate::ids::{AgentId, ChunkCoord, VehicleId};
use crate::mobility::lod::FlowCell;
use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};

/// Monotonic simulation tick counter. Incremented by `tick_increment_system`.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tick(pub u64);

/// Cursor recording the last sim-month the population system advanced through.
/// Persisted in the mobility snapshot alongside `Tick`; the behaviour that reads
/// and writes it lives in the `population` module (which re-exports this type).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LastProcessedMonth(pub u64);

/// Entities marked dirty by advance systems this tick. Read & drained by
/// `MobilityWorld::tick_mobility` to build the per-tick delta.
#[derive(Resource, Debug, Default, Clone)]
pub struct DirtyAgents(pub HashSet<Entity>);

/// Companion to `DirtyAgents` for vehicle entities.
#[derive(Resource, Debug, Default, Clone)]
pub struct DirtyVehicles(pub HashSet<Entity>);

/// Per-chunk aggregate state for warm chunks.
#[derive(Resource, Debug, Default, Clone)]
pub struct FlowCells(pub HashMap<ChunkCoord, FlowCell>);

/// Per-chunk population: agents + vehicles + floor(flow_cell.population).
/// Rebuilt each tick by `track_chunk_populations_system`.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkPopulations(pub HashMap<ChunkCoord, u32>);

/// Derived view of chunk coords that are currently `Active` or `Hot` (i.e. the
/// chunks whose mobility entities tick at full fidelity). Rebuilt each tick
/// from the chunk-entity LOD marker components by
/// `refresh_simulated_chunks_system`. Read by the Advance/Output systems to
/// gate per-entity work. The chunk entities are the source of truth; this
/// resource is a per-tick cache so the gating check is an `O(1)` `HashSet`
/// lookup keyed on the entity's `Position`-derived chunk.
#[derive(Resource, Debug, Default, Clone)]
pub struct SimulatedChunks(pub HashSet<ChunkCoord>);

/// Derived view of chunk coords that are currently `Warm` — consumed by
/// `warm_chunk_flow_system` to drive flow-cell transfers. Rebuilt each tick
/// from chunk-entity markers by `refresh_simulated_chunks_system`.
#[derive(Resource, Debug, Default, Clone)]
pub struct WarmChunkCoords(pub HashSet<ChunkCoord>);

/// Per-tick scratchpad of LOD transitions observed from `ChunkLodChanged`
/// messages. Populated by `consume_chunk_lod_transitions_system` (after
/// `CoreSet::LodReclassify`), drained by `promote_warm_to_active_system` and
/// `demote_active_to_warm_system`. The chunk-entity markers + the
/// `ChunkLodChanged` event stream are the source of truth; this scratchpad
/// just lets the promote/demote systems read transitions without each
/// needing its own message cursor.
#[derive(Resource, Debug, Default, Clone)]
pub struct ChunkLodTransitions(
    pub  Vec<(
        ChunkCoord,
        crate::world::events::ChunkLod,
        crate::world::events::ChunkLod,
    )>,
);

/// Per-chunk reverse-index of agent entities, maintained incrementally by
/// `track_chunk_populations_system`. Lets `demote_active_to_warm_system`
/// despawn an entire chunk's residents in O(K) instead of scanning all
/// `N_agents` per transitioning chunk. `HashSet` so per-entity removal in
/// the incremental rebucketing path is O(1) instead of O(K).
#[derive(Resource, Debug, Default, Clone)]
pub struct AgentsByChunk(pub HashMap<ChunkCoord, HashSet<Entity>>);

/// Per-chunk reverse-index of vehicle entities; mirror of `AgentsByChunk`.
#[derive(Resource, Debug, Default, Clone)]
pub struct VehiclesByChunk(pub HashMap<ChunkCoord, HashSet<Entity>>);

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

/// Per-tick economic destination override for citizens, written by the economy
/// attribution system and read by `route_assignment_system`. Maps a citizen's
/// stable `AgentId` to the routing node it should walk its economic leg toward
/// this tick. Ephemeral: cleared and repopulated every tick; never persisted.
#[derive(bevy_ecs::prelude::Resource, Debug, Default, Clone)]
pub struct CitizenEconomicTargets(
    pub std::collections::BTreeMap<crate::ids::AgentId, crate::routing::NodeId>,
);

/// Per-entity record of which chunk that entity was bucketed into during
/// the previous run of `track_chunk_populations_system`. Lets the system
/// run incrementally via `Changed<Position>` — when an entity's Position
/// changes, we know which old bucket to remove from before inserting into
/// the new one.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousChunkByEntity(pub HashMap<Entity, ChunkCoord>);

/// Per-chunk FlowCell aggregate that was added to `ChunkPopulations` on
/// the previous tick. Subtracted at the start of each incremental tick
/// before the entity-count deltas are applied, then re-added after, so
/// FlowCell contributions don't double-count across ticks.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousFlowCellContrib(pub HashMap<ChunkCoord, u32>);

/// Per-tick scratchpad — filled by `tick_mobility` (via a temporary
/// indirection during the Phase 7c migration) so the publish path can
/// read the per-chunk deltas without re-running the tick. Drained at
/// the end of each tick.
#[derive(Resource, Debug, Default, Clone)]
pub struct PendingPerChunkDeltas(pub Vec<crate::mobility::MobilityChunkDelta>);

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RouteAssignmentStats {
    pub assigned: u64,
    pub skipped: u64,
    pub failed: u64,
}

/// World-derived coordinates for resolvable activities (e.g. round-trip
/// home/destination), populated at seed time from the loaded world's geometry.
#[derive(Resource, Debug, Default, Clone)]
pub struct ActivityWaypoints(pub HashMap<String, (f32, f32)>);
