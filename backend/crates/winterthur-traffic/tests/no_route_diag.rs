//! Task 8 diagnostic: why do trips fail to route on the real net?
//!
//! Replays every unique weekday (origin_edge, dest_edge) pair from the real
//! `trips.bin` through the CH router and classifies the failures against
//! directed edge-graph reachability (forward/backward BFS over turn-backed
//! arcs + largest SCC). Run locally:
//! `cargo test -p winterthur-traffic --release --test no_route_diag -- --ignored --nocapture`

mod common;

use std::collections::HashMap;
use winterthur_traffic::Router;
use winterthur_traffic::demand::DayKind;

/// Per unique (origin_edge, dest_edge) pair: trip count, per-segment counts
/// `[internal, inbound, outbound, through]`, one representative lane pair.
type PairStats = (u64, [u64; 4], (u32, u32));

/// Directed edge-graph adjacency (forward and reverse), arcs backed by >=1
/// turn — the exact connectivity the router's CH is built from.
fn edge_adjacency(net: &traffic_net::TrafficNet) -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
    let n = net.edges.len();
    let mut fwd: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut bwd: Vec<Vec<u32>> = vec![Vec::new(); n];
    for t in &net.turns {
        let from_edge = net.lanes[t.from_lane as usize].edge;
        let to_edge = net.lanes[t.to_lane as usize].edge;
        if !fwd[from_edge as usize].contains(&to_edge) {
            fwd[from_edge as usize].push(to_edge);
            bwd[to_edge as usize].push(from_edge);
        }
    }
    (fwd, bwd)
}

fn bfs(adj: &[Vec<u32>], start: u32) -> Vec<bool> {
    let mut seen = vec![false; adj.len()];
    let mut queue = std::collections::VecDeque::new();
    seen[start as usize] = true;
    queue.push_back(start);
    while let Some(e) = queue.pop_front() {
        for &nx in &adj[e as usize] {
            if !seen[nx as usize] {
                seen[nx as usize] = true;
                queue.push_back(nx);
            }
        }
    }
    seen
}

/// Largest SCC of the edge graph by member count (Kosaraju, iterative).
fn largest_scc(fwd: &[Vec<u32>], bwd: &[Vec<u32>]) -> Vec<bool> {
    let n = fwd.len();
    // 1st pass: finish order on the forward graph.
    let mut order = Vec::with_capacity(n);
    let mut state = vec![0u8; n]; // 0 unseen, 1 in-progress, 2 done
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s] = 1;
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
    // 2nd pass: components on the reverse graph in reverse finish order.
    let mut comp = vec![u32::MAX; n];
    let mut best = (0u32, 0usize); // (comp id, size)
    let mut c = 0u32;
    for &s in order.iter().rev() {
        if comp[s] != u32::MAX {
            continue;
        }
        let mut size = 0usize;
        let mut queue = vec![s];
        comp[s] = c;
        while let Some(v) = queue.pop() {
            size += 1;
            for &nx in &bwd[v] {
                if comp[nx as usize] == u32::MAX {
                    comp[nx as usize] = c;
                    queue.push(nx as usize);
                }
            }
        }
        if size > best.1 {
            best = (c, size);
        }
        c += 1;
    }
    println!(
        "edge graph: {} edges, {} SCCs, largest SCC = {} edges",
        n, c, best.1
    );
    comp.iter().map(|&x| x == best.0).collect()
}

