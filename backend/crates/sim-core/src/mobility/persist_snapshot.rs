//! Persistable snapshot of the mobility ECS world.
//!
//! After Phase 8a Task 9 dissolved the `MobilityWorld` wrapper, persistence
//! goes through a dedicated serializable struct. `MobilityPersistSnapshot`
//! grew out of the previous `MobilityWorld` serde impl. The
//! `last_processed_month` cursor was added later as a required field (no serde
//! default), so the wire format is intentionally NOT compatible with
//! pre-cursor legacy snapshots.
//!
//! Use `extract_from_world` to pull a snapshot out of a live `World`, and
//! `apply_into_world` to hydrate a freshly-installed mobility World from a
//! snapshot read back from storage.
//!
//! Current JSON schema (the two leading fields are the persisted sim cursors):
//!
//! ```text
//! { tick, last_processed_month, agents, vehicles, stops, routes,
//!   link_polylines, flow_cells, chunk_activities }
//! ```

use std::collections::{HashMap, VecDeque};

use bevy_ecs::prelude::Resource;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, ChunkCoord, VehicleId};
use crate::mobility::lod::{FlowCell, MobilityActivity};
use crate::mobility::records::{
    AgentMobilityState, AgentRecord, PersistedActiveRoute, PersistedRouteStep, PlanStage,
    VehicleRecord,
};
use crate::mobility::resources::{FlowCells, Tick};
use crate::routing::{
    Edge, EdgeId, EdgeKind, Graph, ModeState, Node, NodeId, NodeKind, NodeSpatialIndex,
    RoutingProfile, RoutingProfileKey, TrafficRoute, TrafficRouteId, TrafficRoutes, WaitingAgents,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedStop {
    pub id: String,
    pub route_id: String,
    pub link_index: usize,
    pub progress: f32,
    pub waiting_agents: VecDeque<AgentId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedRoute {
    pub id: String,
    pub links: Vec<String>,
}

#[derive(Resource, Debug, Clone, Default)]
struct PersistedStopMetadata(HashMap<String, PersistedStop>);

#[derive(Resource, Debug, Clone, Default)]
struct PersistedLinkPolylineMetadata(HashMap<String, Vec<(f32, f32)>>);

/// Serializable snapshot of mobility-world state. `tick` and
/// `last_processed_month` are the two persisted simulation cursors; the rest is
/// agent/vehicle/graph state.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilityPersistSnapshot {
    pub tick: u64,
    pub last_processed_month: u64,
    pub agents: HashMap<AgentId, AgentRecord>,
    pub vehicles: HashMap<VehicleId, VehicleRecord>,
    pub stops: HashMap<String, PersistedStop>,
    pub routes: HashMap<String, PersistedRoute>,
    pub link_polylines: HashMap<String, Vec<(f32, f32)>>,
    pub flow_cells: HashMap<ChunkCoord, FlowCell>,
    pub chunk_activities: HashMap<ChunkCoord, MobilityActivity>,
}

impl Serialize for MobilityPersistSnapshot {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct WorldRepr<'a> {
            tick: u64,
            last_processed_month: u64,
            agents: &'a HashMap<AgentId, AgentRecord>,
            vehicles: &'a HashMap<VehicleId, VehicleRecord>,
            stops: &'a HashMap<String, PersistedStop>,
            routes: &'a HashMap<String, PersistedRoute>,
            link_polylines: &'a HashMap<String, Vec<(f32, f32)>>,
            flow_cells: Vec<(ChunkCoord, &'a FlowCell)>,
            chunk_activities: Vec<(ChunkCoord, MobilityActivity)>,
        }
        // Sort chunk-keyed entries — JSON output must round-trip byte-stably.
        let mut flow_cells: Vec<(ChunkCoord, &FlowCell)> =
            self.flow_cells.iter().map(|(k, v)| (*k, v)).collect();
        flow_cells.sort_unstable_by_key(|(c, _)| *c);
        let mut chunk_activities: Vec<(ChunkCoord, MobilityActivity)> = self
            .chunk_activities
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        chunk_activities.sort_unstable_by_key(|(c, _)| *c);

        WorldRepr {
            tick: self.tick,
            last_processed_month: self.last_processed_month,
            agents: &self.agents,
            vehicles: &self.vehicles,
            stops: &self.stops,
            routes: &self.routes,
            link_polylines: &self.link_polylines,
            flow_cells,
            chunk_activities,
        }
        .serialize(ser)
    }
}

