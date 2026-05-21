//! Free-function API over the mobility ECS world.
//!
//! Phase 8a Task 9 dissolved the `MobilityWorld` wrapper. The state that
//! used to live inside the wrapper now lives directly in the shared
//! `bevy_ecs::world::World` owned by `SimulationRuntime` (or by callers that
//! stand up their own World for tests/seed flows). The methods that used to
//! be on `MobilityWorld` are now free functions in this module that take
//! `&World` / `&mut World`.
//!
//! The `by_agent_id`/`by_vehicle_id` indexes that lived on the wrapper are
//! gone — they were always a redundant mirror of the `AgentIdIndex` /
//! `VehicleIdIndex` resources kept in lockstep by the spawn + tick paths.
//! Callers should use those resources for stable-id → entity lookups.

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;
use bevy_ecs::world::World;

use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};
use crate::mobility::components::*;
use crate::mobility::records::*;
use crate::mobility::resources::*;

/// Install the mobility plugin into a shared `World` + `Schedule`. Inserts
/// every mobility resource and registers the per-tick schedule. Pre-Task-11
/// shape: this is a free function; Task 11 wraps it into a `MobilityPlugin`
/// implementing `SimPlugin`.
pub fn install_mobility(world: &mut World, schedule: &mut Schedule) {
    world.insert_resource(Tick(0));
    world.insert_resource(Routes::default());
    world.insert_resource(Stops::default());
    world.insert_resource(LinkPolylines::default());
    world.insert_resource(LegacyTransitShim::default());
    // Phase 8b T10: ensure routing resources exist even when RoutingPlugin
    // isn't installed (tests, in-memory seed worlds). The runtime installs
    // RoutingPlugin BEFORE install_mobility and we don't want to clobber its
    // populated resources — only insert defaults if absent.
    if !world.contains_resource::<crate::routing::Graph>() {
        world.insert_resource(crate::routing::Graph::default());
    }
    if !world.contains_resource::<crate::routing::TransitLines>() {
        world.insert_resource(crate::routing::TransitLines::default());
    }
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(DirtyVehicles::default());
    world.insert_resource(FlowCells::default());
    world.insert_resource(ChunkPopulations::default());
    world.insert_resource(SimulatedChunks::default());
    world.insert_resource(WarmChunkCoords::default());
    world.insert_resource(ChunkLodTransitions::default());
    world.insert_resource(AgentsByChunk::default());
    world.insert_resource(VehiclesByChunk::default());
    world.insert_resource(PreviousAgentChunks::default());
    world.insert_resource(PreviousVehicleChunks::default());
    world.insert_resource(AgentIdIndex::default());
    world.insert_resource(VehicleIdIndex::default());
    world.insert_resource(PreviousChunkByEntity::default());
    world.insert_resource(PreviousFlowCellContrib::default());
    world.insert_resource(PendingPerChunkDeltas::default());

    crate::mobility::systems::install_systems(schedule);
}

/// Build a fresh `World` + `Schedule` pair pre-installed with `CorePlugin`
/// and the mobility plugin. Tests/seed callers that don't go through
/// `SimulationRuntime` use this to mint a usable world.
pub fn empty_world_and_schedule() -> (World, Schedule) {
    let mut world = World::new();
    let mut schedule = Schedule::default();
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    CorePlugin::default().install(&mut world, &mut schedule);
    install_mobility(&mut world, &mut schedule);
    (world, schedule)
}

fn stable_index(id: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish() as u32
}

fn compute_agent_sprite_key(id: &AgentId) -> String {
    format!("pedestrian:{}", stable_index(&id.0) % 16)
}

fn compute_vehicle_sprite_key(id: &VehicleId) -> String {
    format!("tram:{}", stable_index(&id.0) % 4)
}

/// Current monotonic mobility tick.
pub fn tick(world: &World) -> u64 {
    world.resource::<Tick>().0
}

pub fn agent(world: &World, id: &AgentId) -> Option<AgentRecord> {
    let entity = *world.resource::<AgentIdIndex>().0.get(id)?;
    agent_record_from_entity(world, entity)
}

pub fn vehicle(world: &World, id: &VehicleId) -> Option<VehicleRecord> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(id)?;
    vehicle_record_from_entity(world, entity)
}

pub fn stop(world: &World, id: &StopId) -> Option<StopRecord> {
    world.resource::<Stops>().0.get(id).cloned()
}

/// Sorted by id for deterministic output.
pub fn agents(world: &World) -> Vec<AgentRecord> {
    let mut out: Vec<AgentRecord> = world
        .resource::<AgentIdIndex>()
        .0
        .keys()
        .filter_map(|id| agent(world, id))
        .collect();
    out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    out
}

