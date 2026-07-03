//! Contraction-hierarchy (CH) route computation over the baked Winterthur
//! traffic network, with live weight updates.
//!
//! # Edge-graph modeling
//!
//! CH runs on the **edge graph**, not the lane graph: a CH node id is a
//! `traffic-net` edge id (as `usize`). A directed CH arc `A -> B` exists iff
//! at least one lane on edge `A` has a turn to at least one lane on edge `B`
//! (i.e. the edges are connected via >=1 turn). This keeps the CH graph small
//! (one node per edge, not per lane) while still respecting turn
//! restrictions, since an arc is only added when a real turn backs it.
//!
//! Arc weight = the travel time of edge `A` in milliseconds, as `usize`:
//! `round(lengthM / speedMs * 1000)`. Length is taken from `net.lanes[edge.lanes[0]].length_m`
//! — the first lane listed on the edge. In a well-formed net all lanes on an
//! edge have equal length (parallel lanes of the same road segment), so any
//! consistent choice works; we pick the first for simplicity and determinism.
//!
//! # Lane expansion
//!
//! `route()` first finds the shortest edge-id path via CH, then expands each
//! edge-to-edge hop into a concrete lane id: among all lanes on the current
//! edge that have >=1 turn reaching some lane on the next edge, pick the
//! **lowest lane id** that works. This matches the sim kernel's boundary
//! crossing, which matches at edge granularity (any lane on the current edge
//! with a turn to any lane on the next edge is drivable) and gives a
//! deterministic, reproducible expansion (no `HashMap` iteration order).
//!
//! If any hop has no lane->lane turn connecting the two edges, expansion
//! fails closed (`None`) rather than emitting an undrivable route.
//!
//! # Live weight updates
//!
//! `update_weights` records freshly measured per-edge travel times (seconds)
//! with MSA smoothing (`smoothed = 0.5*old + 0.5*new`) but does NOT touch the
//! prepared CH. `rebuild()` recomputes arc weights (ms) from the current
//! smoothed times and re-prepares the CH using the *same node ordering* via
//! `fast_paths::prepare_with_order`, which is cheap relative to a from-scratch
//! `prepare`. Callers decide the rebuild cadence (e.g. every 5 sim-minutes);
//! this module has no timer/cadence logic of its own.

use fast_paths::{FastGraph, InputGraph, NodeId};
use traffic_net::TrafficNet;

/// CH-backed router over the edge graph of a [`TrafficNet`].
pub struct Router {
    /// Directed arcs `(from_edge, to_edge)`, deduplicated, used to rebuild the
    /// `InputGraph` on `rebuild()`. Kept alongside the prepared graph since
    /// `fast_paths` does not expose an accessor to recover them.
    arcs: Vec<(usize, usize)>,
    /// Current smoothed per-edge travel time, in seconds, indexed by edge id.
    edge_time_s: Vec<f32>,
    /// Node ordering fixed at first `prepare()`, reused by every `rebuild()`
    /// via `prepare_with_order` for a cheap re-prepare.
    node_order: Vec<NodeId>,
    /// The prepared CH graph used for `calc_path` queries.
    graph: FastGraph,
    /// Number of edges (= number of CH nodes), for bounds checks.
    n_edges: usize,
}

/// Representative length (m) for `edge`: the length of the first lane listed
/// on it. See module doc for why any consistent per-edge choice is fine.
fn edge_length_m(net: &TrafficNet, edge_id: u32) -> f32 {
    let edge = &net.edges[edge_id as usize];
    let lane_id = edge.lanes[0];
    net.lanes[lane_id as usize].length_m
}

/// All distinct target edge ids reachable from `from_edge` via >=1 turn from
/// any of its lanes, in ascending order (dedup'd).
fn distinct_target_edges(net: &TrafficNet, from_edge: u32) -> Vec<u32> {
    let edge = &net.edges[from_edge as usize];
    let mut targets: Vec<u32> = Vec::new();
    for &lane in &edge.lanes {
        for &tid in net.turns_from(lane) {
            let to_lane = net.turns[tid as usize].to_lane;
            let to_edge = net.lanes[to_lane as usize].edge;
            if !targets.contains(&to_edge) {
                targets.push(to_edge);
            }
        }
    }
    targets.sort_unstable();
    targets
}