impl<'de> Deserialize<'de> for MobilityPersistSnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WorldRepr {
            tick: u64,
            last_processed_month: u64,
            agents: HashMap<AgentId, AgentRecord>,
            vehicles: HashMap<VehicleId, VehicleRecord>,
            stops: HashMap<String, PersistedStop>,
            routes: HashMap<String, PersistedRoute>,
            link_polylines: HashMap<String, Vec<(f32, f32)>>,
            #[serde(default)]
            flow_cells: Vec<(ChunkCoord, FlowCell)>,
            #[serde(default)]
            chunk_activities: Vec<(ChunkCoord, MobilityActivity)>,
        }
        let repr = WorldRepr::deserialize(de)?;
        Ok(Self {
            tick: repr.tick,
            last_processed_month: repr.last_processed_month,
            agents: repr.agents,
            vehicles: repr.vehicles,
            stops: repr.stops,
            routes: repr.routes,
            link_polylines: repr.link_polylines,
            flow_cells: repr.flow_cells.into_iter().collect(),
            chunk_activities: repr.chunk_activities.into_iter().collect(),
        })
    }
}

/// Pull a persist snapshot out of a live mobility world. The world must have
/// already had `install_mobility` called on it (and `CorePlugin` for the
/// chunk-entity store).
///
/// `chunk_activities` is derived from chunk-entity LOD markers — the
/// foundation owns chunk entities, and per-chunk activity lives there
/// post-Phase-8a. Asleep chunks are omitted to match the legacy resource
/// retention semantics (the old `classify_activity_system` dropped Asleep
/// chunks that had no subscribers + no population). Empty-tiles chunk
/// entities spawned solely to track subscriptions count as Asleep here.
pub fn extract_from_world(world: &World) -> MobilityPersistSnapshot {
    use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk, WarmChunk};
    let agents_map: HashMap<AgentId, AgentRecord> = crate::mobility::api::agents(world)
        .into_iter()
        .map(|rec| (rec.id.clone(), rec))
        .collect();
    let vehicles_map: HashMap<VehicleId, VehicleRecord> = crate::mobility::api::vehicles(world)
        .into_iter()
        .map(|rec| (rec.id.clone(), rec))
        .collect();
    let routes = graph_routes_for_persist(world);
    let stops = cached_or_graph_stops_for_persist(world);
    let link_polylines = cached_or_graph_link_polylines_for_persist(world, &routes, &agents_map);

    let chunk_activities: HashMap<ChunkCoord, MobilityActivity> = {
        let by_coord = world.resource::<crate::world::resources::ChunksByCoord>();
        let mut out = HashMap::new();
        for (coord, entity) in by_coord.0.iter() {
            // Re-check coord matches the entity's component to defend against
            // a stale `ChunksByCoord` entry (shouldn't happen but cheap to
            // guard).
            if let Some(c) = world.get::<ChunkCoordComp>(*entity)
                && c.0 == *coord
            {
                let activity = if world.get::<HotChunk>(*entity).is_some() {
                    MobilityActivity::Hot
                } else if world.get::<ActiveChunk>(*entity).is_some() {
                    MobilityActivity::Active
                } else if world.get::<WarmChunk>(*entity).is_some() {
                    MobilityActivity::Warm
                } else {
                    continue;
                };
                out.insert(*coord, activity);
            }
        }
        out
    };

    MobilityPersistSnapshot {
        tick: world.resource::<Tick>().0,
        last_processed_month: world
            .resource::<crate::mobility::resources::LastProcessedMonth>()
            .0,
        agents: agents_map,
        vehicles: vehicles_map,
        stops,
        routes,
        link_polylines,
        flow_cells: world.resource::<FlowCells>().0.clone(),
        chunk_activities,
    }
}

fn graph_routes_for_persist(world: &World) -> HashMap<String, PersistedRoute> {
    let graph = world.resource::<Graph>();
    let traffic_routes = world.resource::<TrafficRoutes>();
    traffic_routes
        .iter()
        .map(|route| {
            let route_id = route.legacy_route_id.clone();
            let links = route
                .edges
                .iter()
                .map(|edge_id| {
                    let edge = graph.edge(*edge_id);
                    edge.legacy_id
                        .clone()
                        .unwrap_or_else(|| format!("edge:{}", edge.id.0))
                })
                .collect();
            (
                route_id.clone(),
                PersistedRoute {
                    id: route_id,
                    links,
                },
            )
        })
        .collect()
}