/// Sorted by id.
pub fn vehicles(world: &World) -> Vec<VehicleRecord> {
    let mut out: Vec<VehicleRecord> = world
        .resource::<VehicleIdIndex>()
        .0
        .keys()
        .filter_map(|id| vehicle(world, id))
        .collect();
    out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    out
}

/// Sorted by id.
pub fn stops(world: &World) -> Vec<StopRecord> {
    let mut out: Vec<StopRecord> = world.resource::<Stops>().0.values().cloned().collect();
    out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    out
}

pub fn routes(world: &World) -> &HashMap<RouteId, RouteRecord> {
    &world.resource::<Routes>().0
}

pub fn link_polyline(world: &World, link_id: &LinkId) -> Option<Vec<(f32, f32)>> {
    world
        .resource::<LinkPolylines>()
        .0
        .get(link_id)
        .cloned()
}

pub fn snapshot(world: &World) -> crate::mobility::MobilitySnapshot {
    crate::mobility::MobilitySnapshot {
        agents: agents(world),
        vehicles: vehicles(world),
        stops: stops(world),
    }
}

/// Collect agents + vehicles whose current position falls inside `chunk`.
///
/// Distinct from `crate::persistence::build_chunk_snapshot` which emits a
/// tile-payload `ChunkSnapshotDto` for persistence. This function emits a
/// transient `MobilityChunkSnapshot` consumed by the WS fan-out — they're
/// orthogonal despite the historical naming collision.
pub fn build_mobility_chunk_snapshot(
    world: &World,
    chunk: crate::ids::ChunkCoord,
) -> crate::mobility::MobilityChunkSnapshot {
    let chunk_agents = agents(world)
        .into_iter()
        .filter(|record| {
            world_coord_for_agent(world, &record.id)
                .map(|(x, y)| crate::mobility::chunk_of(x, y, 32) == chunk)
                .unwrap_or(false)
        })
        .collect();
    let chunk_vehicles = vehicles(world)
        .into_iter()
        .filter(|record| {
            world_coord_for_vehicle(world, &record.id)
                .map(|(x, y)| crate::mobility::chunk_of(x, y, 32) == chunk)
                .unwrap_or(false)
        })
        .collect();
    crate::mobility::MobilityChunkSnapshot {
        chunk,
        agents: chunk_agents,
        vehicles: chunk_vehicles,
    }
}

/// Ensure a chunk entity exists at `coord`. If `ChunksByCoord` already maps
/// the coord, return the existing entity. Otherwise spawn an empty-tiles
/// `Asleep` chunk entity — terrain hydrates later if/when the chunk is
/// loaded. Empty-tiles chunks are valid LOD/subscription targets: the LOD
/// reclassifier only reads `ChunkSubscriberCount` + `ChunkPopulations`, and
/// neither cares about tile contents.
fn ensure_chunk_entity(world: &mut World, coord: crate::ids::ChunkCoord) -> Entity {
    if let Some(entity) = world
        .resource::<crate::world::resources::ChunksByCoord>()
        .0
        .get(&coord)
        .copied()
    {
        return entity;
    }
    let chunk_size = world.resource::<crate::world::resources::ChunkSizeRes>().0;
    crate::world::systems::spawn_chunk_entity(
        world,
        coord,
        chunk_size,
        Vec::new(),
        0,
        crate::scheduler::ChunkActivity::Asleep,
    )
}

/// Apply a per-connection chunk-subscription delta: increment for each
/// chunk in `added`, saturating-decrement for each chunk in `removed`.
///
/// The subscriber count lives on the chunk entity as `ChunkSubscriberCount`
/// — chunk entities are the source of truth post-Phase-8a. If a coord in
/// `added` doesn't yet have a chunk entity (the common case for WS clients
/// subscribing to areas without terrain loaded), an empty-tiles chunk
/// entity is spawned via `ensure_chunk_entity` so the LOD reclassifier sees
/// the subscription.
pub fn apply_subscription_diff<'a, A, R>(world: &mut World, added: A, removed: R)
where
    A: IntoIterator<Item = &'a crate::ids::ChunkCoord>,
    R: IntoIterator<Item = &'a crate::ids::ChunkCoord>,
{
    for coord in added {
        let entity = ensure_chunk_entity(world, *coord);
        if let Some(mut sub) = world
            .entity_mut(entity)
            .get_mut::<crate::world::components::ChunkSubscriberCount>()
        {
            sub.0 = sub.0.saturating_add(1);
        }
    }
    for coord in removed {
        let Some(entity) = world
            .resource::<crate::world::resources::ChunksByCoord>()
            .0
            .get(coord)
            .copied()
        else {
            continue;
        };
        if let Some(mut sub) = world
            .entity_mut(entity)
            .get_mut::<crate::world::components::ChunkSubscriberCount>()
        {
            sub.0 = sub.0.saturating_sub(1);
        }
    }
}

