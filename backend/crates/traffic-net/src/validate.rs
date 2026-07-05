//! Fail-fast structural validation of a parsed [`TrafficNetDoc`]. Mirrors the
//! invariants asserted by `tests/geo/trafficnet.test.ts` (the vitest gate on
//! the baked JSON): dangling id references, lane length mismatch, and turn
//! coverage at controlled nodes. No healing, no defaulting — any violation is
//! a typed [`NetError`] and loading stops.

use crate::types::{NodeKind, TrafficNetDoc};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum NetError {
    #[error("failed to parse trafficnet JSON: {0}")]
    Parse(String),

    #[error("edge {edge} references unknown node {node} (as {field})")]
    DanglingNode {
        edge: u32,
        node: u32,
        field: &'static str,
    },

    #[error("edge {edge} references unknown lane {lane}")]
    DanglingLane { edge: u32, lane: u32 },

    #[error("lane {lane} references unknown edge {edge}")]
    LaneDanglingEdge { lane: u32, edge: u32 },

    #[error("turn {turn} references unknown lane {lane} (as {field})")]
    TurnDanglingLane {
        turn: u32,
        lane: u32,
        field: &'static str,
    },

    #[error("turn {turn} references unknown node {node}")]
    TurnDanglingNode { turn: u32, node: u32 },

    #[error("turn {turn} conflictsWith references unknown turn {other}")]
    TurnDanglingConflict { turn: u32, other: u32 },

    #[error("turn {turn} yieldsTo references unknown turn {other}")]
    TurnDanglingYield { turn: u32, other: u32 },

    #[error(
        "lane {lane} lengthM {declared} does not match its polyline length {actual} (tolerance {tolerance})"
    )]
    LaneLengthMismatch {
        lane: u32,
        declared: f32,
        actual: f32,
        tolerance: f32,
    },

    #[error("node {node} is {kind:?} with in/out edges but has no covering turn")]
    UncoveredTurnNode { node: u32, kind: NodeKind },

    #[error(
        "gateway node {node} has total degree {degree}, but gateway boundary stubs must have degree <= 2"
    )]
    GatewayDegreeExceeded { node: u32, degree: u32 },

    #[error("gateway node {node} has turn {turn}, but gateway boundary stubs must have no turns")]
    GatewayHasTurn { node: u32, turn: u32 },

    #[error(
        "signal node {node} phases do not exactly cover its incoming turns (gated {gated:?}, incoming {incoming:?})"
    )]
    SignalPhaseCoverageMismatch {
        node: u32,
        gated: Vec<u32>,
        incoming: Vec<u32>,
    },

    #[error("signal node {node} phases gate turn {turn} more than once")]
    SignalPhaseDuplicateTurn { node: u32, turn: u32 },
}

fn polyline_length(pts: &[[f32; 2]]) -> f32 {
    let mut d = 0.0f32;
    for w in pts.windows(2) {
        let dx = w[1][0] - w[0][0];
        let dy = w[1][1] - w[0][1];
        d += (dx * dx + dy * dy).sqrt();
    }
    d
}