fn cached_or_graph_stops_for_persist(world: &World) -> HashMap<String, PersistedStop> {
    if let Some(metadata) = world.get_resource::<PersistedStopMetadata>() {
        let graph = world.resource::<Graph>();
        let waiting = world.resource::<WaitingAgents>();
        return metadata
            .0
            .iter()
            .map(|(id, stop)| {
                let mut persisted = stop.clone();
                if let Some(node_id) = graph.node_by_legacy(id) {
                    persisted.waiting_agents = waiting
                        .queue(node_id)
                        .map(|queue| queue.iter().cloned().collect())
                        .unwrap_or_default();
                }
                (id.clone(), persisted)
            })
            .collect();
    }

    crate::mobility::api::stops(world)
        .into_iter()
        .map(|stop| {
            let persisted = PersistedStop {
                id: stop.id.clone(),
                route_id: stop.route_id,
                link_index: stop.link_index,
                progress: stop.progress,
                waiting_agents: stop.waiting_agents.into_iter().collect(),
            };
            (persisted.id.clone(), persisted)
        })
        .collect()
}

fn cached_or_graph_link_polylines_for_persist(
    world: &World,
    routes: &HashMap<String, PersistedRoute>,
    agents: &HashMap<AgentId, AgentRecord>,
) -> HashMap<String, Vec<(f32, f32)>> {
    if let Some(metadata) = world.get_resource::<PersistedLinkPolylineMetadata>() {
        let graph = world.resource::<Graph>();
        let mut out = metadata.0.clone();
        for link_id in active_route_canonical_keys_for_persist(agents) {
            out.entry(link_id.clone()).or_insert_with(|| {
                graph
                    .edge(expect_canonical_edge_key(graph, &link_id))
                    .polyline
                    .clone()
            });
        }
        return out;
    }

    let graph = world.resource::<Graph>();
    let mut link_ids = Vec::new();
    for route in routes.values() {
        link_ids.extend(route.links.iter().cloned());
    }
    for agent in agents.values() {
        if let AgentMobilityState::Walking { link_id, .. } = &agent.state {
            link_ids.push(link_id.clone());
        }
        for stage in &agent.plan {
            match stage {
                PlanStage::WalkToStop { link_id, .. }
                | PlanStage::WalkToActivity { link_id, .. } => link_ids.push(link_id.clone()),
                _ => {}
            }
        }
    }
    link_ids.sort();
    link_ids.dedup();

    let mut out: HashMap<String, Vec<(f32, f32)>> = link_ids
        .into_iter()
        .map(|link_id| {
            let edge_id = resolve_canonical_edge_key(graph, &link_id).unwrap_or_else(|| {
                panic!(
                    "extract_from_world: referenced link {} is missing from routing graph",
                    link_id
                )
            });
            (link_id, graph.edge(edge_id).polyline.clone())
        })
        .collect();
    for link_id in active_route_canonical_keys_for_persist(agents) {
        out.entry(link_id.clone()).or_insert_with(|| {
            graph
                .edge(expect_canonical_edge_key(graph, &link_id))
                .polyline
                .clone()
        });
    }
    for edge in graph.edges() {
        if edge.kind != EdgeKind::Footway {
            continue;
        }
        let Some(link_id) = &edge.legacy_id else {
            continue;
        };
        if link_id.starts_with("link:walk:") {
            out.entry(link_id.clone())
                .or_insert_with(|| edge.polyline.clone());
        }
    }
    out
}

fn active_route_canonical_keys_for_persist(
    agents: &HashMap<AgentId, AgentRecord>,
) -> impl Iterator<Item = String> + '_ {
    agents
        .values()
        .filter_map(|agent| agent.active_route.as_ref())
        .flat_map(|route| route.steps.iter())
        .filter(|step| !step.canonical_edge_key.is_empty())
        .map(|step| step.canonical_edge_key.clone())
}