/// Read-only accessor: current activity class of a chunk, or `None` if no
/// chunk entity is loaded for that coord. Source: chunk-entity LOD marker
/// components owned by the foundation (Phase 8a).
pub fn activity_for_chunk(
    world: &World,
    chunk: crate::ids::ChunkCoord,
) -> Option<crate::mobility::lod::MobilityActivity> {
    let entity = *world
        .resource::<crate::world::resources::ChunksByCoord>()
        .0
        .get(&chunk)?;
    use crate::mobility::lod::MobilityActivity;
    use crate::world::components::{ActiveChunk, AsleepChunk, HotChunk, WarmChunk};
    if world.get::<HotChunk>(entity).is_some() {
        Some(MobilityActivity::Hot)
    } else if world.get::<ActiveChunk>(entity).is_some() {
        Some(MobilityActivity::Active)
    } else if world.get::<WarmChunk>(entity).is_some() {
        Some(MobilityActivity::Warm)
    } else if world.get::<AsleepChunk>(entity).is_some() {
        Some(MobilityActivity::Asleep)
    } else {
        // Chunk entity exists but has no LOD marker (in-between state during
        // spawn) — caller treats as Asleep.
        Some(MobilityActivity::Asleep)
    }
}

/// Read-only accessor: aggregate flow-cell state for a chunk if present.
pub fn flow_cell_for_chunk(
    world: &World,
    chunk: crate::ids::ChunkCoord,
) -> Option<&crate::mobility::lod::FlowCell> {
    world.resource::<FlowCells>().0.get(&chunk)
}

/// Number of active WS subscribers for a chunk (0 if no chunk entity loaded).
pub fn chunk_subscriber_count(world: &World, chunk: crate::ids::ChunkCoord) -> u8 {
    let Some(entity) = world
        .resource::<crate::world::resources::ChunksByCoord>()
        .0
        .get(&chunk)
        .copied()
    else {
        return 0;
    };
    world
        .get::<crate::world::components::ChunkSubscriberCount>(entity)
        .map(|s| s.0)
        .unwrap_or(0)
}

/// Snapshot of all chunk subscriber counts for publication into RuntimeReadView.
/// Iterates `ChunksByCoord` and reads the `ChunkSubscriberCount` component on
/// each chunk entity.
pub fn chunk_subscriber_counts_snapshot(
    world: &World,
) -> HashMap<crate::ids::ChunkCoord, u8> {
    let by_coord = world.resource::<crate::world::resources::ChunksByCoord>();
    let mut out = HashMap::with_capacity(by_coord.0.len());
    for (coord, entity) in by_coord.0.iter() {
        let count = world
            .get::<crate::world::components::ChunkSubscriberCount>(*entity)
            .map(|s| s.0)
            .unwrap_or(0);
        if count > 0 {
            out.insert(*coord, count);
        }
    }
    out
}

/// Spawn an agent entity from a record. Updates `AgentIdIndex`.
pub fn spawn_agent_from_record(world: &mut World, record: AgentRecord) -> Entity {
    let id = record.id.clone();
    let sprite_key = compute_agent_sprite_key(&id);
    let (px, py) = {
        let routes = world.resource::<Routes>();
        let stops = world.resource::<Stops>();
        let link_polylines = world.resource::<LinkPolylines>();
        crate::mobility::agent_world_coord(&record.state, routes, stops, link_polylines)
            .unwrap_or((0.0, 0.0))
    };
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(record.id),
            AgentMobilityStateComponent(record.state),
            WalkPlan {
                stages: record.plan,
                cursor: record.plan_cursor,
            },
            WalkSpeed(record.walk_speed_per_tick),
            Position { x: px, y: py },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(sprite_key),
        ))
        .id();
    world
        .resource_mut::<AgentIdIndex>()
        .0
        .insert(id, entity);
    entity
}

