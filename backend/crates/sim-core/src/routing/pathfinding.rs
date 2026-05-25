use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::routing::{
    EdgeId, Graph, ModeState, NodeId, NodeSpatialIndex, RoutingProfile, RoutingProfileKey,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathRequest {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathEdge {
    pub edge_id: EdgeId,
    pub mode: ModeState,
    pub cost: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedPath {
    pub from: NodeId,
    pub to: NodeId,
    pub profile: RoutingProfileKey,
    pub edges: Vec<PathEdge>,
    pub total_cost: f32,
    pub total_length: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    MissingNode(NodeId),
    NoNearestNode,
    NoPath {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    InvalidGraph(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SearchState {
    node: NodeId,
    mode: ModeState,
}

#[derive(Debug, Clone, Copy)]
struct QueueEntry {
    state: SearchState,
    known_cost: f32,
    estimated_total: f32,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.estimated_total == other.estimated_total
            && self.known_cost == other.known_cost
            && self.state.node == other.state.node
            && self.state.mode == other.state.mode
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimated_total
            .partial_cmp(&self.estimated_total)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                other
                    .known_cost
                    .partial_cmp(&self.known_cost)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| other.state.node.0.cmp(&self.state.node.0))
            .then_with(|| other.state.mode.cmp(&self.state.mode))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct AStarRouter;

pub fn request_between_points(
    index: &NodeSpatialIndex,
    from: (f32, f32),
    to: (f32, f32),
    profile: RoutingProfileKey,
) -> Result<PathRequest, RoutingError> {
    let Some(from_node) = index.nearest(from) else {
        return Err(RoutingError::NoNearestNode);
    };
    let Some(to_node) = index.nearest(to) else {
        return Err(RoutingError::NoNearestNode);
    };
    Ok(PathRequest {
        from: from_node,
        to: to_node,
        profile,
    })
}

impl AStarRouter {
    pub fn find_path(
        graph: &Graph,
        request: PathRequest,
        profile: RoutingProfile,
    ) -> Result<PlannedPath, RoutingError> {
        validate_node(graph, request.from)?;
        validate_node(graph, request.to)?;

        if request.from == request.to {
            return Ok(PlannedPath {
                from: request.from,
                to: request.to,
                profile: request.profile,
                edges: Vec::new(),
                total_cost: 0.0,
                total_length: 0.0,
            });
        }

        let start = SearchState {
            node: request.from,
            mode: profile.initial_mode(),
        };
        let mut open = BinaryHeap::new();
        let mut best: HashMap<SearchState, f32> = HashMap::new();
        let mut came_from: HashMap<SearchState, (SearchState, PathEdge)> = HashMap::new();

        best.insert(start, 0.0);
        open.push(QueueEntry {
            state: start,
            known_cost: 0.0,
            estimated_total: heuristic(graph, request.from, request.to, profile),
        });

        while let Some(entry) = open.pop() {
            if entry.state.node == request.to {
                return reconstruct_path(graph, request, start, entry.state, &came_from);
            }

            if entry.known_cost > *best.get(&entry.state).unwrap_or(&f32::INFINITY) {
                continue;
            }

            let current_node = graph.node(entry.state.node);
            for edge_id in graph.outgoing(entry.state.node) {
                let edge = graph.edge(*edge_id);
                let Some((next_mode, edge_cost)) =
                    profile.transition(entry.state.mode, current_node.kind, edge)
                else {
                    continue;
                };
                if edge_cost < 0.0 || !edge_cost.is_finite() {
                    return Err(RoutingError::InvalidGraph(
                        "edge cost must be finite and non-negative",
                    ));
                }

                let next_state = SearchState {
                    node: edge.to,
                    mode: next_mode,
                };
                let next_cost = entry.known_cost + edge_cost;
                if next_cost < *best.get(&next_state).unwrap_or(&f32::INFINITY) {
                    best.insert(next_state, next_cost);
                    came_from.insert(
                        next_state,
                        (
                            entry.state,
                            PathEdge {
                                edge_id: *edge_id,
                                mode: next_mode,
                                cost: edge_cost,
                            },
                        ),
                    );
                    open.push(QueueEntry {
                        state: next_state,
                        known_cost: next_cost,
                        estimated_total: next_cost + heuristic(graph, edge.to, request.to, profile),
                    });
                }
            }
        }

        Err(RoutingError::NoPath {
            from: request.from,
            to: request.to,
            profile: request.profile,
        })
    }
}

fn validate_node(graph: &Graph, id: NodeId) -> Result<(), RoutingError> {
    if (id.0 as usize) < graph.node_count() {
        Ok(())
    } else {
        Err(RoutingError::MissingNode(id))
    }
}

fn heuristic(graph: &Graph, from: NodeId, to: NodeId, profile: RoutingProfile) -> f32 {
    let a = graph.node(from).position;
    let b = graph.node(to).position;
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    (dx * dx + dy * dy).sqrt() / profile.fastest_speed()
}

fn reconstruct_path(
    graph: &Graph,
    request: PathRequest,
    start: SearchState,
    mut current: SearchState,
    came_from: &HashMap<SearchState, (SearchState, PathEdge)>,
) -> Result<PlannedPath, RoutingError> {
    let mut edges = Vec::new();
    while current != start {
        let Some((previous, path_edge)) = came_from.get(&current).copied() else {
            return Err(RoutingError::InvalidGraph(
                "path reconstruction missing predecessor",
            ));
        };
        edges.push(path_edge);
        current = previous;
    }
    edges.reverse();
    let total_cost = edges.iter().map(|edge| edge.cost).sum();
    let total_length = edges
        .iter()
        .map(|edge| graph.edge(edge.edge_id).length)
        .sum();
    Ok(PlannedPath {
        from: request.from,
        to: request.to,
        profile: request.profile,
        edges,
        total_cost,
        total_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, EdgeKind, Node, NodeKind};

    fn node(id: u32, x: f32, y: f32, kind: NodeKind) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind,
            legacy_id: None,
        }
    }

    fn edge(id: u32, from: u32, to: u32, kind: EdgeKind, length: f32) -> Edge {
        Edge {
            id: EdgeId(id),
            from: NodeId(from),
            to: NodeId(to),
            polyline: vec![(from as f32, 0.0), (to as f32, 0.0)],
            length,
            kind,
            speed_limit: match kind {
                EdgeKind::Footway => 1.0,
                EdgeKind::Road => 6.0,
                EdgeKind::TramTrack => 4.0,
            },
            capacity: 1,
            legacy_id: Some(format!("edge:{id}")),
        }
    }

    fn graph_for_modes() -> Graph {
        Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
                node(2, 20.0, 0.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 10.0),
                edge(1, 1, 2, EdgeKind::Footway, 10.0),
                edge(2, 0, 2, EdgeKind::Road, 30.0),
            ],
        )
    }

    #[test]
    fn same_node_returns_empty_path() {
        let graph = graph_for_modes();
        let path = AStarRouter::find_path(
            &graph,
            PathRequest {
                from: NodeId(1),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("same-node route should exist");
        assert_eq!(path.edges, Vec::<PathEdge>::new());
        assert_eq!(path.total_cost, 0.0);
    }

    #[test]
    fn walk_profile_uses_footways() {
        let graph = graph_for_modes();
        let path = AStarRouter::find_path(
            &graph,
            PathRequest {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect("walk route should exist");
        assert_eq!(
            path.edges.iter().map(|e| e.edge_id).collect::<Vec<_>>(),
            vec![EdgeId(0), EdgeId(1)]
        );
        assert!(path.edges.iter().all(|e| e.mode == ModeState::Walking));
    }

    #[test]
    fn walk_profile_rejects_road_only_route() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
            ],
            vec![edge(0, 0, 1, EdgeKind::Road, 10.0)],
        );
        let err = AStarRouter::find_path(
            &graph,
            PathRequest {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            },
            RoutingProfile::for_key(RoutingProfileKey::Walk),
        )
        .expect_err("road-only graph should not satisfy walk route");
        assert_eq!(
            err,
            RoutingError::NoPath {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::Walk,
            }
        );
    }

    #[test]
    fn missing_nodes_are_errors() {
        let graph = graph_for_modes();
        assert_eq!(
            AStarRouter::find_path(
                &graph,
                PathRequest {
                    from: NodeId(99),
                    to: NodeId(1),
                    profile: RoutingProfileKey::Walk,
                },
                RoutingProfile::for_key(RoutingProfileKey::Walk),
            ),
            Err(RoutingError::MissingNode(NodeId(99)))
        );
    }

    fn walk_transit_graph() -> Graph {
        Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::TransitStop),
                node(2, 20.0, 0.0, NodeKind::TransitStop),
                node(3, 30.0, 0.0, NodeKind::Intersection),
            ],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, 10.0),
                edge(1, 1, 2, EdgeKind::TramTrack, 10.0),
                edge(2, 2, 3, EdgeKind::Footway, 10.0),
            ],
        )
    }

    #[test]
    fn walk_transit_combines_walk_tram_walk() {
        let graph = walk_transit_graph();
        let path = AStarRouter::find_path(
            &graph,
            PathRequest {
                from: NodeId(0),
                to: NodeId(3),
                profile: RoutingProfileKey::WalkTransit,
            },
            RoutingProfile::for_key(RoutingProfileKey::WalkTransit),
        )
        .expect("walk-transit route should exist");
        assert_eq!(
            path.edges.iter().map(|e| e.edge_id).collect::<Vec<_>>(),
            vec![EdgeId(0), EdgeId(1), EdgeId(2)]
        );
        assert_eq!(
            path.edges.iter().map(|e| e.mode).collect::<Vec<_>>(),
            vec![ModeState::Walking, ModeState::OnTram, ModeState::Walking]
        );
    }

    #[test]
    fn walk_transit_cannot_board_at_intersection() {
        let graph = Graph::new(
            vec![
                node(0, 0.0, 0.0, NodeKind::Intersection),
                node(1, 10.0, 0.0, NodeKind::Intersection),
            ],
            vec![edge(0, 0, 1, EdgeKind::TramTrack, 10.0)],
        );
        let err = AStarRouter::find_path(
            &graph,
            PathRequest {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::WalkTransit,
            },
            RoutingProfile::for_key(RoutingProfileKey::WalkTransit),
        )
        .expect_err("boarding at intersection must be illegal");
        assert_eq!(
            err,
            RoutingError::NoPath {
                from: NodeId(0),
                to: NodeId(1),
                profile: RoutingProfileKey::WalkTransit,
            }
        );
    }

    #[test]
    fn request_between_points_resolves_nearest_nodes() {
        let graph = graph_for_modes();
        let index = crate::routing::NodeSpatialIndex::from_nodes(graph.nodes());
        let request =
            request_between_points(&index, (1.0, 0.0), (19.0, 0.0), RoutingProfileKey::Walk)
                .expect("points should resolve to nearest graph nodes");
        assert_eq!(request.from, NodeId(0));
        assert_eq!(request.to, NodeId(2));
    }

    #[test]
    fn request_between_points_errors_on_empty_index() {
        let index = crate::routing::NodeSpatialIndex::default();
        assert_eq!(
            request_between_points(&index, (1.0, 0.0), (19.0, 0.0), RoutingProfileKey::Walk),
            Err(RoutingError::NoNearestNode)
        );
    }
}
