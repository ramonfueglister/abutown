use std::collections::HashMap;

use crate::city_network::CityNetwork;
use crate::routing::graph::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::transit::{LineId, TransitLine, TransitLines};

/// Speed-limit constants for 8b (placeholders for future per-edge data).
const SPEED_TRAM: f32 = 4.0;
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
) -> (Graph, TransitLines, NodeSpatialIndex) {
    // Phase 1: collect polylines + coord occurrence count.
    let mut coord_use_count: HashMap<(i32, i32), u32> = HashMap::new();
    let mut endpoint_coords: Vec<(i32, i32)> = Vec::new();
    let mut polyline_coords: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut polyline_kinds: Vec<PolylineKind> = Vec::new();

    for (idx, path) in network.arterial_paths.iter().enumerate() {
        let coords = path.iter().map(|nc| (nc.x, nc.y)).collect::<Vec<_>>();
        if coords.is_empty() {
            continue;
        }
        endpoint_coords.push(coords[0]);
        endpoint_coords.push(*coords.last().unwrap());
        for c in &coords {
            *coord_use_count.entry(*c).or_insert(0) += 1;
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
    let mut walk_coords: Vec<(i32, i32)> = Vec::new();
    for walk in seeded_walks {
        if walk.polyline.len() < 2 {
            continue;
        }
        let from = (
            walk.polyline.first().unwrap().0.round() as i32,
            walk.polyline.first().unwrap().1.round() as i32,
        );
        let to = (
            walk.polyline.last().unwrap().0.round() as i32,
            walk.polyline.last().unwrap().1.round() as i32,
        );
        walk_coords.push(from);
        walk_coords.push(to);
    }

    // Phase 2: identify which coords become nodes.
    let mut is_node: HashMap<(i32, i32), bool> = HashMap::new();
    for c in &endpoint_coords {
        is_node.insert(*c, true);
    }
    for (c, count) in &coord_use_count {
        if *count >= 2 {
            is_node.insert(*c, true);
        }
    }
    for stop in seeded_stops {
        let coord = (stop.coord.0.round() as i32, stop.coord.1.round() as i32);
        is_node.insert(coord, true);
    }
    for coord in &walk_coords {
        is_node.insert(*coord, true);
    }

    // Phase 3: assign NodeIds deterministically (sorted by coord).
    let mut node_coords: Vec<(i32, i32)> = is_node.keys().copied().collect();
    node_coords.sort();
    let mut nodes: Vec<Node> = Vec::with_capacity(node_coords.len());
    let mut node_id_by_coord: HashMap<(i32, i32), NodeId> = HashMap::new();
    for (idx, coord) in node_coords.iter().enumerate() {
        let id = NodeId(idx as u32);
        node_id_by_coord.insert(*coord, id);
        nodes.push(Node {
            id,
            position: (coord.0 as f32, coord.1 as f32),
            kind: NodeKind::Intersection,
            legacy_id: None,
        });
    }

    // Mark stop nodes.
    for stop in seeded_stops {
        let coord = (stop.coord.0.round() as i32, stop.coord.1.round() as i32);
        let node_id = *node_id_by_coord
            .get(&coord)
            .expect("seeded stop coord must be a node");
        let n = &mut nodes[node_id.0 as usize];
        n.kind = NodeKind::TransitStop;
        n.legacy_id = Some(stop.legacy_stop_id.clone());
    }

    // Phase 4: split polylines at node coords, emit edges.
    let mut edges: Vec<Edge> = Vec::new();
    let mut tram_edges_by_arterial: HashMap<usize, Vec<EdgeId>> = HashMap::new();

    for (poly_idx, coords) in polyline_coords.iter().enumerate() {
        let kind = polyline_kinds[poly_idx];
        let mut split_indices: Vec<usize> = vec![0];
        for (i, c) in coords
            .iter()
            .enumerate()
            .skip(1)
            .take(coords.len().saturating_sub(2))
        {
            if node_id_by_coord.contains_key(c) {
                split_indices.push(i);
            }
        }
        split_indices.push(coords.len() - 1);
        for win in split_indices.windows(2) {
            let (a, b) = (win[0], win[1]);
            let segment = &coords[a..=b];
            let from = node_id_by_coord[&segment[0]];
            let to = node_id_by_coord[segment.last().unwrap()];
            let polyline: Vec<(f32, f32)> =
                segment.iter().map(|c| (c.0 as f32, c.1 as f32)).collect();
            let length = polyline_length(&polyline);
            match kind {
                PolylineKind::Arterial { index } => {
                    let tram_legacy_fwd = Some(format!(
                        "link:tram:{}:{}_{}",
                        index, segment[0].0, segment[0].1
                    ));
                    let tram_fwd = Edge {
                        id: EdgeId(edges.len() as u32),
                        from,
                        to,
                        polyline: polyline.clone(),
                        length,
                        kind: EdgeKind::TramTrack,
                        speed_limit: SPEED_TRAM,
                        capacity: 1,
                        legacy_id: tram_legacy_fwd,
                    };
                    tram_edges_by_arterial
                        .entry(index)
                        .or_default()
                        .push(tram_fwd.id);
                    edges.push(tram_fwd);
                    edges.push(Edge {
                        id: EdgeId(edges.len() as u32),
                        from: to,
                        to: from,
                        polyline: polyline.iter().rev().copied().collect(),
                        length,
                        kind: EdgeKind::TramTrack,
                        speed_limit: SPEED_TRAM,
                        capacity: 1,
                        legacy_id: None,
                    });
                    edges.push(Edge {
                        id: EdgeId(edges.len() as u32),
                        from,
                        to,
                        polyline: polyline.clone(),
                        length,
                        kind: EdgeKind::Road,
                        speed_limit: SPEED_ROAD,
                        capacity: 1,
                        legacy_id: None,
                    });
                    edges.push(Edge {
                        id: EdgeId(edges.len() as u32),
                        from: to,
                        to: from,
                        polyline: polyline.iter().rev().copied().collect(),
                        length,
                        kind: EdgeKind::Road,
                        speed_limit: SPEED_ROAD,
                        capacity: 1,
                        legacy_id: None,
                    });
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
        let from_coord = (first.0.round() as i32, first.1.round() as i32);
        let to_coord = (last.0.round() as i32, last.1.round() as i32);
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
        let coord = (stop.coord.0.round() as i32, stop.coord.1.round() as i32);
        if let Some(node_id) = node_id_by_coord.get(&coord).copied() {
            graph.add_legacy_node_alias(stop.legacy_stop_id.clone(), node_id);
        }
    }
    let spatial_index = NodeSpatialIndex::from_nodes(graph.nodes());

    // Phase 5: transit lines — one per arterial.
    let mut lines: Vec<TransitLine> = Vec::new();
    // Sort arterial indices for deterministic LineId order.
    let mut arterial_indices: Vec<usize> = tram_edges_by_arterial.keys().copied().collect();
    arterial_indices.sort();
    for arterial_idx in arterial_indices {
        let edges_in_line = tram_edges_by_arterial.remove(&arterial_idx).unwrap();
        let stops_in_line: Vec<NodeId> = graph
            .nodes()
            .iter()
            .filter(|n| n.kind == NodeKind::TransitStop)
            .filter(|n| {
                let np = n.position;
                edges_in_line.iter().any(|e| {
                    graph
                        .edge(*e)
                        .polyline
                        .iter()
                        .any(|p| p.0 == np.0 && p.1 == np.1)
                })
            })
            .map(|n| n.id)
            .collect();
        let legacy_route_id = if arterial_idx == 0 {
            Some("route:horizontal".to_string())
        } else if arterial_idx == 1 {
            Some("route:vertical".to_string())
        } else {
            None
        };
        lines.push(TransitLine {
            id: LineId(lines.len() as u32),
            name: format!("arterial_{arterial_idx}"),
            edges: edges_in_line,
            stops: stops_in_line,
            legacy_route_id,
        });
    }
    let mut transit_lines = TransitLines::new(lines);
    for arterial_idx in 0..network.arterial_paths.len() {
        if let Some(line_id) = transit_lines
            .line_by_legacy("route:horizontal")
            .filter(|_| arterial_idx == 0)
            .or_else(|| {
                transit_lines
                    .line_by_legacy("route:vertical")
                    .filter(|_| arterial_idx == 1)
            })
            .or_else(|| {
                (arterial_idx < transit_lines.count()).then_some(LineId(arterial_idx as u32))
            })
        {
            transit_lines.add_legacy_route_alias(format!("route:arterial:{arterial_idx}"), line_id);
        }
    }

    (graph, transit_lines, spatial_index)
}

#[derive(Debug, Clone, Copy)]
enum PolylineKind {
    Arterial { index: usize },
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
    use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};

    fn nc(x: i32, y: i32) -> NetworkCoord {
        NetworkCoord { x, y }
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
                vec![nc(0, 0), nc(5, 0), nc(10, 0)],
                vec![nc(5, 0), nc(5, 5)],
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
    fn builder_emits_bidirectional_tram_plus_road_per_arterial_segment() {
        let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &[]);
        // Arterial 0 has 2 segments → 4 edges each = 8.
        // Arterial 1 has 1 segment → 4 edges.
        // Total = 12.
        assert_eq!(graph.edge_count(), 12);
        for e in graph.edges() {
            assert!(matches!(e.kind, EdgeKind::TramTrack | EdgeKind::Road));
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
    fn builder_creates_one_transit_line_per_arterial() {
        let (_, lines, _) = build_graph_from_city_network(&simple_network(), &[], &[]);
        assert_eq!(lines.count(), 2);
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
        assert_eq!(graph.edge_count(), 12);
    }
}