/// Spawn a vehicle entity from a record. Updates `VehicleIdIndex`.
///
/// Phase 8b T10: `RoutePosition` is keyed by `LineId`. We resolve the
/// caller-supplied legacy `record.route_id` via two sources, in order:
/// (1) the runtime `TransitLines` (real graph-derived lines), then
/// (2) the `LegacyTransitShim` (populated by `add_route` for the legacy
/// seed/test path). If neither resolves, we lazily register the route in
/// the shim with an empty link list — the caller will then need to call
/// `add_route` before any system actually consumes the position. This
/// matches the legacy behavior of allowing vehicles to be spawned
/// independent of route registration order.
pub fn spawn_vehicle_from_record(world: &mut World, record: VehicleRecord) -> Entity {
    let id = record.id.clone();
    let sprite_key = compute_vehicle_sprite_key(&id);
    let line_id = resolve_or_register_line_id(world, &record.route_id);
    let edge_index = record.link_index;
    let (px, py) = {
        let shim = world.resource::<LegacyTransitShim>();
        let link_polylines = world.resource::<LinkPolylines>();
        let rp = RoutePosition {
            line_id,
            edge_index,
            progress: record.progress,
            speed: record.speed_per_tick,
        };
        crate::mobility::vehicle_world_coord(&rp, shim, link_polylines)
            .unwrap_or((0.0, 0.0))
    };
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(record.id),
            VehicleKindComponent(record.kind),
            RoutePosition {
                line_id,
                edge_index,
                progress: record.progress,
                speed: record.speed_per_tick,
            },
            Capacity(record.capacity),
            Occupants(record.occupants),
            DwellTicksRemaining(record.dwell_ticks_remaining),
            Position { x: px, y: py },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(sprite_key),
        ))
        .id();
    world
        .resource_mut::<VehicleIdIndex>()
        .0
        .insert(id, entity);
    entity
}

/// Resolve the `LineId` for a legacy `RouteId`. The shim is the source of
/// truth during the 8b transition — we always return the LineId that
/// `add_route` assigned. If the shim doesn't have it yet, we register it
/// (backfilling links from `Routes` if available).
///
/// Note: this deliberately does NOT prefer `TransitLines::line_by_legacy`
/// because the real graph-derived LineIds are unrelated to the order
/// `add_route` ran. Mixing them would cause `pos.line_id` to disagree with
/// `shim.line_id_for_route(stop.route_id)` in `boarding_alighting_system`.
fn resolve_or_register_line_id(world: &mut World, route_id: &RouteId) -> crate::routing::LineId {
    let shim_lookup = world
        .resource::<LegacyTransitShim>()
        .line_id_for_route(route_id);
    let needs_backfill = match shim_lookup {
        Some(id) => world
            .resource::<LegacyTransitShim>()
            .links_for(id)
            .map(|l| l.is_empty())
            .unwrap_or(true),
        None => true,
    };
    if needs_backfill {
        let links_from_routes: Option<Vec<LinkId>> = world
            .resource::<Routes>()
            .0
            .get(route_id)
            .map(|r| r.links.clone());
        if let Some(links) = links_from_routes {
            return world
                .resource_mut::<LegacyTransitShim>()
                .register_route(route_id, &links);
        }
    }
    if let Some(id) = shim_lookup {
        return id;
    }
    world
        .resource_mut::<LegacyTransitShim>()
        .register_route(route_id, &[])
}

pub fn add_stop(world: &mut World, stop: StopRecord) {
    world
        .resource_mut::<Stops>()
        .0
        .insert(stop.id.clone(), stop);
}

pub fn add_route(world: &mut World, route: RouteRecord) {
    // Phase 8b T10: also register in the legacy shim so RoutePosition's
    // LineId can be translated back to (RouteId, LinkId) for the
    // unmigrated legacy resources.
    world
        .resource_mut::<LegacyTransitShim>()
        .register_route(&route.id, &route.links);
    world
        .resource_mut::<Routes>()
        .0
        .insert(route.id.clone(), route);
}

pub fn set_link_polyline(world: &mut World, link_id: LinkId, points: Vec<(f32, f32)>) {
    world
        .resource_mut::<LinkPolylines>()
        .0
        .insert(link_id, points);
}

fn agent_record_from_entity(world: &World, entity: Entity) -> Option<AgentRecord> {
    let stable = world.get::<StableAgentId>(entity)?;
    let state = world.get::<AgentMobilityStateComponent>(entity)?;
    let plan = world.get::<WalkPlan>(entity)?;
    let speed = world.get::<WalkSpeed>(entity)?;
    Some(AgentRecord {
        id: stable.0.clone(),
        state: state.0.clone(),
        plan: plan.stages.clone(),
        plan_cursor: plan.cursor,
        walk_speed_per_tick: speed.0,
    })
}

fn vehicle_record_from_entity(world: &World, entity: Entity) -> Option<VehicleRecord> {
    let stable = world.get::<StableVehicleId>(entity)?;
    let kind = world.get::<VehicleKindComponent>(entity)?;
    let pos = world.get::<RoutePosition>(entity)?;
    let cap = world.get::<Capacity>(entity)?;
    let occ = world.get::<Occupants>(entity)?;
    let dwell = world.get::<DwellTicksRemaining>(entity)?;
    let route_id = legacy_route_id_for(world, pos.line_id);
    Some(VehicleRecord {
        id: stable.0.clone(),
        kind: kind.0,
        route_id,
        link_index: pos.edge_index,
        progress: pos.progress,
        speed_per_tick: pos.speed,
        capacity: cap.0,
        occupants: occ.0.clone(),
        dwell_ticks_remaining: dwell.0,
    })
}