/// Among the lanes of edge `from_edge`, the lowest lane id that has >=1 turn
/// reaching some lane on edge `to_edge`. `None` if no such lane exists.
fn lowest_lane_to_edge(net: &TrafficNet, from_edge: u32, to_edge: u32) -> Option<u32> {
    let edge = &net.edges[from_edge as usize];
    let mut candidates: Vec<u32> = edge
        .lanes
        .iter()
        .copied()
        .filter(|&lane| {
            net.turns_from(lane)
                .iter()
                .any(|&tid| net.lanes[net.turns[tid as usize].to_lane as usize].edge == to_edge)
        })
        .collect();
    candidates.sort_unstable();
    candidates.into_iter().next()
}

impl Router {
    /// Build a CH router over `net`'s edge graph. Infallible: a validated
    /// `TrafficNet` always yields a well-formed (possibly disconnected) edge
    /// graph, and `fast_paths::prepare` handles disconnected graphs fine
    /// (unreachable pairs simply return `None` from `route`).
    pub fn new(net: &TrafficNet) -> Self {
        let n_edges = net.edges.len();
        let edge_time_s: Vec<f32> = net
            .edges
            .iter()
            .map(|e| edge_length_m(net, e.id) / e.speed_ms)
            .collect();

        let arcs = Self::build_arcs(net);
        let input_graph = Self::input_graph_from_arcs(&arcs, &edge_time_s, n_edges);
        // First prepare picks its own node ordering (no order to reuse yet);
        // stash that ordering so later `rebuild()`s can cheaply re-prepare
        // via `prepare_with_order` instead of recomputing an ordering.
        let graph = fast_paths::prepare(&input_graph);
        let node_order = fast_paths::get_node_ordering(&graph);

        Router {
            arcs,
            edge_time_s,
            node_order,
            graph,
            n_edges,
        }
    }

    /// All directed edge->edge arcs backed by >=1 turn, over the whole net.
    fn build_arcs(net: &TrafficNet) -> Vec<(usize, usize)> {
        let mut arcs = Vec::new();
        for edge in &net.edges {
            for to_edge in distinct_target_edges(net, edge.id) {
                arcs.push((edge.id as usize, to_edge as usize));
            }
        }
        arcs
    }

    /// Build a fresh, frozen `InputGraph` from `arcs` with weights derived
    /// from `edge_time_s` (seconds -> ms on the *source* edge of each arc).
    fn input_graph_from_arcs(
        arcs: &[(usize, usize)],
        edge_time_s: &[f32],
        n_edges: usize,
    ) -> InputGraph {
        let mut input_graph = InputGraph::new();
        for &(from, to) in arcs {
            // `edge_time_s[from]` is already a travel time in seconds (either
            // the initial length/speed derivation, or the MSA-smoothed
            // measured value); convert to whole ms, floored at 1 so
            // `fast_paths` (which rejects zero-weight edges) never sees 0.
            let weight_ms = ((edge_time_s[from] * 1000.0).round() as usize).max(1);
            input_graph.add_edge(from, to, weight_ms);
        }
        // Ensure every edge id 0..n_edges is a known node even if it has no
        // arcs (isolated / dead-end edges), so `route` bounds checks against
        // `n_edges` stay meaningful and fast_paths doesn't choke on gaps.
        for id in 0..n_edges {
            input_graph.add_edge(id, id, usize::MAX / 4);
        }
        input_graph.freeze();
        input_graph
    }

    /// Shortest edge-id path from `from_edge` to `to_edge` via the prepared
    /// CH, expanded into a concrete lane-id sequence per the module's
    /// lane-expansion rule. `None` if either edge is unknown, no path exists,
    /// or any hop can't be expanded into a drivable lane transition.
    pub fn route(&self, net: &TrafficNet, from_edge: u32, to_edge: u32) -> Option<Vec<u32>> {
        let from = from_edge as usize;
        let to = to_edge as usize;
        if from >= self.n_edges || to >= self.n_edges {
            return None;
        }

        let mut path_calc = fast_paths::create_calculator(&self.graph);
        let shortest = path_calc.calc_path(&self.graph, from, to)?;
        let edge_path: Vec<usize> = shortest.get_nodes().to_vec();

        // Self-loop hack from `input_graph_from_arcs` can make a same-edge
        // query return `[from]`; handle explicitly with the edge's first lane.
        if edge_path.len() == 1 {
            let only = edge_path[0] as u32;
            let edge = &net.edges[only as usize];
            return Some(vec![edge.lanes[0]]);
        }

        // One lane per edge in `edge_path`: for every edge except the last,
        // pick the lowest-id lane on that edge with a turn reaching the next
        // edge. The terminal edge contributes its own lowest lane id (no
        // "reaches next edge" constraint — there is no next edge).
        let mut lanes: Vec<u32> = Vec::with_capacity(edge_path.len());
        for w in edge_path.windows(2) {
            let cur_edge = w[0] as u32;
            let next_edge = w[1] as u32;
            let lane = lowest_lane_to_edge(net, cur_edge, next_edge)?;
            lanes.push(lane);
        }
        let last_edge = *edge_path.last().expect("edge_path has >= 2 elements here");
        let last_lane = net.edges[last_edge]
            .lanes
            .iter()
            .copied()
            .min()
            .expect("every edge in a validated TrafficNet has at least one lane");
        lanes.push(last_lane);

        Some(lanes)
    }

