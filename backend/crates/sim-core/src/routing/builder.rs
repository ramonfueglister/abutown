use std::collections::HashMap;

use crate::city_network::CityNetwork;
use crate::routing::graph::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::traffic::{TrafficRoute, TrafficRouteId, TrafficRoutes};

/// Speed-limit constants for 8b (placeholders for future per-edge data).
const SPEED_ROAD: f32 = 6.0;
const SPEED_FOOT: f32 = 1.0;

/// Pre-seeded transit stops. Source of truth stays in `mobility/seed.rs`;
/// the builder takes them as input and snaps each to a graph node.
#[derive(Debug, Clone)]
pub struct SeededStop {
    pub legacy_stop_id: String,
    pub coord: (f32, f32),
    pub legacy_route_id: String,
}

/// Pre-seeded pedestrian walk. Today `seed.rs` generates one walk per
/// procedural corridor with a stable id. Builder emits one Footway edge
/// per SeededWalk (NOT split at intersections — walks own their geometry).
#[derive(Debug, Clone)]
pub struct SeededWalk {
    pub legacy_link_id: String,
    pub polyline: Vec<(f32, f32)>,
}

pub fn build_graph_from_city_network(
    network: &CityNetwork,
    seeded_stops: &[SeededStop],
    seeded_walks: &[SeededWalk],
) -> (Graph, TrafficRoutes, NodeSpatialIndex) {
    // Phase 1: collect polylines + coord occurrence count.
    let mut coord_use_count: HashMap<CoordKey, u32> = HashMap::new();
    let mut point_by_key: HashMap<CoordKey, (f32, f32)> = HashMap::new();
    let mut endpoint_coords: Vec<CoordKey> = Vec::new();
    let mut polyline_coords: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut polyline_kinds: Vec<PolylineKind> = Vec::new();

    for (idx, path) in network.arterial_paths.iter().enumerate() {
        let coords = path
            .iter()
            .map(|point| (point.x, point.y))
            .collect::<Vec<_>>();
        if coords.is_empty() {
            continue;
        }
        endpoint_coords.push(remember_point(&mut point_by_key, coords[0]));
        endpoint_coords.push(remember_point(&mut point_by_key, *coords.last().unwrap()));
        for coord in &coords {
            let key = remember_point(&mut point_by_key, *coord);
            *coord_use_count.entry(key).or_insert(0) += 1;
        }
        polyline_coords.push(coords);
        polyline_kinds.push(PolylineKind::Arterial { index: idx });
    }

    // Pedestrian corridors from the network JSON are intentionally NOT
    // processed here. The seed path generates its own walks procedurally
    // and publishes them via `seeded_walks`; that is the SOLE source of
    // Footway edges in the graph. See `seeded_walks_from_network` in
    // `mobility/seed.rs`.

    // Collect seeded-walk endpoint coords so they become graph nodes
    // alongside arterial endpoints + intersections + seeded stops. The
    // walks themselves are emitted as Footway edges after arterial
    // splitting (Phase 4b below) — they are NOT split at intersections.
    let mut walk_coords: Vec<CoordKey> = Vec::new();
    for walk in seeded_walks {
        if walk.polyline.len() < 2 {
            continue;
        }
        let from = remember_point(&mut point_by_key, *walk.polyline.first().unwrap());
        let to = remember_point(&mut point_by_key, *walk.polyline.last().unwrap());
        walk_coords.push(from);
        walk_coords.push(to);
    }

    // Phase 2: identify which coords become nodes.
    let mut is_node: HashMap<CoordKey, bool> = HashMap::new();
    for c in &endpoint_coords {
        is_node.insert(*c, true);
    }
    for (c, count) in &coord_use_count {
        if *count >= 2 {
            is_node.insert(*c, true);
        }
    }
    for stop in seeded_stops {
        is_node.insert(remember_point(&mut point_by_key, stop.coord), true);
    }
    for coord in &walk_coords {
        is_node.insert(*coord, true);
    }

    // Phase 3: assign NodeIds deterministically (sorted by coord).
    let mut node_keys: Vec<CoordKey> = is_node.keys().copied().collect();
    node_keys.sort();
    let mut nodes: Vec<Node> = Vec::with_capacity(node_keys.len());
    let mut node_id_by_coord: HashMap<CoordKey, NodeId> = HashMap::new();
    for (idx, key) in node_keys.iter().enumerate() {
        let id = NodeId(idx as u32);
        node_id_by_coord.insert(*key, id);
        nodes.push(Node {
            id,
            position: point_by_key[key],
            kind: NodeKind::Intersection,
            legacy_id: None,
        });
    }

    // Mark stop nodes.
    for stop in seeded_stops {
        let coord = coord_key(stop.coord);
        let node_id = *node_id_by_coord
            .get(&coord)
            .expect("seeded stop coord must be a node");
        let n = &mut nodes[node_id.0 as usize];
        n.kind = NodeKind::TransitStop;
        n.legacy_id = Some(stop.legacy_stop_id.clone());
    }

    // Phase 4: split polylines at node coords, emit edges.
    let mut edges: Vec<Edge> = Vec::new();
    let mut road_forward_by_arterial: HashMap<usize, Vec<EdgeId>> = HashMap::new();
    let mut road_reverse_by_arterial: HashMap<usize, Vec<EdgeId>> = HashMap::new();

    for (poly_idx, coords) in polyline_coords.iter().enumerate() {
        let kind = polyline_kinds[poly_idx];
        let mut split_indices: Vec<usize> = vec![0];
        for (i, c) in coords
            .iter()
            .enumerate()
            .skip(1)
            .take(coords.len().saturating_sub(2))
        {
            if node_id_by_coord.contains_key(&coord_key(*c)) {
                split_indices.push(i);
            }
        }
        split_indices.push(coords.len() - 1);
        for win in split_indices.windows(2) {
            let (a, b) = (win[0], win[1]);
            let segment = &coords[a..=b];
            let from = node_id_by_coord[&coord_key(segment[0])];
            let to = node_id_by_coord[&coord_key(*segment.last().unwrap())];
            let polyline: Vec<(f32, f32)> = segment.to_vec();
            let length = polyline_length(&polyline);
            match kind {
                PolylineKind::Arterial { index } => {
                    let legacy_key = road_legacy_coord_key(segment[0]);
                    let fwd_id = EdgeId(edges.len() as u32);
                    edges.push(Edge {
                        id: fwd_id,
                        from,
                        to,
                        polyline: polyline.clone(),
                        length,
                        kind: EdgeKind::Road,
                        speed_limit: SPEED_ROAD,
                        capacity: 1,
                        legacy_id: Some(format!("link:road:{index}:{legacy_key},fwd")),
                    });
                    road_forward_by_arterial
                        .entry(index)
                        .or_default()
                        .push(fwd_id);

                    let rev_id = EdgeId(edges.len() as u32);
                    edges.push(Edge {
                        id: rev_id,
                        from: to,
                        to: from,
                        polyline: polyline.iter().rev().copied().collect(),
                        length,
                        kind: EdgeKind::Road,
                        speed_limit: SPEED_ROAD,
                        capacity: 1,
                        legacy_id: Some(format!("link:road:{index}:{legacy_key},rev")),
                    });
                    road_reverse_by_arterial
                        .entry(index)
                        .or_default()
                        .push(rev_id);
                }
            }
        }
    }

    // Phase 4b: emit one Footway edge per seeded walk (NOT split at
    // intersections — pedestrian walks own their geometry end-to-end and
    // don't share topology with the arterial network today). The seed path
    // is the sole source of these legacy ids; `walk_advance_system`
    // resolves the walker's `link_id` via `graph.edge_by_legacy`.
    for walk in seeded_walks {
        if walk.polyline.len() < 2 {
            continue;
        }
        let first = walk.polyline.first().unwrap();
        let last = walk.polyline.last().unwrap();
        let from_coord = coord_key(*first);
        let to_coord = coord_key(*last);
        let from = node_id_by_coord[&from_coord];
        let to = node_id_by_coord[&to_coord];
        let length = polyline_length(&walk.polyline);
        edges.push(Edge {
            id: EdgeId(edges.len() as u32),
            from,
            to,
            polyline: walk.polyline.clone(),
            length,
            kind: EdgeKind::Footway,
            speed_limit: SPEED_FOOT,
            capacity: 1,
            legacy_id: Some(walk.legacy_link_id.clone()),
        });
        edges.push(Edge {
            id: EdgeId(edges.len() as u32),
            from: to,
            to: from,
            polyline: walk.polyline.iter().rev().copied().collect(),
            length,
            kind: EdgeKind::Footway,
            speed_limit: SPEED_FOOT,
            capacity: 1,
            legacy_id: None,
        });
    }

    let mut graph = Graph::new(nodes, edges);
    for stop in seeded_stops {
        let coord = coord_key(stop.coord);
        if let Some(node_id) = node_id_by_coord.get(&coord).copied() {
            graph.add_legacy_node_alias(stop.legacy_stop_id.clone(), node_id);
        }
    }
    let spatial_index = NodeSpatialIndex::from_nodes(graph.nodes());

    let mut routes: Vec<TrafficRoute> = Vec::new();
    let mut arterial_indices: Vec<usize> = road_forward_by_arterial.keys().copied().collect();
    arterial_indices.sort();
    for arterial_idx in arterial_indices {
        let mut route_edges = road_forward_by_arterial
            .remove(&arterial_idx)
            .unwrap_or_default();
        if let Some(reverse_edges) = road_reverse_by_arterial.remove(&arterial_idx) {
            route_edges.extend(reverse_edges.into_iter().rev());
        }
        if route_edges.is_empty() {
            continue;
        }
        routes.push(TrafficRoute {
            id: TrafficRouteId(routes.len() as u32),
            name: format!("arterial_{arterial_idx}"),
            edges: route_edges,
            legacy_route_id: format!("route:arterial:{arterial_idx}"),
        });
    }

    let traffic_routes = TrafficRoutes::new(routes);
    (graph, traffic_routes, spatial_index)
}