/// Resolve the legacy `RouteId` for a `LineId`. Checks the shim first
/// (legacy seed path), then real TransitLines (runtime path). Returns
/// an empty placeholder if neither has a mapping — callers downstream
/// will see a route_id that doesn't exist in `Routes` and skip work.
fn legacy_route_id_for(world: &World, line_id: crate::routing::LineId) -> RouteId {
    if let Some(rid) = world
        .resource::<LegacyTransitShim>()
        .route_id_for_line(line_id)
    {
        return rid.clone();
    }
    let tl = world.resource::<crate::routing::TransitLines>();
    if (line_id.0 as usize) < tl.count() {
        let line = tl.line(line_id);
        if let Some(legacy) = &line.legacy_route_id {
            return RouteId(legacy.clone());
        }
        return RouteId(line.name.clone());
    }
    RouteId(String::new())
}

fn resolve_link_polyline(
    world: &World,
    link_id: &LinkId,
) -> Option<crate::mobility_geometry::LinkGeometry> {
    world
        .resource::<LinkPolylines>()
        .0
        .get(link_id)
        .map(|points| crate::mobility_geometry::LinkGeometry {
            points: points.clone(),
        })
}

pub fn world_coord_for_agent(world: &World, agent_id: &AgentId) -> Option<(f32, f32)> {
    use crate::mobility_geometry::activity_geometry;
    let entity = *world.resource::<AgentIdIndex>().0.get(agent_id)?;
    let state = world.get::<AgentMobilityStateComponent>(entity)?;
    let routes = world.resource::<Routes>();
    let stops = world.resource::<Stops>();
    let link_polylines = world.resource::<LinkPolylines>();
    match &state.0 {
        AgentMobilityState::AtActivity { activity_id } => {
            activity_geometry(activity_id).map(|g| g.coord)
        }
        AgentMobilityState::InVehicle { vehicle_id, .. } => {
            world_coord_for_vehicle(world, vehicle_id)
        }
        other => crate::mobility::agent_world_coord(other, routes, stops, link_polylines),
    }
}

pub fn direction_for_agent(
    world: &World,
    agent_id: &AgentId,
) -> Option<abutown_protocol::DirectionDto> {
    let entity = *world.resource::<AgentIdIndex>().0.get(agent_id)?;
    let state = world.get::<AgentMobilityStateComponent>(entity)?;
    match &state.0 {
        AgentMobilityState::Walking { link_id, progress } => {
            let geom = resolve_link_polyline(world, link_id)?;
            Some(geom.direction_at_progress(*progress))
        }
        AgentMobilityState::InVehicle { vehicle_id, .. } => {
            direction_for_vehicle(world, vehicle_id)
        }
        _ => Some(abutown_protocol::DirectionDto::S),
    }
}

pub fn world_coord_for_vehicle(world: &World, vehicle_id: &VehicleId) -> Option<(f32, f32)> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(vehicle_id)?;
    let pos = world.get::<RoutePosition>(entity)?;
    let shim = world.resource::<LegacyTransitShim>();
    let link_id = shim.link_for(pos.line_id, pos.edge_index)?;
    let geom = resolve_link_polyline(world, link_id)?;
    Some(geom.world_coord_at_progress(pos.progress))
}

pub fn direction_for_vehicle(
    world: &World,
    vehicle_id: &VehicleId,
) -> Option<abutown_protocol::DirectionDto> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(vehicle_id)?;
    let pos = world.get::<RoutePosition>(entity)?;
    let shim = world.resource::<LegacyTransitShim>();
    let link_id = shim.link_for(pos.line_id, pos.edge_index)?;
    let geom = resolve_link_polyline(world, link_id)?;
    Some(geom.direction_at_progress(pos.progress))
}

pub fn sprite_key_for_agent(world: &World, agent_id: &AgentId) -> Option<String> {
    let entity = *world.resource::<AgentIdIndex>().0.get(agent_id)?;
    world.get::<SpriteKey>(entity).map(|s| s.0.clone())
}

pub fn sprite_key_for_vehicle(world: &World, vehicle_id: &VehicleId) -> Option<String> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(vehicle_id)?;
    world.get::<SpriteKey>(entity).map(|s| s.0.clone())
}

