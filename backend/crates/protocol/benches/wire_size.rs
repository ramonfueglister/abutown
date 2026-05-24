use abutown_protocol::v1::*;
use criterion::{Criterion, criterion_group, criterion_main};
use prost::Message;

fn sample_delta_proto(n_agents: usize) -> ServerMessage {
    let agents: Vec<AgentMobility> = (0..n_agents)
        .map(|i| AgentMobility {
            id: format!("agent:walker:{i}"),
            state: Some(AgentState {
                state: Some(agent_state::State::Walking(Walking {
                    link_id: "link:walk:corridor:22".into(),
                    progress: (i as f32) / (n_agents.max(1) as f32),
                })),
            }),
            world_coord: Some(WorldCoord {
                x: 92.0 + i as f32,
                y: 133.0,
            }),
            direction: Direction::E as i32,
            sprite_key: format!("pedestrian:{}", i % 16),
            plan_cursor: 0,
        })
        .collect();

    ServerMessage {
        body: Some(server_message::Body::MobilityChunkDelta(
            MobilityChunkDelta {
                protocol_version: 16,
                world_id: "abutown-main".into(),
                tick: 1234,
                chunk: Some(ChunkCoord { x: 4, y: 4 }),
                changed_agents: agents,
                changed_vehicles: vec![],
                left_agents: vec![],
                left_vehicles: vec![],
            },
        )),
    }
}

fn json_equivalent_for_delta(n_agents: usize) -> String {
    let agents: Vec<serde_json::Value> = (0..n_agents)
        .map(|i| {
            serde_json::json!({
                "id": format!("agent:walker:{i}"),
                "state": {
                    "type": "walking",
                    "link_id": "link:walk:corridor:22",
                    "progress": (i as f32) / (n_agents.max(1) as f32),
                },
                "plan_cursor": 0,
                "world_coord": { "x": 92.0 + i as f32, "y": 133.0 },
                "direction": "e",
                "sprite_key": format!("pedestrian:{}", i % 16),
            })
        })
        .collect();
    let payload = serde_json::json!({
        "type": "mobility_chunk_delta",
        "protocol_version": 16,
        "world_id": "abutown-main",
        "tick": 1234,
        "chunk": { "x": 4, "y": 4 },
        "changed_agents": agents,
        "changed_vehicles": [],
        "left_agents": [],
        "left_vehicles": [],
    });
    serde_json::to_string(&payload).unwrap()
}

fn bench_wire_size(c: &mut Criterion) {
    println!("\n=== Wire-size comparison: MobilityChunkDelta with N walking agents ===");
    println!(
        "{:>8} | {:>10} | {:>10} | {:>8}",
        "N agents", "JSON bytes", "Proto bytes", "ratio"
    );
    println!("{:->8} | {:->10} | {:->10} | {:->8}", "", "", "", "");
    for &n in &[1usize, 10, 50, 100] {
        let proto = sample_delta_proto(n);
        let proto_bytes = proto.encode_to_vec();
        let json_str = json_equivalent_for_delta(n);
        let ratio = json_str.len() as f64 / proto_bytes.len() as f64;
        println!(
            "{n:>8} | {:>10} | {:>10} | {:>6.2}x",
            json_str.len(),
            proto_bytes.len(),
            ratio
        );
    }
    println!();

    c.bench_function("encode_50_agent_delta", |b| {
        let msg = sample_delta_proto(50);
        b.iter(|| {
            let _ = msg.encode_to_vec();
        });
    });
}

criterion_group!(benches, bench_wire_size);
criterion_main!(benches);