fn existing_graph_polyline(world: &World, link_id: &str) -> Option<Vec<(f32, f32)>> {
    let graph = world.resource::<Graph>();
    resolve_canonical_edge_key(graph, link_id).map(|edge_id| graph.edge(edge_id).polyline.clone())
}

fn polyline_length(polyline: &[(f32, f32)]) -> f32 {
    polyline
        .windows(2)
        .map(|pair| {
            let dx = pair[1].0 - pair[0].0;
            let dy = pair[1].1 - pair[0].1;
            (dx * dx + dy * dy).sqrt()
        })
        .sum()
}

fn resolve_snapshot_polyline(
    world: &World,
    link_id: &str,
    polylines: &HashMap<String, Vec<(f32, f32)>>,
) -> Vec<(f32, f32)> {
    polylines
        .get(link_id)
        .cloned()
        .or_else(|| existing_graph_polyline(world, link_id))
        .unwrap_or_else(|| {
            panic!(
                "apply_into_world: no graph polyline available for persisted link {}",
                link_id
            )
        })
}

fn point_key(point: (f32, f32)) -> (u32, u32) {
    (point.0.to_bits(), point.1.to_bits())
}

fn node_for_polyline_point(
    nodes: &mut Vec<Node>,
    point_nodes: &mut HashMap<(u32, u32), NodeId>,
    position: (f32, f32),
) -> NodeId {
    if let Some(id) = point_nodes.get(&point_key(position)).copied() {
        return id;
    }
    let id = NodeId(nodes.len() as u32);
    nodes.push(Node {
        id,
        position,
        kind: NodeKind::Intersection,
        legacy_id: None,
    });
    point_nodes.insert(point_key(position), id);
    id
}

fn push_edge(
    nodes: &mut Vec<Node>,
    point_nodes: &mut HashMap<(u32, u32), NodeId>,
    edges: &mut Vec<Edge>,
    link_id: String,
    polyline: Vec<(f32, f32)>,
    kind: EdgeKind,
) -> EdgeId {
    assert!(
        polyline.len() >= 2,
        "apply_into_world: persisted link {} needs at least two points",
        link_id
    );

    let from = node_for_polyline_point(nodes, point_nodes, polyline[0]);
    let to = node_for_polyline_point(
        nodes,
        point_nodes,
        *polyline.last().expect("polyline length checked"),
    );

    let edge_id = EdgeId(edges.len() as u32);
    edges.push(Edge {
        id: edge_id,
        from,
        to,
        length: polyline_length(&polyline),
        polyline,
        kind,
        speed_limit: 1.0,
        capacity: 16,
        legacy_id: Some(link_id),
    });
    edge_id
}

fn push_reverse_footway_for_edge(edges: &mut Vec<Edge>, edge_id: EdgeId) {
    let edge = edges[edge_id.0 as usize].clone();
    if edge.kind != EdgeKind::Footway {
        return;
    }
    edges.push(Edge {
        id: EdgeId(edges.len() as u32),
        from: edge.to,
        to: edge.from,
        length: edge.length,
        polyline: edge.polyline.iter().rev().copied().collect(),
        kind: EdgeKind::Footway,
        speed_limit: edge.speed_limit,
        capacity: edge.capacity,
        legacy_id: None,
    });
}

fn edge_kind_for_mode(mode: ModeState) -> EdgeKind {
    match mode {
        ModeState::Walking => EdgeKind::Footway,
        ModeState::Driving => EdgeKind::Road,
        ModeState::OnTram => {
            panic!("apply_into_world: persisted active_route contains retired tram mode")
        }
    }
}

fn push_persisted_non_route_link(
    links: &mut HashMap<String, EdgeKind>,
    link_id: String,
    kind: EdgeKind,
) {
    if let Some(existing) = links.insert(link_id.clone(), kind) {
        assert_eq!(
            existing, kind,
            "apply_into_world: persisted link {} has conflicting edge kinds {:?} and {:?}",
            link_id, existing, kind
        );
    }
}

