//! Router-reachability over the directed edge-turn graph (plan Task 8).
//!
//! The Task-1 bake prunes by **undirected** component, so the committed net
//! legitimately keeps one-way fragments (service loops, split carriageway
//! stubs) that the runtime CH router — whose arcs are turn-backed and
//! *directed* — can never leave (`|forward reach| = 1`) or never enter
//! (`|backward reach| = 1`). A demand endpoint snapped onto such a fragment
//! produces a trip the router must refuse (measured: 12 % of weekday trips,
//! 43 % of the 07:30 peak, before this module existed).
//!
//! The fix is at the demand layer: compute the **routable core** — the
//! largest strongly-connected component of the edge graph, measured by total
//! lane length, exactly mirroring the bake's "largest component by lane
//! length" rule but on the *directed* graph — and only snap zones onto core
//! lanes. Gateway stubs are directional by construction and never inside the
//! SCC, so for them the relaxed conditions apply: a spawn lane must *reach*
//! the core, a sink lane must be *reachable from* the core. Any
//! (core-or-gateway) origin can then route to any (core-or-gateway)
//! destination by construction.

use traffic_net::TrafficNet;

/// Per-edge routability classification. All vectors are indexed by edge id
/// (dense array indices on a validated net).
pub struct Reach {
    /// Edge is in the routable core (largest SCC by total lane length).
    pub core: Vec<bool>,
    /// Edge can reach the core via directed turn-backed arcs (core included).
    pub reaches_core: Vec<bool>,
    /// Edge is reachable from the core (core included).
    pub from_core: Vec<bool>,
    /// Forward adjacency (edge -> successor edges), for pair queries.
    fwd: Vec<Vec<u32>>,
}

impl Reach {
    /// Directed pair reachability `from -> to` over the edge graph — for
    /// validating authored through pairs, which may legitimately live on a
    /// motorway segment detached from the core (e.g. a boundary-to-boundary
    /// A1 chunk with no ramps inside the Gemeinde).
    pub fn edge_reaches(&self, from: u32, to: u32) -> bool {
        let mut seen = vec![false; self.fwd.len()];
        let mut queue = std::collections::VecDeque::new();
        seen[from as usize] = true;
        queue.push_back(from);
        while let Some(v) = queue.pop_front() {
            if v == to {
                return true;
            }
            for &nx in &self.fwd[v as usize] {
                if !seen[nx as usize] {
                    seen[nx as usize] = true;
                    queue.push_back(nx);
                }
            }
        }
        false
    }
}

/// Analyze `net`'s directed edge graph: one node per edge, an arc `A -> B`
/// iff at least one turn connects a lane of `A` to a lane of `B` — the exact
/// connectivity the runtime `Router` builds its CH from.
pub fn analyze(net: &TrafficNet) -> Reach {
    let n = net.edges.len();
    let mut arcs: Vec<(u32, u32)> = net
        .turns
        .iter()
        .map(|t| {
            (
                net.lanes[t.from_lane as usize].edge,
                net.lanes[t.to_lane as usize].edge,
            )
        })
        .collect();
    arcs.sort_unstable();
    arcs.dedup();
    let lane_len: Vec<f64> = net
        .edges
        .iter()
        .map(|e| {
            e.lanes
                .iter()
                .map(|&l| net.lanes[l as usize].length_m as f64)
                .sum()
        })
        .collect();
    analyze_arcs(n, &arcs, &lane_len)
}

/// Core computation on a plain arc list (unit-testable without a net).
pub fn analyze_arcs(n: usize, arcs: &[(u32, u32)], lane_len: &[f64]) -> Reach {
    let mut fwd: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut bwd: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &(a, b) in arcs {
        fwd[a as usize].push(b);
        bwd[b as usize].push(a);
    }

    let comp = kosaraju(&fwd, &bwd);
    // Largest component by total lane length; ties break on the lower
    // component id (deterministic — Kosaraju's order is fixed by edge ids).
    let n_comp = comp.iter().map(|&c| c + 1).max().unwrap_or(0) as usize;
    let mut comp_len = vec![0.0f64; n_comp];
    for (e, &c) in comp.iter().enumerate() {
        comp_len[c as usize] += lane_len[e];
    }
    let best = comp_len
        .iter()
        .enumerate()
        .max_by(|(ia, a), (ib, b)| a.partial_cmp(b).expect("finite").then(ib.cmp(ia)))
        .map(|(i, _)| i as u32)
        .unwrap_or(0);
    let core: Vec<bool> = comp.iter().map(|&c| c == best).collect();

    // Edges that can reach the core = multi-source BFS from the core over
    // the *reverse* graph; edges reachable from the core = over the forward.
    let reaches_core = multi_bfs(&bwd, &core);
    let from_core = multi_bfs(&fwd, &core);
    Reach {
        core,
        reaches_core,
        from_core,
        fwd,
    }
}