#[derive(Debug, Clone, Copy)]
enum PolylineKind {
    Arterial { index: usize },
}

type CoordKey = (i32, i32);
const COORD_KEY_SCALE: f32 = 1000.0;

fn coord_key(point: (f32, f32)) -> CoordKey {
    (
        (point.0 * COORD_KEY_SCALE).round() as i32,
        (point.1 * COORD_KEY_SCALE).round() as i32,
    )
}

fn remember_point(points: &mut HashMap<CoordKey, (f32, f32)>, point: (f32, f32)) -> CoordKey {
    let key = coord_key(point);
    // Quantized keys intentionally coalesce near-identical authored coordinates;
    // the first authored point wins so node positions stay deterministic.
    points.entry(key).or_insert(point);
    key
}

fn road_legacy_coord_key(point: (f32, f32)) -> String {
    let key = coord_key(point);
    format!(
        "{}_{}",
        road_legacy_coord_axis(point.0, key.0),
        road_legacy_coord_axis(point.1, key.1)
    )
}

fn road_legacy_coord_axis(value: f32, quantized: i32) -> String {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i32::MIN as f32
        && value <= i32::MAX as f32
    {
        (value as i32).to_string()
    } else {
        format!("q{quantized}")
    }
}

fn polyline_length(points: &[(f32, f32)]) -> f32 {
    points
        .windows(2)
        .map(|w| {
            let dx = w[1].0 - w[0].0;
            let dy = w[1].1 - w[0].1;
            (dx * dx + dy * dy).sqrt()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city_network::{CityNetwork, NetworkPoint, WorldTiles};

    fn np(x: i32, y: i32) -> NetworkPoint {
        NetworkPoint {
            x: x as f32,
            y: y as f32,
        }
    }

    fn simple_network() -> CityNetwork {
        // Two arterials forming a T-junction at (5, 0).
        // Arterial 0: (0,0) → (5,0) → (10,0)
        // Arterial 1: (5,0) → (5,5)
        CityNetwork {
            version: 1,
            world_id: "test".into(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 32,
                height: 32,
            },
            arterial_paths: vec![
                vec![np(0, 0), np(5, 0), np(10, 0)],
                vec![np(5, 0), np(5, 5)],
            ],
            pedestrian_corridors: vec![],
        }
    }

    #[test]
    fn builder_creates_nodes_at_intersections() {
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &[]);
        // Expected nodes: endpoints (0,0), (10,0), (5,5) AND intersection (5,0).
        assert_eq!(graph.node_count(), 4);
    }

    #[test]
    fn builder_emits_bidirectional_road_per_arterial_segment() {
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &[]);
        // Arterial 0 has 2 segments -> 2 edges each = 4.
        // Arterial 1 has 1 segment -> 2 edges.
        // Total = 6.
        assert_eq!(graph.edge_count(), 6);
        for e in graph.edges() {
            assert_eq!(e.kind, EdgeKind::Road);
        }
    }

    #[test]
    fn builder_uses_seeded_stops_as_nodes() {
        let stops = vec![SeededStop {
            legacy_stop_id: "stop:on_arterial".into(),
            coord: (5.0, 0.0),
            legacy_route_id: "route:horizontal".into(),
        }];
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &stops, &[]);
        let node_id = graph
            .node_by_legacy("stop:on_arterial")
            .expect("stop must resolve");
        assert_eq!(graph.node(node_id).kind, NodeKind::TransitStop);
    }

    #[test]
    fn seeded_walks_become_footway_edges() {
        let walks = vec![SeededWalk {
            legacy_link_id: "link:walk:corridor:7".into(),
            polyline: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
        }];
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &walks);
        let edge_id = graph
            .edge_by_legacy("link:walk:corridor:7")
            .expect("walk edge must resolve");
        let edge = graph.edge(edge_id);
        assert_eq!(edge.kind, EdgeKind::Footway);
        assert_eq!(edge.polyline.len(), 3);
        // The reverse edge must also exist (no legacy id on the reverse).
        assert!(
            graph
                .edges()
                .iter()
                .filter(|e| e.kind == EdgeKind::Footway)
                .count()
                >= 2
        );
    }

    #[test]
    fn builder_preserves_fractional_seeded_walk_nodes() {
        let walks = vec![
            SeededWalk {
                legacy_link_id: "link:walk:corridor:north".into(),
                polyline: vec![(2.0, 2.49), (13.0, 2.49)],
            },
            SeededWalk {
                legacy_link_id: "link:walk:corridor:south".into(),
                polyline: vec![(2.0, 3.51), (13.0, 3.51)],
            },
        ];

        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &walks);
        let north = graph.edge(
            graph
                .edge_by_legacy("link:walk:corridor:north")
                .expect("north sidewalk edge exists"),
        );
        let south = graph.edge(
            graph
                .edge_by_legacy("link:walk:corridor:south")
                .expect("south sidewalk edge exists"),
        );

        assert_eq!(north.polyline, vec![(2.0, 2.49), (13.0, 2.49)]);
        assert_eq!(south.polyline, vec![(2.0, 3.51), (13.0, 3.51)]);
        assert_eq!(graph.node(north.from).position, (2.0, 2.49));
        assert_eq!(graph.node(north.to).position, (13.0, 2.49));
        assert_eq!(graph.node(south.from).position, (2.0, 3.51));
        assert_eq!(graph.node(south.to).position, (13.0, 3.51));
        assert!(
            graph
                .nodes()
                .iter()
                .any(|node| node.position == (2.0, 2.49))
        );
        assert!(
            graph
                .nodes()
                .iter()
                .any(|node| node.position == (2.0, 3.51))
        );
    }

    #[test]
    fn integer_road_legacy_ids_keep_unscaled_format() {
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &[]);

        assert!(graph.edge_by_legacy("link:road:0:0_0,fwd").is_some());
        assert!(graph.edge_by_legacy("link:road:0:5_0,fwd").is_some());
        assert!(graph.edge_by_legacy("link:road:0:5_0,rev").is_some());
        assert!(graph.edge_by_legacy("link:road:0:5000_0,fwd").is_none());
    }

    #[test]
    fn builder_creates_one_traffic_route_per_arterial() {
        let (_, routes, _) = build_graph_from_city_network(&simple_network(), &[], &[]);
        assert_eq!(routes.count(), 2);
    }

    #[test]
    fn builder_creates_traffic_routes_from_road_edges_only() {
        let (graph, traffic_routes, _) = build_graph_from_city_network(&simple_network(), &[], &[]);

        assert_eq!(traffic_routes.count(), 2);
        assert!(traffic_routes.route_by_legacy("route:arterial:0").is_some());
        assert!(traffic_routes.route_by_legacy("route:arterial:1").is_some());

        for route in traffic_routes.iter() {
            assert!(
                route.edges.len() >= 2,
                "traffic routes include forward and reverse road edges so route-end looping is physical"
            );
            for edge_id in &route.edges {
                assert_eq!(graph.edge(*edge_id).kind, EdgeKind::Road);
            }
        }
    }

    #[test]
    fn builder_does_not_create_tram_track_edges_for_runtime_routes() {
        let (graph, traffic_routes, _) = build_graph_from_city_network(&simple_network(), &[], &[]);

        assert!(
            graph
                .edges()
                .iter()
                .all(|edge| edge.kind != EdgeKind::TramTrack),
            "tram-track edges are not part of the mobility runtime graph"
        );
        assert_eq!(
            traffic_routes.count(),
            simple_network().arterial_paths.len()
        );
    }

    #[test]
    fn polyline_length_is_arc_length() {
        let p = vec![(0.0, 0.0), (3.0, 4.0), (3.0, 8.0)];
        assert_eq!(polyline_length(&p), 5.0 + 4.0);
    }

    #[test]
    fn empty_polyline_skipped() {
        let mut net = simple_network();
        net.arterial_paths.push(vec![]);
        let (graph, _, _) = build_graph_from_city_network(&net, &[], &[]);
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph.edge_count(), 6);
    }
}