fn persisted_walking_links(snap: &MobilityPersistSnapshot) -> Vec<(String, EdgeKind)> {
    let mut links = HashMap::new();
    for agent in snap.agents.values() {
        if let AgentMobilityState::Walking { link_id, .. } = &agent.state {
            push_persisted_non_route_link(&mut links, link_id.clone(), EdgeKind::Footway);
        }
        for stage in &agent.plan {
            match stage {
                PlanStage::WalkToStop { link_id, .. }
                | PlanStage::WalkToActivity { link_id, .. } => {
                    push_persisted_non_route_link(&mut links, link_id.clone(), EdgeKind::Footway);
                }
                _ => {}
            }
        }
        if let Some(active_route) = &agent.active_route {
            for step in active_route
                .steps
                .iter()
                .filter(|step| !step.canonical_edge_key.is_empty())
            {
                push_persisted_non_route_link(
                    &mut links,
                    step.canonical_edge_key.clone(),
                    edge_kind_for_mode(step.mode),
                );
            }
        }
    }
    let mut links: Vec<_> = links.into_iter().collect();
    links.sort_by(|a, b| a.0.cmp(&b.0));
    links
}

fn install_snapshot_routing(world: &mut World, snap: &MobilityPersistSnapshot) {
    let mut nodes = Vec::new();
    let mut point_nodes = HashMap::new();
    let mut edges = Vec::new();
    let mut edge_by_link: HashMap<String, EdgeId> = HashMap::new();

    let mut routes: Vec<_> = snap.routes.values().collect();
    routes.sort_by(|a, b| a.id.cmp(&b.id));
    for route in &routes {
        for link_id in &route.links {
            if edge_by_link.contains_key(link_id) {
                continue;
            }
            let polyline = resolve_snapshot_polyline(world, link_id, &snap.link_polylines);
            let edge_id = push_edge(
                &mut nodes,
                &mut point_nodes,
                &mut edges,
                link_id.clone(),
                polyline,
                EdgeKind::Road,
            );
            edge_by_link.insert(link_id.clone(), edge_id);
        }
    }

    for (link_id, kind) in persisted_walking_links(snap) {
        if edge_by_link.contains_key(&link_id) {
            continue;
        }
        let polyline = resolve_snapshot_polyline(world, &link_id, &snap.link_polylines);
        let edge_id = push_edge(
            &mut nodes,
            &mut point_nodes,
            &mut edges,
            link_id.clone(),
            polyline,
            kind,
        );
        if link_id.starts_with("link:walk:") {
            push_reverse_footway_for_edge(&mut edges, edge_id);
        }
        edge_by_link.insert(link_id, edge_id);
    }

    let mut walk_link_ids: Vec<_> = snap
        .link_polylines
        .keys()
        .filter(|link_id| link_id.starts_with("link:walk:"))
        .cloned()
        .collect();
    walk_link_ids.sort();
    for link_id in walk_link_ids {
        if edge_by_link.contains_key(&link_id) {
            continue;
        }
        let polyline = resolve_snapshot_polyline(world, &link_id, &snap.link_polylines);
        let edge_id = push_edge(
            &mut nodes,
            &mut point_nodes,
            &mut edges,
            link_id.clone(),
            polyline,
            EdgeKind::Footway,
        );
        push_reverse_footway_for_edge(&mut edges, edge_id);
        edge_by_link.insert(link_id, edge_id);
    }

    let mut waiting = WaitingAgents::default();
    let mut stop_aliases: Vec<(String, NodeId)> = Vec::new();
    let mut stops: Vec<_> = snap.stops.values().collect();
    stops.sort_by(|a, b| a.id.cmp(&b.id));
    for stop in stops {
        let route = snap.routes.get(&stop.route_id).unwrap_or_else(|| {
            panic!(
                "apply_into_world: persisted stop {} references unknown route {}",
                stop.id, stop.route_id
            )
        });
        let link_id = route.links.get(stop.link_index).unwrap_or_else(|| {
            panic!(
                "apply_into_world: persisted stop {} references missing link index {} on route {}",
                stop.id, stop.link_index, stop.route_id
            )
        });
        let edge_id = *edge_by_link.get(link_id).unwrap_or_else(|| {
            panic!(
                "apply_into_world: persisted stop {} references unbuilt link {}",
                stop.id, link_id
            )
        });
        let edge = &edges[edge_id.0 as usize];
        let node_id = if stop.progress <= 0.0 {
            edge.from
        } else if stop.progress >= 1.0 {
            edge.to
        } else {
            let position = crate::mobility_geometry::world_coord_at_progress_slice(
                &edge.polyline,
                stop.progress,
            );
            let id = NodeId(nodes.len() as u32);
            nodes.push(Node {
                id,
                position,
                kind: NodeKind::TransitStop,
                legacy_id: None,
            });
            id
        };

        let node = &mut nodes[node_id.0 as usize];
        node.kind = NodeKind::TransitStop;
        if node.legacy_id.is_none() {
            node.legacy_id = Some(stop.id.clone());
        } else {
            stop_aliases.push((stop.id.clone(), node_id));
        }
        for agent_id in &stop.waiting_agents {
            waiting.enqueue(node_id, agent_id.clone());
        }
    }

    let mut graph = Graph::new(nodes, edges);
    for (legacy_id, node_id) in stop_aliases {
        graph.add_legacy_node_alias(legacy_id, node_id);
    }

    let traffic_routes = routes
        .into_iter()
        .enumerate()
        .map(|(index, route)| TrafficRoute {
            id: TrafficRouteId(index as u32),
            name: route.id.clone(),
            edges: route
                .links
                .iter()
                .map(|link_id| {
                    *edge_by_link.get(link_id).unwrap_or_else(|| {
                        panic!(
                            "apply_into_world: persisted route {} references unbuilt link {}",
                            route.id, link_id
                        )
                    })
                })
                .collect(),
            legacy_route_id: route.id.clone(),
        })
        .collect();

    let spatial_index = NodeSpatialIndex::from_nodes(graph.nodes());
    world.insert_resource(graph);
    world.insert_resource(spatial_index);
    world.insert_resource(TrafficRoutes::new(traffic_routes));
    world.insert_resource(waiting);
}

