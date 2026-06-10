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

use crate::ids::{AgentId, VehicleId};
use crate::mobility::components::*;
use crate::mobility::records::*;
use crate::mobility::resources::*;

/// Install the mobility plugin into a shared `World` + `Schedule`. Inserts
/// every mobility resource and registers the per-tick schedule. Pre-Task-11
/// shape: this is a free function; Task 11 wraps it into a `MobilityPlugin`
/// implementing `SimPlugin`.
pub fn install_mobility(world: &mut World, schedule: &mut Schedule) {
    world.insert_resource(Tick(0));
    // Phase 8b T10: ensure routing resources exist even when RoutingPlugin
    // isn't installed (tests, in-memory seed worlds). The runtime installs
    // RoutingPlugin BEFORE install_mobility and we don't want to clobber its
    // populated resources — only insert defaults if absent.
    if !world.contains_resource::<crate::routing::Graph>() {
        world.insert_resource(crate::routing::Graph::default());
    }
    if !world.contains_resource::<crate::routing::TrafficRoutes>() {
        world.insert_resource(crate::routing::TrafficRoutes::default());
    }
    if !world.contains_resource::<crate::routing::WaitingAgents>() {
        world.insert_resource(crate::routing::WaitingAgents::default());
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
    // Lives in the mobility plugin so it always exists even when the economy is
    // absent; the attribution system only populates it.
    world.insert_resource(crate::mobility::resources::CitizenEconomicTargets::default());
    world.insert_resource(PreviousChunkByEntity::default());
    world.insert_resource(PreviousFlowCellContrib::default());
    world.insert_resource(PendingPerChunkDeltas::default());
    world.insert_resource(RouteAssignmentStats::default());
    world.insert_resource(crate::mobility::resources::ActivityWaypoints::default());
    world.insert_resource(crate::mobility::resources::LastProcessedMonth::default());

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
    crate::time::TimePlugin.install(&mut world, &mut schedule);
    crate::population::PopulationPlugin.install(&mut world, &mut schedule);
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
    format!("vehicle:{}", stable_index(&id.0) % 8)
}

fn activity_world_coord(world: &World, activity_id: &str) -> Option<(f32, f32)> {
    world
        .get_resource::<crate::mobility::resources::ActivityWaypoints>()
        .and_then(|waypoints| waypoints.0.get(activity_id).copied())
        .or_else(|| {
            crate::mobility_geometry::activity_geometry(activity_id).map(|geometry| geometry.coord)
        })
}

fn initial_agent_position(world: &World, state: &AgentMobilityState) -> (f32, f32) {
    match state {
        AgentMobilityState::AtActivity { activity_id } => activity_world_coord(world, activity_id)
            .unwrap_or_else(|| {
                panic!("spawn_agent_from_record: unknown activity_id {activity_id}")
            }),
        AgentMobilityState::InVehicle { vehicle_id, .. } => {
            world_coord_for_vehicle(world, vehicle_id).unwrap_or_else(|| {
                panic!(
                    "spawn_agent_from_record: vehicle {} must exist before in-vehicle agent spawn",
                    vehicle_id.0
                )
            })
        }
        other => {
            let graph = world.resource::<crate::routing::Graph>();
            crate::mobility::agent_world_coord(other, graph).unwrap_or_else(|| {
                panic!("spawn_agent_from_record: state {other:?} did not resolve through graph")
            })
        }
    }
}

/// Current monotonic mobility tick.
pub fn tick(world: &World) -> u64 {
    world.resource::<Tick>().0
}

pub fn canonical_edge_key(
    graph: &crate::routing::Graph,
    edge_id: crate::routing::EdgeId,
) -> String {
    let edge = graph.edge(edge_id);
    edge.legacy_id
        .clone()
        .unwrap_or_else(|| format!("edge:{}", edge.id.0))
}

pub fn edge_by_canonical_key(
    graph: &crate::routing::Graph,
    key: &str,
) -> Option<crate::routing::EdgeId> {
    if let Some(edge_id) = graph.edge_by_legacy(key)
        && canonical_edge_key(graph, edge_id) == key
    {
        return Some(edge_id);
    }

    let raw_id = key.strip_prefix("edge:")?.parse::<u32>().ok()?;
    if (raw_id as usize) >= graph.edge_count() {
        return None;
    }
    let edge_id = crate::routing::EdgeId(raw_id);
    (canonical_edge_key(graph, edge_id) == key).then_some(edge_id)
}

pub fn agent(world: &World, id: &AgentId) -> Option<AgentRecord> {
    let entity = *world.resource::<AgentIdIndex>().0.get(id)?;
    agent_record_from_entity(world, entity)
}

pub fn vehicle(world: &World, id: &VehicleId) -> Option<VehicleRecord> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(id)?;
    vehicle_record_from_entity(world, entity)
}