pub fn agent_dto_for(
    world: &World,
    agent_id: &AgentId,
) -> Option<abutown_protocol::AgentMobilityDto> {
    let entity = *world.resource::<AgentIdIndex>().0.get(agent_id)?;
    let state = world.get::<AgentMobilityStateComponent>(entity)?;
    let plan = world.get::<WalkPlan>(entity)?;
    let stable = world.get::<StableAgentId>(entity)?;
    let (cx, cy) = world_coord_for_agent(world, agent_id).unwrap_or((0.0, 0.0));
    let direction =
        direction_for_agent(world, agent_id).unwrap_or(abutown_protocol::DirectionDto::S);
    let sprite_key =
        sprite_key_for_agent(world, agent_id).unwrap_or_else(|| "pedestrian:0".to_string());
    Some(abutown_protocol::AgentMobilityDto {
        id: abutown_protocol::EntityId(stable.0.0.clone()),
        state: abutown_protocol::AgentMobilityStateDto::from(&state.0),
        plan_cursor: plan.cursor,
        world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
        direction,
        sprite_key,
    })
}

pub fn vehicle_dto_for(
    world: &World,
    vehicle_id: &VehicleId,
) -> Option<abutown_protocol::VehicleMobilityDto> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(vehicle_id)?;
    let stable = world.get::<StableVehicleId>(entity)?;
    let kind = world.get::<VehicleKindComponent>(entity)?;
    let pos = world.get::<RoutePosition>(entity)?;
    let cap = world.get::<Capacity>(entity)?;
    let occ = world.get::<Occupants>(entity)?;
    let dwell = world.get::<DwellTicksRemaining>(entity)?;
    let (cx, cy) = world_coord_for_vehicle(world, vehicle_id).unwrap_or((0.0, 0.0));
    let direction =
        direction_for_vehicle(world, vehicle_id).unwrap_or(abutown_protocol::DirectionDto::S);
    let sprite_key =
        sprite_key_for_vehicle(world, vehicle_id).unwrap_or_else(|| "tram:0".to_string());
    let route_id_str = legacy_route_id_for(world, pos.line_id).0;
    Some(abutown_protocol::VehicleMobilityDto {
        id: abutown_protocol::EntityId(stable.0.0.clone()),
        kind: kind.0.into(),
        route_id: route_id_str,
        link_index: pos.edge_index,
        progress: pos.progress,
        capacity: cap.0,
        occupants: occ
            .0
            .iter()
            .map(|agent_id| abutown_protocol::EntityId(agent_id.0.clone()))
            .collect(),
        dwell_ticks_remaining: dwell.0,
        world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
        direction,
        sprite_key,
    })
}

/// Test-only helper: mark a wide range of chunks as `Active` so the LOD
/// activity filter does not skip them. Spawns chunk entities (if not yet
/// present) and forces their LOD marker to `ActiveChunk`. Also primes the
/// `SimulatedChunks` resource so the very first tick (before
/// `refresh_simulated_chunks_system` runs) already sees these as simulated.
pub fn force_all_chunks_active_for_test(world: &mut World) {
    use crate::ids::ChunkCoord;
    use crate::scheduler::ChunkActivity;
    use crate::world::components::{
        ActiveChunk, AsleepChunk, ChunkSubscriberCount, HotChunk, WarmChunk,
    };
    let chunks: Vec<ChunkCoord> = (-16..=32)
        .flat_map(|x: i32| (-16..=32).map(move |y| ChunkCoord { x, y }))
        .collect();
    for coord in &chunks {
        let entity = ensure_chunk_entity(world, *coord);
        let mut e = world.entity_mut(entity);
        e.remove::<AsleepChunk>();
        e.remove::<WarmChunk>();
        e.remove::<HotChunk>();
        e.insert(ActiveChunk);
        if let Some(mut sub) = e.get_mut::<ChunkSubscriberCount>() {
            sub.0 = sub.0.max(1);
        }
        // ChunkActivity tag the helper used was for the legacy resource
        // map; we no longer write it. Suppress unused warning.
        let _ = ChunkActivity::Active;
    }
    let mut sim = world.resource_mut::<SimulatedChunks>();
    sim.0.extend(chunks);
}