/// Multi-source BFS: every node of `seeds` plus everything reachable from
/// them along `adj`.
fn multi_bfs(adj: &[Vec<u32>], seeds: &[bool]) -> Vec<bool> {
    let mut seen = seeds.to_vec();
    let mut queue: std::collections::VecDeque<u32> = seeds
        .iter()
        .enumerate()
        .filter(|&(_, &s)| s)
        .map(|(i, _)| i as u32)
        .collect();
    while let Some(v) = queue.pop_front() {
        for &nx in &adj[v as usize] {
            if !seen[nx as usize] {
                seen[nx as usize] = true;
                queue.push_back(nx);
            }
        }
    }
    seen
}

/// Iterative Kosaraju: component id per node, ids assigned in reverse finish
/// order (deterministic given the deterministic adjacency order).
fn kosaraju(fwd: &[Vec<u32>], bwd: &[Vec<u32>]) -> Vec<u32> {
    let n = fwd.len();
    let mut order = Vec::with_capacity(n);
    let mut state = vec![0u8; n]; // 0 unseen, 1 in progress, 2 done
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        state[s] = 1;
        let mut stack = vec![(s, 0usize)];
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            if *i < fwd[v].len() {
                let nx = fwd[v][*i] as usize;
                *i += 1;
                if state[nx] == 0 {
                    state[nx] = 1;
                    stack.push((nx, 0));
                }
            } else {
                state[v] = 2;
                order.push(v);
                stack.pop();
            }
        }
    }
    let mut comp = vec![u32::MAX; n];
    let mut c = 0u32;
    for &s in order.iter().rev() {
        if comp[s] != u32::MAX {
            continue;
        }
        comp[s] = c;
        let mut queue = vec![s];
        while let Some(v) = queue.pop() {
            for &nx in &bwd[v] {
                if comp[nx as usize] == u32::MAX {
                    comp[nx as usize] = c;
                    queue.push(nx as usize);
                }
            }
        }
        c += 1;
    }
    comp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Graph: 0 <-> 1 form the big cycle (long lanes); 2 is an exit-only
    /// stub (1 -> 2, no way back); 3 is an entry-only stub (3 -> 0); 4 is
    /// fully detached.
    fn toy() -> Reach {
        let arcs = [(0, 1), (1, 0), (1, 2), (3, 0)];
        let lane_len = [100.0, 100.0, 10.0, 10.0, 10.0];
        analyze_arcs(5, &arcs, &lane_len)
    }

    #[test]
    fn core_is_the_cycle_only() {
        let r = toy();
        assert_eq!(r.core, vec![true, true, false, false, false]);
    }

    #[test]
    fn stubs_classify_by_direction() {
        let r = toy();
        // exit-only stub: reachable FROM the core, cannot reach it
        assert!(r.from_core[2] && !r.reaches_core[2]);
        // entry-only stub: reaches the core, not reachable from it
        assert!(r.reaches_core[3] && !r.from_core[3]);
        // detached: neither
        assert!(!r.from_core[4] && !r.reaches_core[4]);
        // core edges: both (core is included in both closures)
        assert!(r.reaches_core[0] && r.from_core[0]);
    }

    #[test]
    fn edge_reaches_is_directed_pair_reachability() {
        let r = toy();
        // core -> exit stub: yes; exit stub -> core: no; entry -> exit: yes
        assert!(r.edge_reaches(0, 2));
        assert!(!r.edge_reaches(2, 0));
        assert!(r.edge_reaches(3, 2));
        assert!(!r.edge_reaches(0, 4));
        assert!(r.edge_reaches(4, 4));
    }

    #[test]
    fn largest_scc_is_by_lane_length_not_node_count() {
        // Two 2-cycles: {0,1} short lanes, {2,3} one long lane — length wins.
        let arcs = [(0, 1), (1, 0), (2, 3), (3, 2)];
        let lane_len = [1.0, 1.0, 50.0, 1.0];
        let r = analyze_arcs(4, &arcs, &lane_len);
        assert_eq!(r.core, vec![false, false, true, true]);
    }
}