fn parse_edge_key(key: &str) -> Option<EdgeId> {
    key.strip_prefix("edge:")
        .and_then(|raw| raw.parse::<u32>().ok())
        .map(EdgeId)
}

fn canonical_edge_key(edge: &Edge) -> String {
    edge.legacy_id
        .clone()
        .unwrap_or_else(|| format!("edge:{}", edge.id.0))
}

fn resolve_canonical_edge_key(graph: &Graph, key: &str) -> Option<EdgeId> {
    if let Some(edge_id) = graph.edge_by_legacy(key) {
        return Some(edge_id);
    }

    if let Some(edge_id) = parse_edge_key(key)
        && (edge_id.0 as usize) < graph.edge_count()
    {
        let edge = graph.edge(edge_id);
        if canonical_edge_key(edge) == key {
            return Some(edge_id);
        }
    }

    None
}

fn expect_canonical_edge_key(graph: &Graph, key: &str) -> EdgeId {
    resolve_canonical_edge_key(graph, key)
        .unwrap_or_else(|| panic!("persisted active_route canonical edge key {key} is missing"))
}

fn validate_active_route_mode(
    profile: RoutingProfileKey,
    from_mode: ModeState,
    from_node_kind: NodeKind,
    edge: &Edge,
    expected_next_mode: ModeState,
    key: &str,
) -> ModeState {
    let Some((next_mode, _cost)) =
        RoutingProfile::for_key(profile).transition(from_mode, from_node_kind, edge)
    else {
        panic!(
            "apply_into_world: persisted active_route step {} mode {:?} cannot traverse {:?} from {:?} with profile {:?}",
            key, from_mode, edge.kind, from_node_kind, profile
        );
    };
    if next_mode != expected_next_mode {
        panic!(
            "apply_into_world: persisted active_route step {} expected mode {:?} but profile transition produced {:?}",
            key, expected_next_mode, next_mode
        );
    }
    next_mode
}

fn initial_mode_for_profile(profile: RoutingProfileKey) -> ModeState {
    match profile {
        RoutingProfileKey::Walk | RoutingProfileKey::WalkTransit => ModeState::Walking,
        RoutingProfileKey::Car => ModeState::Driving,
        RoutingProfileKey::Tram => {
            panic!("apply_into_world: persisted active_route contains retired tram profile")
        }
    }
}