pub fn validate(doc: &TrafficNetDoc) -> Result<(), NetError> {
    let node_ids: HashSet<u32> = doc.nodes.iter().map(|n| n.id).collect();
    let lane_ids: HashSet<u32> = doc.lanes.iter().map(|l| l.id).collect();
    let edge_ids: HashSet<u32> = doc.edges.iter().map(|e| e.id).collect();
    let turn_ids: HashSet<u32> = doc.turns.iter().map(|t| t.id).collect();

    // edges: from/to nodes exist, lane refs exist
    for e in &doc.edges {
        if !node_ids.contains(&e.from) {
            return Err(NetError::DanglingNode {
                edge: e.id,
                node: e.from,
                field: "from",
            });
        }
        if !node_ids.contains(&e.to) {
            return Err(NetError::DanglingNode {
                edge: e.id,
                node: e.to,
                field: "to",
            });
        }
        for &lid in &e.lanes {
            if !lane_ids.contains(&lid) {
                return Err(NetError::DanglingLane {
                    edge: e.id,
                    lane: lid,
                });
            }
        }
    }

    // lanes: edge ref exists, lengthM matches polyline within tolerance
    for l in &doc.lanes {
        if !edge_ids.contains(&l.edge) {
            return Err(NetError::LaneDanglingEdge {
                lane: l.id,
                edge: l.edge,
            });
        }
        let actual = polyline_length(&l.pts);
        let tolerance = (actual * 0.01).max(0.05);
        if (l.length_m - actual).abs() > tolerance {
            return Err(NetError::LaneLengthMismatch {
                lane: l.id,
                declared: l.length_m,
                actual,
                tolerance,
            });
        }
    }

    // turns: lane/node refs exist, conflictsWith/yieldsTo refs exist
    for t in &doc.turns {
        if !lane_ids.contains(&t.from_lane) {
            return Err(NetError::TurnDanglingLane {
                turn: t.id,
                lane: t.from_lane,
                field: "fromLane",
            });
        }
        if !lane_ids.contains(&t.to_lane) {
            return Err(NetError::TurnDanglingLane {
                turn: t.id,
                lane: t.to_lane,
                field: "toLane",
            });
        }
        if !node_ids.contains(&t.node) {
            return Err(NetError::TurnDanglingNode {
                turn: t.id,
                node: t.node,
            });
        }
        for &c in &t.conflicts_with {
            if !turn_ids.contains(&c) {
                return Err(NetError::TurnDanglingConflict {
                    turn: t.id,
                    other: c,
                });
            }
        }
        for &y in &t.yields_to {
            if !turn_ids.contains(&y) {
                return Err(NetError::TurnDanglingYield {
                    turn: t.id,
                    other: y,
                });
            }
        }
    }

    let mut in_count: HashMap<u32, u32> = HashMap::new();
    let mut out_count: HashMap<u32, u32> = HashMap::new();
    for e in &doc.edges {
        *out_count.entry(e.from).or_insert(0) += 1;
        *in_count.entry(e.to).or_insert(0) += 1;
    }

    // gateway nodes are boundary stubs: total degree <= 2 (at most one in-edge
    // and one out-edge from the cut two-way road) and never any turns — they
    // are pure demand sources/sinks, the network ends there.
    for n in &doc.nodes {
        if n.kind != NodeKind::Gateway {
            continue;
        }
        let degree =
            in_count.get(&n.id).copied().unwrap_or(0) + out_count.get(&n.id).copied().unwrap_or(0);
        if degree > 2 {
            return Err(NetError::GatewayDegreeExceeded { node: n.id, degree });
        }
        if let Some(t) = doc.turns.iter().find(|t| t.node == n.id) {
            return Err(NetError::GatewayHasTurn {
                node: n.id,
                turn: t.id,
            });
        }
    }

    // turn coverage: every non-dead_end, non-gateway node with >=1 in and
    // >=1 out edge has >=1 turn (gateways legitimately have in+out from a cut
    // two-way road but no turns — traffic never passes through the boundary).
    let turn_nodes: HashSet<u32> = doc.turns.iter().map(|t| t.node).collect();
    for n in &doc.nodes {
        if matches!(n.kind, NodeKind::DeadEnd | NodeKind::Gateway) {
            continue;
        }
        let has_in = in_count.get(&n.id).copied().unwrap_or(0) >= 1;
        let has_out = out_count.get(&n.id).copied().unwrap_or(0) >= 1;
        if has_in && has_out && !turn_nodes.contains(&n.id) {
            return Err(NetError::UncoveredTurnNode {
                node: n.id,
                kind: n.kind,
            });
        }
    }

    // signal phase coverage: gated turns == incoming turns at that node, exactly once
    for n in &doc.nodes {
        let Some(signal) = &n.signal else { continue };
        let incoming: HashSet<u32> = doc
            .turns
            .iter()
            .filter(|t| t.node == n.id)
            .map(|t| t.id)
            .collect();
        let mut gated: Vec<u32> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        for phase in &signal.phases {
            for &turn in &phase.turns {
                if !seen.insert(turn) {
                    return Err(NetError::SignalPhaseDuplicateTurn { node: n.id, turn });
                }
                gated.push(turn);
            }
        }
        let gated_set: HashSet<u32> = gated.iter().copied().collect();
        if gated_set != incoming {
            let mut gated_sorted: Vec<u32> = gated_set.into_iter().collect();
            gated_sorted.sort_unstable();
            let mut incoming_sorted: Vec<u32> = incoming.into_iter().collect();
            incoming_sorted.sort_unstable();
            return Err(NetError::SignalPhaseCoverageMismatch {
                node: n.id,
                gated: gated_sorted,
                incoming: incoming_sorted,
            });
        }
    }

    Ok(())
}