/// Tick the mobility schedule once and return the per-chunk delta map.
pub fn tick_mobility(
    world: &mut World,
    schedule: &mut Schedule,
) -> HashMap<crate::ids::ChunkCoord, crate::mobility::MobilityChunkDelta> {
    schedule.run(world);

    // Sync AgentIdIndex with newly-spawned agents (from promote_warm_to_active_system).
    let mut new_agents: Vec<(AgentId, Entity)> = Vec::new();
    {
        let mut q = world.query::<(Entity, &StableAgentId)>();
        let index = world.resource::<AgentIdIndex>();
        for (entity, stable) in q.iter(world) {
            if !index.0.contains_key(&stable.0) {
                new_agents.push((stable.0.clone(), entity));
            }
        }
    }
    {
        let mut index = world.resource_mut::<AgentIdIndex>();
        for (id, entity) in new_agents {
            index.0.insert(id, entity);
        }
    }

    // Remove despawned agents from the index.
    let agent_ids_to_remove: Vec<AgentId> = {
        let index = world.resource::<AgentIdIndex>();
        index
            .0
            .iter()
            .filter(|(_, entity)| world.get_entity(**entity).is_err())
            .map(|(id, _)| id.clone())
            .collect()
    };
    {
        let mut index = world.resource_mut::<AgentIdIndex>();
        for id in &agent_ids_to_remove {
            index.0.remove(id);
        }
    }

    // Same for vehicles — sync newly-spawned vehicles.
    let mut new_vehicles: Vec<(VehicleId, Entity)> = Vec::new();
    {
        let mut q = world.query::<(Entity, &StableVehicleId)>();
        let index = world.resource::<VehicleIdIndex>();
        for (entity, stable) in q.iter(world) {
            if !index.0.contains_key(&stable.0) {
                new_vehicles.push((stable.0.clone(), entity));
            }
        }
    }
    {
        let mut index = world.resource_mut::<VehicleIdIndex>();
        for (id, entity) in new_vehicles {
            index.0.insert(id, entity);
        }
    }

    // Remove despawned vehicles from the index.
    let vehicle_ids_to_remove: Vec<VehicleId> = {
        let index = world.resource::<VehicleIdIndex>();
        index
            .0
            .iter()
            .filter(|(_, entity)| world.get_entity(**entity).is_err())
            .map(|(id, _)| id.clone())
            .collect()
    };
    {
        let mut index = world.resource_mut::<VehicleIdIndex>();
        for id in &vehicle_ids_to_remove {
            index.0.remove(id);
        }
    }

    // Drain dirty sets populated by the Advance systems.
    let dirty_agents: Vec<Entity> = std::mem::take(&mut world.resource_mut::<DirtyAgents>().0)
        .into_iter()
        .collect();
    let dirty_vehicles: Vec<Entity> = std::mem::take(&mut world.resource_mut::<DirtyVehicles>().0)
        .into_iter()
        .collect();

    // Build (current chunk → changed records) for agents.
    let mut changed_by_chunk_agents: HashMap<crate::ids::ChunkCoord, Vec<AgentRecord>> =
        HashMap::new();
    let mut current_agent_chunks: HashMap<crate::ids::AgentId, crate::ids::ChunkCoord> =
        HashMap::new();
    for entity in &dirty_agents {
        if let Some(record) = agent_record_from_entity(world, *entity)
            && let Some((x, y)) = world_coord_for_agent(world, &record.id)
        {
            let chunk = crate::mobility::chunk_of(x, y, 32);
            current_agent_chunks.insert(record.id.clone(), chunk);
            changed_by_chunk_agents
                .entry(chunk)
                .or_default()
                .push(record);
        }
    }

    // Same for vehicles.
    let mut changed_by_chunk_vehicles: HashMap<crate::ids::ChunkCoord, Vec<VehicleRecord>> =
        HashMap::new();
    let mut current_vehicle_chunks: HashMap<crate::ids::VehicleId, crate::ids::ChunkCoord> =
        HashMap::new();
    for entity in &dirty_vehicles {
        if let Some(record) = vehicle_record_from_entity(world, *entity)
            && let Some((x, y)) = world_coord_for_vehicle(world, &record.id)
        {
            let chunk = crate::mobility::chunk_of(x, y, 32);
            current_vehicle_chunks.insert(record.id.clone(), chunk);
            changed_by_chunk_vehicles
                .entry(chunk)
                .or_default()
                .push(record);
        }
    }

    // Compute left_* by comparing current chunk vs PreviousAgentChunks.
    let mut left_by_chunk_agents: HashMap<crate::ids::ChunkCoord, Vec<crate::ids::AgentId>> =
        HashMap::new();
    {
        let prev = world.resource::<PreviousAgentChunks>();
        for (id, current_chunk) in &current_agent_chunks {
            if let Some(prev_chunk) = prev.0.get(id)
                && prev_chunk != current_chunk
            {
                left_by_chunk_agents
                    .entry(*prev_chunk)
                    .or_default()
                    .push(id.clone());
            }
        }
    }
    let mut left_by_chunk_vehicles: HashMap<crate::ids::ChunkCoord, Vec<crate::ids::VehicleId>> =
        HashMap::new();
    {
        let prev = world.resource::<PreviousVehicleChunks>();
        for (id, current_chunk) in &current_vehicle_chunks {
            if let Some(prev_chunk) = prev.0.get(id)
                && prev_chunk != current_chunk
            {
                left_by_chunk_vehicles
                    .entry(*prev_chunk)
                    .or_default()
                    .push(id.clone());
            }
        }
    }

    // Update PreviousAgentChunks + PreviousVehicleChunks for next tick.
    {
        let mut prev = world.resource_mut::<PreviousAgentChunks>();
        for (id, chunk) in &current_agent_chunks {
            prev.0.insert(id.clone(), *chunk);
        }
    }
    {
        let mut prev = world.resource_mut::<PreviousVehicleChunks>();
        for (id, chunk) in &current_vehicle_chunks {
            prev.0.insert(id.clone(), *chunk);
        }
    }

    // Assemble per-chunk delta map.
    let mut out: HashMap<crate::ids::ChunkCoord, crate::mobility::MobilityChunkDelta> =
        HashMap::new();
    for (chunk, agents) in changed_by_chunk_agents {
        out.entry(chunk)
            .or_insert_with(|| crate::mobility::MobilityChunkDelta {
                chunk,
                changed_agents: Vec::new(),
                changed_vehicles: Vec::new(),
                left_agents: Vec::new(),
                left_vehicles: Vec::new(),
            })
            .changed_agents = agents;
    }
    for (chunk, vehicles) in changed_by_chunk_vehicles {
        out.entry(chunk)
            .or_insert_with(|| crate::mobility::MobilityChunkDelta {
                chunk,
                changed_agents: Vec::new(),
                changed_vehicles: Vec::new(),
                left_agents: Vec::new(),
                left_vehicles: Vec::new(),
            })
            .changed_vehicles = vehicles;
    }
    for (chunk, ids) in left_by_chunk_agents {
        out.entry(chunk)
            .or_insert_with(|| crate::mobility::MobilityChunkDelta {
                chunk,
                changed_agents: Vec::new(),
                changed_vehicles: Vec::new(),
                left_agents: Vec::new(),
                left_vehicles: Vec::new(),
            })
            .left_agents = ids;
    }
    for (chunk, ids) in left_by_chunk_vehicles {
        out.entry(chunk)
            .or_insert_with(|| crate::mobility::MobilityChunkDelta {
                chunk,
                changed_agents: Vec::new(),
                changed_vehicles: Vec::new(),
                left_agents: Vec::new(),
                left_vehicles: Vec::new(),
            })
            .left_vehicles = ids;
    }

    out
}