fn normalize_active_route(graph: &Graph, route: &PersistedActiveRoute) -> PersistedActiveRoute {
    if route.steps.is_empty() {
        panic!("apply_into_world: persisted active_route has no steps");
    }
    if route.cursor >= route.steps.len() {
        panic!(
            "apply_into_world: persisted active_route cursor {} is outside {} steps",
            route.cursor,
            route.steps.len()
        );
    }
    let mut normalized_steps = Vec::with_capacity(route.steps.len());
    let mut previous_edge_id: Option<EdgeId> = None;
    let mut mode = initial_mode_for_profile(route.profile);
    for step in &route.steps {
        if step.length < 0.0 || !step.length.is_finite() {
            panic!(
                "apply_into_world: persisted active_route edge {} has invalid length {}",
                step.edge_id, step.length
            );
        }
        let edge_id = expect_canonical_edge_key(graph, &step.canonical_edge_key);
        let edge = graph.edge(edge_id);
        let expected_key = canonical_edge_key(edge);
        if step.canonical_edge_key != expected_key {
            panic!(
                "apply_into_world: persisted active_route edge {} canonical key mismatch: got {}, expected {}",
                step.edge_id, step.canonical_edge_key, expected_key
            );
        }
        if let Some(previous) = previous_edge_id
            && graph.edge(previous).to != edge.from
        {
            panic!(
                "apply_into_world: persisted active_route edge {} is disconnected from previous step",
                step.edge_id
            );
        }
        mode = validate_active_route_mode(
            route.profile,
            mode,
            graph.node(edge.from).kind,
            edge,
            step.mode,
            &step.canonical_edge_key,
        );

        normalized_steps.push(PersistedRouteStep {
            edge_id: edge.id.0,
            mode: step.mode,
            canonical_edge_key: step.canonical_edge_key.clone(),
            length: step.length,
        });
        previous_edge_id = Some(edge_id);
    }

    let final_edge_id = EdgeId(
        normalized_steps
            .last()
            .expect("active route steps checked")
            .edge_id,
    );
    let destination_node = graph.edge(final_edge_id).to;

    PersistedActiveRoute {
        destination_node: destination_node.0,
        profile: route.profile,
        cursor: route.cursor,
        steps: normalized_steps,
    }
}

fn normalize_persisted_active_routes(
    graph: &Graph,
    agents: &HashMap<AgentId, AgentRecord>,
) -> HashMap<AgentId, AgentRecord> {
    agents
        .iter()
        .map(|(id, agent)| {
            let mut normalized = agent.clone();
            if let Some(active_route) = &agent.active_route {
                normalized.active_route = Some(normalize_active_route(graph, active_route));
            }
            (id.clone(), normalized)
        })
        .collect()
}

/// Hydrate a freshly-installed mobility World from a persist snapshot.
///
/// Registers graph data before spawning agents/vehicles so the spawn helpers
/// resolve real positions from the routing graph.
pub fn apply_into_world(world: &mut World, snap: MobilityPersistSnapshot) {
    world.resource_mut::<Tick>().0 = snap.tick;
    world
        .resource_mut::<crate::mobility::resources::LastProcessedMonth>()
        .0 = snap.last_processed_month;
    install_snapshot_routing(world, &snap);
    let agents = {
        let graph = world.resource::<Graph>();
        normalize_persisted_active_routes(graph, &snap.agents)
    };
    world.insert_resource(PersistedStopMetadata(snap.stops.clone()));
    world.insert_resource(PersistedLinkPolylineMetadata(snap.link_polylines.clone()));
    for vehicle in snap.vehicles.values() {
        crate::mobility::api::spawn_vehicle_from_record(world, vehicle.clone());
    }
    for agent in agents.values() {
        crate::mobility::api::spawn_agent_from_record(world, agent.clone());
    }
    world.resource_mut::<FlowCells>().0 = snap.flow_cells.clone();
    // Re-apply chunk activities into chunk-entity LOD markers. If a coord
    // has no chunk entity yet (round-trip test path: only mobility plugins
    // installed, no chunks loaded by the persistence layer), spawn an
    // empty-tiles chunk entity at that coord so the marker has somewhere
    // to live. Production hydration always installs CorePlugin first.
    for (coord, activity) in &snap.chunk_activities {
        crate::mobility::api::seed_chunk_activity(world, *coord, *activity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "retired tram mode")]
    fn snapshot_restore_rejects_retired_tram_active_route_mode() {
        let _ = edge_kind_for_mode(ModeState::OnTram);
    }

    #[test]
    #[should_panic(expected = "retired tram profile")]
    fn snapshot_restore_rejects_retired_tram_active_route_profile() {
        let _ = initial_mode_for_profile(RoutingProfileKey::Tram);
    }
}