pub fn stop(world: &World, id: &str) -> Option<StopMobilityRecord> {
    let graph = world.resource::<crate::routing::Graph>();
    let node_id = graph.node_by_legacy(id)?;
    stop_record_for_node(world, node_id, id.to_string())
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
pub fn stops(world: &World) -> Vec<StopMobilityRecord> {
    let graph = world.resource::<crate::routing::Graph>();
    let mut out: Vec<StopMobilityRecord> = graph
        .nodes()
        .iter()
        .filter(|node| node.kind == crate::routing::NodeKind::TransitStop)
        .flat_map(|node| {
            graph
                .legacy_node_ids(node.id)
                .iter()
                .filter_map(move |id| stop_record_for_node(world, node.id, id.clone()))
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn stop_record_for_node(
    _world: &World,
    _node_id: crate::routing::NodeId,
    _stop_id: String,
) -> Option<StopMobilityRecord> {
    None
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

/// Bulk variant of `build_mobility_chunk_snapshot`: one O(agents + vehicles)
/// pass bucketing every entity by its chunk, instead of a full agent scan per
/// chunk. Returns an entry (possibly empty) for every requested coord —
/// identical output to N independent per-chunk builds. This is the per-tick
/// read-view path: the per-chunk scan was O(chunks × agents) and dominated
/// tick cost (2026-06-10 tick-cost design).
pub fn build_mobility_chunk_snapshots(
    world: &World,
    coords: &[crate::ids::ChunkCoord],
) -> HashMap<crate::ids::ChunkCoord, crate::mobility::MobilityChunkSnapshot> {
    let mut out: HashMap<crate::ids::ChunkCoord, crate::mobility::MobilityChunkSnapshot> = coords
        .iter()
        .map(|coord| {
            (
                *coord,
                crate::mobility::MobilityChunkSnapshot {
                    chunk: *coord,
                    agents: Vec::new(),
                    vehicles: Vec::new(),
                },
            )
        })
        .collect();
    for record in agents(world) {
        let Some((x, y)) = world_coord_for_agent(world, &record.id) else {
            continue;
        };
        if let Some(snap) = out.get_mut(&crate::mobility::chunk_of(x, y, 32)) {
            snap.agents.push(record);
        }
    }
    for record in vehicles(world) {
        let Some((x, y)) = world_coord_for_vehicle(world, &record.id) else {
            continue;
        };
        if let Some(snap) = out.get_mut(&crate::mobility::chunk_of(x, y, 32)) {
            snap.vehicles.push(record);
        }
    }
    out
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
pub fn chunk_subscriber_counts_snapshot(world: &World) -> HashMap<crate::ids::ChunkCoord, u8> {
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
    spawn_agent_from_record_with_position(world, record, None)
}

/// Spawn an agent entity at an already authoritative world coordinate.
/// Updates `AgentIdIndex`.
pub fn spawn_agent_from_record_at_position(
    world: &mut World,
    record: AgentRecord,
    position: (f32, f32),
) -> Entity {
    spawn_agent_from_record_with_position(world, record, Some(position))
}

fn spawn_agent_from_record_with_position(
    world: &mut World,
    record: AgentRecord,
    position: Option<(f32, f32)>,
) -> Entity {
    let AgentRecord {
        id: record_id,
        state,
        plan,
        plan_cursor,
        walk_speed_per_tick,
        birth_tick,
        active_route,
        sex,
        parent_id,
        cyclic,
        home_market,
        work_market,
    } = record;
    let id = record_id.clone();
    let sprite_key = compute_agent_sprite_key(&id);
    let (px, py) = position.unwrap_or_else(|| initial_agent_position(world, &state));

    // Assign the market binding from spawn position the first time only
    // (record carries 0 = unassigned at initial seed). On restore/birth the
    // record already carries real ids (>= 9001), which we PRESERVE — never
    // recompute from a since-moved position.
    let (home_market, work_market) = if home_market == 0 {
        let markets = crate::mobility::market_binding::markets_with_positions(world);
        crate::mobility::market_binding::assign_binding((px, py), &markets)
            .map(|b| (b.home_market, b.work_market))
            .unwrap_or((home_market, work_market))
    } else {
        (home_market, work_market)
    };

    let active_route = active_route.map(|route| ActiveRoute {
        destination: crate::routing::NodeId(route.destination_node),
        profile: route.profile,
        steps: route
            .steps
            .into_iter()
            .map(|step| RouteStep {
                edge_id: crate::routing::EdgeId(step.edge_id),
                mode: step.mode,
                canonical_edge_key: step.canonical_edge_key,
                length: step.length,
            })
            .collect(),
        cursor: route.cursor,
    });
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(record_id),
            AgentMobilityStateComponent(state),
            WalkPlan {
                stages: plan,
                cursor: plan_cursor,
                cyclic,
            },
            WalkSpeed(walk_speed_per_tick),
            crate::mobility::components::BirthTick(birth_tick),
            sex,
            crate::mobility::components::ParentId(parent_id),
            Position { x: px, y: py },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(sprite_key),
            crate::mobility::MarketBinding {
                home_market,
                work_market,
            },
        ))
        .id();
    if let Some(active_route) = active_route {
        world.entity_mut(entity).insert(active_route);
    }
    world.resource_mut::<AgentIdIndex>().0.insert(id, entity);
    entity
}

/// Re-resolve `MarketBinding` for every agent still carrying the unassigned
/// sentinel (`home_market == 0`), using the seeded `Markets` and each agent's
/// spawn `Position`.
///
/// The runtime constructors install the authoritative routing graph through
/// `apply_into_world` — which ALSO spawns the seeded agents — and the economy
/// markets only resolve to correct positions when seeded AGAINST that graph
/// (each `MarketSite` stores a `node_id`, and `markets_with_positions` reads
/// `graph.node(node_id)`; a market seeded against the pre-`apply` plugin graph
/// has a node_id that is stale once the snapshot graph replaces it). So the
/// economy must be seeded after `apply`, which means the seeded agents are
/// necessarily spawned before any market exists and freeze at `home_market = 0`.
/// This runs once, right after `seed_from_markets_layer`, applying the same
/// nearest-market rule as the spawn-time guard in `spawn_agent_from_record`.
///
/// Idempotent: agents already bound (`home_market >= 1`, e.g. restored from a
/// snapshot or born after seeding) are left untouched, so re-hydrating an
/// already-bound world is a no-op.
pub fn rebind_unassigned_market_agents(world: &mut World) {
    let markets = crate::mobility::market_binding::markets_with_positions(world);
    if markets.is_empty() {
        return;
    }
    let mut rebinds: Vec<(Entity, crate::mobility::MarketBinding)> = Vec::new();
    {
        let mut query =
            world.query_filtered::<(Entity, &crate::mobility::MarketBinding, &Position), With<AgentMarker>>();
        for (entity, binding, position) in query.iter(world) {
            if binding.home_market != 0 {
                continue;
            }
            if let Some(rebound) =
                crate::mobility::market_binding::assign_binding((position.x, position.y), &markets)
            {
                rebinds.push((entity, rebound));
            }
        }
    }
    for (entity, rebound) in rebinds {
        if let Some(mut binding) = world.get_mut::<crate::mobility::MarketBinding>(entity) {
            *binding = rebound;
        }
    }
}

/// Spawn a vehicle entity from a record. Updates `VehicleIdIndex`.
///
pub fn spawn_vehicle_from_record(world: &mut World, record: VehicleRecord) -> Entity {
    let id = record.id.clone();
    let sprite_key = compute_vehicle_sprite_key(&id);
    let route_id = world
        .resource::<crate::routing::TrafficRoutes>()
        .route_by_legacy(&record.route_id)
        .unwrap_or_else(|| panic!("unknown traffic route_id {}", record.route_id));
    let edge_index = record.link_index;
    let (px, py) = {
        let traffic_routes = world.resource::<crate::routing::TrafficRoutes>();
        let graph = world.resource::<crate::routing::Graph>();
        let rp = RoutePosition {
            route_id,
            edge_index,
            progress: record.progress,
            speed: record.speed_per_tick,
        };
        crate::mobility::vehicle_world_coord(&rp, traffic_routes, graph)
            .expect("vehicle route position must resolve through traffic routes")
    };
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(record.id),
            VehicleKindComponent(record.kind),
            RoutePosition {
                route_id,
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
    world.resource_mut::<VehicleIdIndex>().0.insert(id, entity);
    entity
}

fn agent_record_from_entity(world: &World, entity: Entity) -> Option<AgentRecord> {
    let stable = world.get::<StableAgentId>(entity)?;
    let state = world.get::<AgentMobilityStateComponent>(entity)?;
    let plan = world.get::<WalkPlan>(entity)?;
    let speed = world.get::<WalkSpeed>(entity)?;
    let birth_tick = world
        .get::<crate::mobility::components::BirthTick>(entity)
        .map(|b| b.0)
        .unwrap_or_else(|| {
            panic!(
                "agent_record_from_entity: agent {} is missing BirthTick",
                stable.0.0
            )
        });
    let sex = world
        .get::<crate::mobility::components::Sex>(entity)
        .copied()
        .unwrap_or_default();
    let active_route = world
        .get::<ActiveRoute>(entity)
        .map(|route| PersistedActiveRoute {
            destination_node: route.destination.0,
            profile: route.profile,
            cursor: route.cursor,
            steps: route
                .steps
                .iter()
                .map(|step| PersistedRouteStep {
                    edge_id: step.edge_id.0,
                    mode: step.mode,
                    canonical_edge_key: step.canonical_edge_key.clone(),
                    length: step.length,
                })
                .collect(),
        });
    let parent_id = world
        .get::<crate::mobility::components::ParentId>(entity)
        .and_then(|p| p.0.clone());
    let binding = world.get::<crate::mobility::MarketBinding>(entity);
    Some(AgentRecord {
        id: stable.0.clone(),
        state: state.0.clone(),
        plan: plan.stages.clone(),
        plan_cursor: plan.cursor,
        walk_speed_per_tick: speed.0,
        birth_tick,
        active_route,
        sex,
        parent_id,
        cyclic: plan.cyclic,
        home_market: binding.map(|b| b.home_market).unwrap_or(0),
        work_market: binding.map(|b| b.work_market).unwrap_or(0),
    })
}

fn vehicle_record_from_entity(world: &World, entity: Entity) -> Option<VehicleRecord> {
    let stable = world.get::<StableVehicleId>(entity)?;
    let kind = world.get::<VehicleKindComponent>(entity)?;
    let pos = world.get::<RoutePosition>(entity)?;
    let cap = world.get::<Capacity>(entity)?;
    let occ = world.get::<Occupants>(entity)?;
    let dwell = world.get::<DwellTicksRemaining>(entity)?;
    let route_id = legacy_route_id_for(world, pos.route_id);
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

fn legacy_route_id_for(world: &World, route_id: crate::routing::TrafficRouteId) -> String {
    let routes = world.resource::<crate::routing::TrafficRoutes>();
    if (route_id.0 as usize) < routes.count() {
        return routes.route(route_id).legacy_route_id.clone();
    }
    panic!("unknown traffic route_id {}", route_id.0)
}

fn resolve_link_polyline(
    world: &World,
    link_id: &str,
) -> Option<crate::mobility_geometry::LinkGeometry> {
    let graph = world.resource::<crate::routing::Graph>();
    edge_by_canonical_key(graph, link_id).map(|edge_id| crate::mobility_geometry::LinkGeometry {
        points: graph.edge(edge_id).polyline.clone(),
    })
}

pub fn world_coord_for_agent(world: &World, agent_id: &AgentId) -> Option<(f32, f32)> {
    let entity = *world.resource::<AgentIdIndex>().0.get(agent_id)?;
    // Materialized trader-agents carry an authoritative Position written by the
    // economy materialize bridge; their mobility state is only a benign DTO filler.
    if world
        .get::<crate::mobility::components::TraderAgent>(entity)
        .is_some()
    {
        let pos = world.get::<crate::mobility::components::Position>(entity)?;
        return Some((pos.x, pos.y));
    }
    let state = world.get::<AgentMobilityStateComponent>(entity)?;
    let graph = world.resource::<crate::routing::Graph>();
    match &state.0 {
        AgentMobilityState::AtActivity { activity_id } => activity_world_coord(world, activity_id),
        AgentMobilityState::InVehicle { vehicle_id, .. } => {
            world_coord_for_vehicle(world, vehicle_id)
        }
        other => crate::mobility::agent_world_coord(other, graph),
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
    let traffic_routes = world.resource::<crate::routing::TrafficRoutes>();
    let graph = world.resource::<crate::routing::Graph>();
    crate::mobility::vehicle_world_coord(pos, traffic_routes, graph)
}

pub fn direction_for_vehicle(
    world: &World,
    vehicle_id: &VehicleId,
) -> Option<abutown_protocol::DirectionDto> {
    let entity = *world.resource::<VehicleIdIndex>().0.get(vehicle_id)?;
    let pos = world.get::<RoutePosition>(entity)?;
    let traffic_routes = world.resource::<crate::routing::TrafficRoutes>();
    let graph = world.resource::<crate::routing::Graph>();
    if (pos.route_id.0 as usize) >= traffic_routes.count() {
        return None;
    }
    let route = traffic_routes.route(pos.route_id);
    let edge_id = *route.edges.get(pos.edge_index)?;
    let edge = graph.edge(edge_id);
    Some(crate::mobility_geometry::direction_at_progress_slice(
        &edge.polyline,
        pos.progress,
    ))
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
    let (cx, cy) = world_coord_for_agent(world, agent_id)?;
    let direction =
        direction_for_agent(world, agent_id).unwrap_or(abutown_protocol::DirectionDto::S);
    let sprite_key =
        sprite_key_for_agent(world, agent_id).unwrap_or_else(|| "pedestrian:0".to_string());
    let current_tick = tick(world);
    let birth_tick = world
        .get::<crate::mobility::components::BirthTick>(entity)
        .map(|b| b.0)
        .unwrap_or_else(|| panic!("agent_dto_for: agent {} is missing BirthTick", stable.0.0));
    let age_seconds = world
        .resource::<crate::time::SimClock>()
        .age_seconds(current_tick, birth_tick);
    Some(abutown_protocol::AgentMobilityDto {
        id: abutown_protocol::EntityId(stable.0.0.clone()),
        state: abutown_protocol::AgentMobilityStateDto::from(&state.0),
        plan_cursor: plan.cursor,
        world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
        direction,
        sprite_key,
        age_seconds,
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
    let (cx, cy) = world_coord_for_vehicle(world, vehicle_id)?;
    let direction =
        direction_for_vehicle(world, vehicle_id).unwrap_or(abutown_protocol::DirectionDto::S);
    let sprite_key =
        sprite_key_for_vehicle(world, vehicle_id).unwrap_or_else(|| "vehicle:0".to_string());
    let route_id_str = legacy_route_id_for(world, pos.route_id);
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
        MobilityActivity::Asleep => {
            e.insert(AsleepChunk);
        }
        MobilityActivity::Warm => {
            e.insert(WarmChunk);
        }
        MobilityActivity::Active => {
            e.insert(ActiveChunk);
        }
        MobilityActivity::Hot => {
            e.insert(HotChunk);
        }
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

pub fn seed_chunk_subscriber_count(world: &mut World, chunk: crate::ids::ChunkCoord, count: u8) {
    let entity = ensure_chunk_entity(world, chunk);
    if let Some(mut sub) = world
        .entity_mut(entity)
        .get_mut::<crate::world::components::ChunkSubscriberCount>()
    {
        sub.0 = count;
    }
}

#[cfg(test)]
mod market_binding_roundtrip_tests {
    use super::*;
    use crate::ids::AgentId;
    use crate::mobility::records::{AgentMobilityState, AgentRecord, PlanStage};

    /// Reuse the `empty_world_and_schedule` fixture (same pattern as
    /// `mod.rs::empty_world`). Seed a single activity waypoint so
    /// `AtActivity` resolves to a known coord without a graph lookup.
    #[test]
    fn market_binding_round_trips_through_spawn_and_extract() {
        let (mut world, _schedule) = empty_world_and_schedule();

        // Prime the activity waypoint so `initial_agent_position` can resolve it.
        world
            .resource_mut::<crate::mobility::resources::ActivityWaypoints>()
            .0
            .insert("activity:home".to_string(), (10.0, 20.0));

        let agent_id = AgentId("agent:binding:test".to_string());
        let mut record = AgentRecord::new(
            agent_id.clone(),
            AgentMobilityState::AtActivity {
                activity_id: "activity:home".to_string(),
            },
            vec![PlanStage::Activity {
                activity_id: "activity:home".to_string(),
            }],
            1.0,
        );
        record.home_market = 9001;
        record.work_market = 9002;

        let entity = spawn_agent_from_record(&mut world, record);

        // Verify the component was inserted on the entity.
        let binding = world
            .get::<crate::mobility::MarketBinding>(entity)
            .expect("MarketBinding component must be present after spawn");
        assert_eq!(binding.home_market, 9001);
        assert_eq!(binding.work_market, 9002);

        // Verify the round-trip via agent_record_from_entity.
        let extracted =
            agent_record_from_entity(&world, entity).expect("agent record must be extractable");
        assert_eq!(extracted.home_market, 9001);
        assert_eq!(extracted.work_market, 9002);
    }

    /// Build a world with both `Graph` + `Markets` seeded (via the economy
    /// seed_world fixture path) then validate:
    /// 1. An agent with `home_market == 0` gets assigned a real market id at spawn.
    /// 2. An agent with `home_market == 9003` keeps it (restore-safety).
    #[test]
    fn assign_on_unassigned_and_preserve_on_assigned() {
        use crate::routing::{Graph, Node, NodeId, NodeKind, NodeSpatialIndex};
        use crate::world::schedule::SimPlugin;

        // Build a world with the EconomyPlugin + mobility plugin, mirroring the
        // economy/tests/seed.rs fixture so both `Graph` and `Markets` exist.
        let mut world = World::new();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        use crate::world::plugin::CorePlugin;
        CorePlugin::default().install(&mut world, &mut schedule);
        crate::time::TimePlugin.install(&mut world, &mut schedule);
        crate::population::PopulationPlugin.install(&mut world, &mut schedule);
        install_mobility(&mut world, &mut schedule);
        crate::economy::EconomyPlugin.install(&mut world, &mut schedule);

        // Install graph nodes matching the abutopia market anchor positions.
        let nodes = vec![
            Node {
                id: NodeId(0),
                position: (2.0, 3.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (111.5, 64.51),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (16.0, 48.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(3),
                position: (208.0, 48.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ];
        world.insert_resource(NodeSpatialIndex::from_nodes(&nodes));
        world.insert_resource(Graph::new(nodes, vec![]));

        // Seed the economy (4 markets from the abutopia markets layer).
        let bundle =
            crate::base_world::BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
                .expect("abutopia bundle loads");
        crate::economy::seed_from_markets_layer(&mut world, &bundle.markets);

        // Prime an activity waypoint near node 0 so initial_agent_position resolves.
        world
            .resource_mut::<crate::mobility::resources::ActivityWaypoints>()
            .0
            .insert("activity:home".to_string(), (2.0, 3.0));

        // --- Test 1: Assign when unassigned (home_market == 0) ---
        let id1 = AgentId("agent:assign:unassigned".to_string());
        let rec1 = AgentRecord::new(
            id1.clone(),
            AgentMobilityState::AtActivity {
                activity_id: "activity:home".to_string(),
            },
            vec![PlanStage::Activity {
                activity_id: "activity:home".to_string(),
            }],
            1.0,
        );
        // home_market == 0 means unassigned — spawn should assign it.
        assert_eq!(rec1.home_market, 0);
        let entity1 = spawn_agent_from_record(&mut world, rec1);
        let binding1 = world
            .get::<crate::mobility::MarketBinding>(entity1)
            .expect("binding must be present after spawn");
        assert!(
            binding1.home_market >= 9001,
            "unassigned agent must be assigned a real home_market >= 9001, got {}",
            binding1.home_market
        );
        assert!(
            binding1.work_market >= 9001,
            "unassigned agent must be assigned a real work_market >= 9001, got {}",
            binding1.work_market
        );

        // --- Test 2: Preserve when already assigned (restore-safety) ---
        let id2 = AgentId("agent:assign:preserved".to_string());
        let mut rec2 = AgentRecord::new(
            id2.clone(),
            AgentMobilityState::AtActivity {
                activity_id: "activity:home".to_string(),
            },
            vec![PlanStage::Activity {
                activity_id: "activity:home".to_string(),
            }],
            1.0,
        );
        rec2.home_market = 9003;
        rec2.work_market = 9004;
        let entity2 = spawn_agent_from_record(&mut world, rec2);
        let binding2 = world
            .get::<crate::mobility::MarketBinding>(entity2)
            .expect("binding must be present after spawn");
        assert_eq!(
            binding2.home_market, 9003,
            "pre-assigned home_market must NOT be recomputed"
        );
        assert_eq!(
            binding2.work_market, 9004,
            "pre-assigned work_market must NOT be recomputed"
        );
    }
}

#[cfg(test)]
mod chunk_snapshot_bucketing_tests {
    use super::*;
    use crate::ids::AgentId;
    use crate::mobility::records::{AgentMobilityState, AgentRecord, PlanStage};

    /// Spawn an agent parked at a fixed world coordinate via an activity
    /// waypoint (no graph needed — same trick as the market-binding tests).
    fn spawn_at(world: &mut World, id: &str, pos: (f32, f32)) {
        let activity_id = format!("activity:{id}");
        world
            .resource_mut::<crate::mobility::resources::ActivityWaypoints>()
            .0
            .insert(activity_id.clone(), pos);
        spawn_agent_from_record(
            world,
            AgentRecord::new(
                AgentId(format!("agent:{id}")),
                AgentMobilityState::AtActivity {
                    activity_id: activity_id.clone(),
                },
                vec![PlanStage::Activity { activity_id }],
                1.0,
            ),
        );
    }

    /// The single-pass bulk builder must return exactly what N independent
    /// per-chunk builds return: same agents per chunk, and an entry (possibly
    /// empty) for every requested chunk.
    #[test]
    fn bulk_chunk_snapshots_match_per_chunk_builds() {
        let (mut world, _schedule) = empty_world_and_schedule();

        // Three chunks at chunk_size 32: (0,0), (1,0), (2,1); plus one
        // requested chunk with no agents at all.
        spawn_at(&mut world, "a", (5.0, 5.0)); // chunk (0,0)
        spawn_at(&mut world, "b", (10.0, 20.0)); // chunk (0,0)
        spawn_at(&mut world, "c", (40.0, 8.0)); // chunk (1,0)
        spawn_at(&mut world, "d", (70.0, 40.0)); // chunk (2,1)

        let coords = [
            crate::ids::ChunkCoord { x: 0, y: 0 },
            crate::ids::ChunkCoord { x: 1, y: 0 },
            crate::ids::ChunkCoord { x: 2, y: 1 },
            crate::ids::ChunkCoord { x: 9, y: 9 }, // empty
        ];

        let bulk = build_mobility_chunk_snapshots(&world, &coords);

        assert_eq!(bulk.len(), coords.len(), "one entry per requested chunk");
        for coord in &coords {
            let per_chunk = build_mobility_chunk_snapshot(&world, *coord);
            let bulk_snap = bulk
                .get(coord)
                .unwrap_or_else(|| panic!("bulk result missing requested chunk {coord:?}"));
            assert_eq!(
                bulk_snap, &per_chunk,
                "bulk snapshot for {coord:?} must equal the per-chunk build"
            );
        }
        // Sanity: the fixture actually distributes agents across chunks.
        assert_eq!(bulk[&coords[0]].agents.len(), 2);
        assert_eq!(bulk[&coords[1]].agents.len(), 1);
        assert_eq!(bulk[&coords[2]].agents.len(), 1);
        assert_eq!(bulk[&coords[3]].agents.len(), 0);
    }
}
