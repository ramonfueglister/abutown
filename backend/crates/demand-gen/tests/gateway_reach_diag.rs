//! Task 8 diagnostic: which gateways lose a spawn/sink direction under the
//! router-reachability rule, and why. Run locally:
//! `cargo test -p demand-gen --test gateway_reach_diag -- --ignored --nocapture`

use demand_gen::{gateways, reach};

#[test]
#[ignore = "needs the committed real trafficnet.json — diagnostic, run locally"]
fn report_gateway_reachability_on_real_net() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let json = std::fs::read_to_string(root.join("data/winterthur/trafficnet.json")).unwrap();
    let net = traffic_net::load(&json).unwrap();
    let r = reach::analyze(&net);

    let core_edges = r.core.iter().filter(|&&c| c).count();
    println!("edges={} core={}", net.edges.len(), core_edges);

    for &node_id in net.gateways() {
        for e in &net.edges {
            if e.from != node_id && e.to != node_id {
                continue;
            }
            let dir = if e.from == node_id { "spawn" } else { "sink " };
            let ok = if e.from == node_id {
                r.reaches_core[e.id as usize]
            } else {
                r.from_core[e.id as usize]
            };
            if !ok {
                println!(
                    "gateway {node_id}: {dir} edge {} speed {:.1} NOT core-connected \
                     (core={} reaches={} from={})",
                    e.id,
                    e.speed_ms,
                    r.core[e.id as usize],
                    r.reaches_core[e.id as usize],
                    r.from_core[e.id as usize],
                );
            }
        }
    }
    let infos = gateways::gateway_infos(&net, &r);
    let no_spawn = infos.iter().filter(|g| g.spawn_lane.is_none()).count();
    let no_sink = infos.iter().filter(|g| g.sink_lane.is_none()).count();
    println!(
        "gateways={} without spawn={} without sink={}",
        infos.len(),
        no_spawn,
        no_sink
    );
}