// ---- Seed/test helpers usable from anywhere (LOD bootstrap relies on
// being able to seed flow-cell + activity state). ----

pub fn seed_flow_cell(
    world: &mut World,
    chunk: crate::ids::ChunkCoord,
    cell: crate::mobility::lod::FlowCell,
) {
    world.resource_mut::<FlowCells>().0.insert(chunk, cell);
}

pub fn seed_chunk_activity(
    world: &mut World,
    chunk: crate::ids::ChunkCoord,
    activity: crate::mobility::lod::MobilityActivity,
) {
    use crate::mobility::lod::MobilityActivity;
    use crate::world::components::{ActiveChunk, AsleepChunk, HotChunk, WarmChunk};
    let entity = ensure_chunk_entity(world, chunk);
    let mut e = world.entity_mut(entity);
    e.remove::<AsleepChunk>();
    e.remove::<WarmChunk>();
    e.remove::<ActiveChunk>();
    e.remove::<HotChunk>();
    match activity {
        MobilityActivity::Asleep => { e.insert(AsleepChunk); }
        MobilityActivity::Warm => { e.insert(WarmChunk); }
        MobilityActivity::Active => { e.insert(ActiveChunk); }
        MobilityActivity::Hot => { e.insert(HotChunk); }
    }
    // Keep the derived `SimulatedChunks` / `WarmChunkCoords` in lockstep so
    // tests that don't run the full schedule still observe the seed.
    let is_sim = matches!(activity, MobilityActivity::Active | MobilityActivity::Hot);
    let is_warm = matches!(activity, MobilityActivity::Warm);
    if is_sim {
        world.resource_mut::<SimulatedChunks>().0.insert(chunk);
    } else {
        world.resource_mut::<SimulatedChunks>().0.remove(&chunk);
    }
    if is_warm {
        world.resource_mut::<WarmChunkCoords>().0.insert(chunk);
    } else {
        world.resource_mut::<WarmChunkCoords>().0.remove(&chunk);
    }
}

pub fn seed_chunk_subscriber_count(
    world: &mut World,
    chunk: crate::ids::ChunkCoord,
    count: u8,
) {
    let entity = ensure_chunk_entity(world, chunk);
    if let Some(mut sub) = world
        .entity_mut(entity)
        .get_mut::<crate::world::components::ChunkSubscriberCount>()
    {
        sub.0 = count;
    }
}