#[test]
#[ignore = "real net + real trips.bin diagnostic — run locally with -- --ignored --nocapture"]
fn classify_no_route_trip_pairs_on_real_net() {
    let json = common::load_real_net_json();
    let net = common::load_real_net(&json);
    let trips = common::load_real_trips(&json);
    let router = Router::new(&net);

    let (fwd, bwd) = edge_adjacency(&net);
    let in_scc = largest_scc(&fwd, &bwd);

    let gw_out = net.gateway_lanes_out();
    let gw_in = net.gateway_lanes_in();

    // Unique (origin_edge, dest_edge) pairs over the whole weekday block,
    // with trip counts and the segment mix.
    let all = trips.trips_in(DayKind::Workday, 0..86_400);
    let mut pairs: HashMap<(u32, u32), PairStats> = HashMap::new();
    for t in all {
        let oe = net.lanes[t.origin_lane as usize].edge;
        let de = net.lanes[t.dest_lane as usize].edge;
        let e = pairs
            .entry((oe, de))
            .or_insert((0, [0; 4], (t.origin_lane, t.dest_lane)));
        e.0 += 1;
        e.1[t.segment as usize] += 1;
    }
    println!(
        "weekday trips: {}, unique edge pairs: {}",
        all.len(),
        pairs.len()
    );

    let mut fail_trips = 0u64;
    let mut fail_by_segment = [0u64; 4];
    let mut fail_pairs: Vec<((u32, u32), PairStats)> = Vec::new();
    for (&(oe, de), &(n, seg, lanes)) in &pairs {
        if router.route(&net, oe, de).is_none() {
            fail_trips += n;
            for s in 0..4 {
                fail_by_segment[s] += seg[s];
            }
            fail_pairs.push(((oe, de), (n, seg, lanes)));
        }
    }
    fail_pairs.sort_by_key(|&(_, (n, _, _))| std::cmp::Reverse(n));
    let total: u64 = pairs.values().map(|v| v.0).sum();
    println!(
        "no-route: {} of {} trips ({:.1}%), {} unique pairs; by segment [int,in,out,thr] = {:?}",
        fail_trips,
        total,
        100.0 * fail_trips as f64 / total as f64,
        fail_pairs.len(),
        fail_by_segment
    );

    // Classify: is the origin edge / dest edge in the largest SCC? Can the
    // origin reach the SCC at all? Can the dest be reached from the SCC?
    let mut class_counts: HashMap<&'static str, (u64, u64)> = HashMap::new(); // (pairs, trips)
    for (i, &((oe, de), (n, seg, (ol, dl)))) in fail_pairs.iter().enumerate() {
        let o_scc = in_scc[oe as usize];
        let d_scc = in_scc[de as usize];
        let o_reach = bfs(&fwd, oe);
        let o_reaches_scc = o_reach.iter().zip(&in_scc).any(|(&r, &s)| r && s);
        let d_reach = bfs(&bwd, de);
        let d_reached_from_scc = d_reach.iter().zip(&in_scc).any(|(&r, &s)| r && s);
        let raw_reachable = o_reach[de as usize];
        let class = match (o_scc || o_reaches_scc, d_scc || d_reached_from_scc) {
            _ if raw_reachable => "reachable-but-router-failed",
            (false, true) => "origin-cannot-reach-core",
            (true, false) => "dest-not-reachable-from-core",
            (false, false) => "both-detached",
            (true, true) => "core-ok-but-disconnected-halves",
        };
        let e = class_counts.entry(class).or_insert((0, 0));
        e.0 += 1;
        e.1 += n;
        if i < 20 {
            let o_fwd_size = o_reach.iter().filter(|&&x| x).count();
            let d_bwd_size = d_reach.iter().filter(|&&x| x).count();
            println!(
                "#{i:2} pair edge {oe}->{de} lanes {ol}->{dl} trips={n} seg={seg:?} class={class} \
                 o_scc={o_scc} d_scc={d_scc} |fwd(o)|={o_fwd_size} |bwd(d)|={d_bwd_size} \
                 o_gw={} d_gw={} o_len={:.1} d_len={:.1}",
                gw_out.binary_search(&ol).is_ok(),
                gw_in.binary_search(&dl).is_ok(),
                net.lanes[ol as usize].length_m,
                net.lanes[dl as usize].length_m,
            );
        }
    }
    let mut classes: Vec<_> = class_counts.into_iter().collect();
    classes.sort_by_key(|&(_, (_, t))| std::cmp::Reverse(t));
    for (class, (pairs_n, trips_n)) in classes {
        println!("class {class}: {pairs_n} pairs, {trips_n} trips");
    }

    let rate = 100.0 * fail_trips as f64 / total as f64;
    assert!(
        rate < 2.0,
        "no-route rate {rate:.1}% exceeds the 2% target (Task 8)"
    );
}