    /// Record newly measured per-edge travel times (seconds), MSA-smoothed
    /// (`0.5*old + 0.5*new`) into the stored `edge_time_s`. Does not touch
    /// the prepared CH — call [`Router::rebuild`] to apply.
    pub fn update_weights(&mut self, times_s: &[f32]) {
        for (edge_id, &new_t) in times_s.iter().enumerate() {
            if edge_id >= self.edge_time_s.len() {
                break;
            }
            if !new_t.is_finite() {
                continue;
            }
            let old = self.edge_time_s[edge_id];
            self.edge_time_s[edge_id] = 0.5 * old + 0.5 * new_t;
        }
    }

    /// Recompute arc weights (ms) from the current smoothed `edge_time_s` and
    /// re-prepare the CH using the node ordering fixed at construction (cheap
    /// relative to a from-scratch `prepare`).
    pub fn rebuild(&mut self) {
        let input_graph = Self::input_graph_from_arcs(&self.arcs, &self.edge_time_s, self.n_edges);
        self.graph = fast_paths::prepare_with_order(&input_graph, &self.node_order)
            .expect("node ordering must still cover the input graph's nodes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_fixture(path: &str) -> TrafficNet {
        let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        traffic_net::load(&json).expect("fixture must validate")
    }

    fn diamond_path() -> String {
        format!("{}/tests/fixtures/diamond.json", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn shortest_route_matches_hand_computed() {
        // diamond.json: edge0 (node0->1, shared entry) forks at node1 into
        // edge1 (node1->2, "upper", 100m @ 20 m/s = 5s) and edge2 (node1->3,
        // "lower", 100m @ 10 m/s = 10s); both merge via edge3/edge4 into
        // edge5 (node4->5). Upper is faster baseline, so CH must route
        // edge0 -> edge1 -> edge3 -> edge5.
        let net = load_fixture(&diamond_path());
        let router = Router::new(&net);

        let route = router
            .route(&net, 0, 5)
            .expect("route from edge0 to edge5 must exist");

        // Hand-computed lane path: lane0 (edge0) -> lane1 (edge1, upper) ->
        // lane3 (edge3) -> lane5 (edge5).
        assert_eq!(route, vec![0, 1, 3, 5]);

        for w in route.windows(2) {
            let turns = net.turns_from(w[0]);
            assert!(
                turns
                    .iter()
                    .any(|&tid| net.turns[tid as usize].to_lane == w[1]),
                "no turn from lane {} to lane {}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn route_avoids_penalized_edge_after_rebuild() {
        let net = load_fixture(&diamond_path());
        let mut router = Router::new(&net);

        // Baseline confirms the upper path is preferred (see test above).
        let baseline = router.route(&net, 0, 5).expect("baseline route exists");
        assert_eq!(baseline, vec![0, 1, 3, 5]);

        // Penalize the upper path's first hop (edge1) by 10x its travel
        // time, then rebuild. The route from edge0 to edge5 must now avoid
        // edge1 and go via the lower path (edge2 -> edge4).
        let mut times_s = vec![f32::NAN; net.edges.len()];
        let edge1_time_s = edge_length_m(&net, 1) / net.edges[1].speed_ms;
        times_s[1] = edge1_time_s * 10.0;
        router.update_weights(&times_s);
        router.rebuild();

        let after = router
            .route(&net, 0, 5)
            .expect("route must still exist after penalizing edge1");

        assert_eq!(
            after,
            vec![0, 2, 4, 5],
            "route must avoid penalized edge1 and use the lower path instead"
        );
        assert!(
            !after.contains(&1),
            "penalized edge1's lane must not appear in the route"
        );
    }

    #[test]
    fn route_unknown_edge_returns_none() {
        let net = load_fixture(&diamond_path());
        let router = Router::new(&net);
        assert_eq!(router.route(&net, 0, 999), None);
        assert_eq!(router.route(&net, 999, 0), None);
    }

    #[test]
    fn route_same_edge_returns_single_lane() {
        let net = load_fixture(&diamond_path());
        let router = Router::new(&net);
        let route = router.route(&net, 0, 0).expect("same-edge route");
        assert_eq!(route, vec![0]);
    }
}
