use std::time::Duration;

use abutown_protocol::v1 as w;
use abutown_protocol::PROTOCOL_VERSION;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use prost::Message as _;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tower::ServiceExt;

use sim_server::{
    app::{build_app, build_app_with_runtime},
    runtime::SimulationRuntime,
};

fn runtime_with_seeded_mobility() -> SimulationRuntime {
    let mut runtime = SimulationRuntime::new();
    runtime.set_mobility_for_test(sim_core::mobility::seed::tiny_world());
    runtime
}

#[tokio::test]
async fn websocket_sends_hello_and_tile_pulse() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            build_app_with_runtime(runtime_with_seeded_mobility()),
        )
        .await
        .unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(
        hello.body,
        Some(w::server_message::Body::Hello(_))
    ));

    subscribe_to_seeded_chunks(&mut stream).await;

    // 10 Hz tick: the first tile pulse arrives within roughly one tick period.
    // Use 250 ms to absorb scheduler jitter on slow CI without weakening intent.
    let first_pulse = tokio::time::timeout(
        Duration::from_millis(250),
        read_next_tile_pulse(&mut stream),
    )
    .await
    .expect("first tile pulse must arrive within one tick window");
    assert_eq!(first_pulse.world_id, "abutown-main");
    let coord = first_pulse.coord.as_ref().expect("coord");
    assert_eq!(coord.x, 4);
    assert_eq!(coord.y, 4);
    assert_eq!(first_pulse.tick, 1);
    assert_eq!(first_pulse.version, 1);
    assert!(first_pulse.local_index < 1024);

    // Next tick arrives within one tick window.
    let next_pulse = tokio::time::timeout(
        Duration::from_millis(250),
        read_next_tile_pulse(&mut stream),
    )
    .await
    .expect("next tile pulse arrives within one tick window");
    assert_eq!(next_pulse.tick, 2);

    server.abort();
}

#[tokio::test]
async fn websocket_pulses_rotate_loaded_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(
        hello.body,
        Some(w::server_message::Body::Hello(_))
    ));

    let first_delta = read_next_tile_pulse(&mut stream).await;
    let second_delta = read_next_tile_pulse(&mut stream).await;
    let third_delta = read_next_tile_pulse(&mut stream).await;

    assert_eq!(first_delta.coord, Some(w::ChunkCoord { x: 4, y: 4 }));
    assert_eq!(second_delta.coord, Some(w::ChunkCoord { x: 5, y: 4 }));
    assert_eq!(third_delta.coord, Some(w::ChunkCoord { x: 4, y: 5 }));

    server.abort();
}

#[tokio::test]
async fn websocket_clients_receive_the_same_broadcast_tick() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut first_stream, _) = connect_async(url.clone()).await.unwrap();
    let (mut second_stream, _) = connect_async(url).await.unwrap();

    let first_hello = read_server_message(&mut first_stream).await;
    let second_hello = read_server_message(&mut second_stream).await;
    assert!(matches!(
        first_hello.body,
        Some(w::server_message::Body::Hello(_))
    ));
    assert!(matches!(
        second_hello.body,
        Some(w::server_message::Body::Hello(_))
    ));

    let first_delta = read_next_tile_pulse(&mut first_stream).await;
    let second_delta = read_next_tile_pulse(&mut second_stream).await;

    assert_eq!(second_delta.tick, first_delta.tick);
    assert_eq!(second_delta.version, first_delta.version);
    assert_eq!(second_delta.local_index, first_delta.local_index);

    server.abort();
}

#[tokio::test]
async fn websocket_sends_mobility_snapshots_after_subscribe() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            build_app_with_runtime(runtime_with_seeded_mobility()),
        )
        .await
        .unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(
        hello.body,
        Some(w::server_message::Body::Hello(_))
    ));

    subscribe_to_seeded_chunks(&mut stream).await;

    // Subscribe emits one MobilityChunkSnapshot per chunk. Collect all three.
    let mut snapshots: Vec<w::MobilityChunkSnapshot> = Vec::new();
    while snapshots.len() < 3 {
        let msg = read_server_message(&mut stream).await;
        if let Some(w::server_message::Body::MobilityChunkSnapshot(snap)) = msg.body {
            assert_eq!(snap.world_id, "abutown-main");
            snapshots.push(snap);
        }
    }
    // At least one snapshot must carry agents (tiny_world has walking agents in these chunks).
    let total_agents: usize = snapshots.iter().map(|s| s.agents.len()).sum();
    assert!(
        total_agents > 0,
        "at least one chunk snapshot must include agents in the subscribed chunks"
    );

    server.abort();
}

#[tokio::test]
async fn websocket_broadcasts_accepted_command_event() {
    let app = build_app();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(
        hello.body,
        Some(w::server_message::Body::Hello(_))
    ));

    let command = w::ClientCommand {
        command: Some(w::client_command::Command::SetTileKind(
            w::SetTileKindCommand {
                protocol_version: u32::from(PROTOCOL_VERSION),
                world_id: "abutown-main".to_string(),
                command_id: "command:ws:1".to_string(),
                coord: Some(w::ChunkCoord { x: 4, y: 4 }),
                local_index: 12,
                kind: w::TileKind::BuildingFootprint as i32,
            },
        )),
    };

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/x-protobuf")
                .body(Body::from(command.encode_to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.into_body().collect().await.unwrap();

    loop {
        let message = read_server_message(&mut stream).await;
        if let Some(w::server_message::Body::WorldEvent(event)) = message.body
            && let Some(w::world_event::Event::TileKindSet(tk)) = event.event
        {
            assert_eq!(tk.command_id, "command:ws:1");
            assert_eq!(tk.coord, Some(w::ChunkCoord { x: 4, y: 4 }));
            assert_eq!(tk.local_index, 12);
            assert_eq!(tk.kind, w::TileKind::BuildingFootprint as i32);
            break;
        }
    }

    server.abort();
}

#[tokio::test]
async fn websocket_does_not_broadcast_failed_command_append() {
    let app = build_app_with_runtime(SimulationRuntime::new_with_event_store(Box::new(
        sim_core::events::FailingWorldEventStore::new("database offline"),
    )));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.unwrap();
    });

    let (mut websocket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
    let _hello = read_server_message(&mut websocket).await;

    let command = w::ClientCommand {
        command: Some(w::client_command::Command::SetTileKind(
            w::SetTileKindCommand {
                protocol_version: u32::from(PROTOCOL_VERSION),
                world_id: "abutown-main".to_string(),
                command_id: "command:ws:store-failure".to_string(),
                coord: Some(w::ChunkCoord { x: 4, y: 4 }),
                local_index: 11,
                kind: w::TileKind::Water as i32,
            },
        )),
    };

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/x-protobuf")
                .body(Body::from(command.encode_to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Drain any background messages (tile pulses, mobility deltas) for one tick
    // and assert that none of them are a WorldEvent — that would indicate the rejected
    // command was broadcast despite the store failure.
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    loop {
        let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, websocket.next()).await {
            Err(_) => break,   // window expired
            Ok(None) => break, // stream closed
            Ok(Some(message)) => {
                let bytes = match message.expect("ws message") {
                    tokio_tungstenite::tungstenite::Message::Binary(b) => b,
                    _ => continue,
                };
                let parsed = w::ServerMessage::decode(bytes.as_ref()).expect("server message");
                assert!(
                    !matches!(parsed.body, Some(w::server_message::Body::WorldEvent(_))),
                    "failed command must not broadcast a WorldEvent, got: {parsed:?}"
                );
            }
        }
    }

    server.abort();
}

async fn read_server_message<S>(stream: &mut S) -> w::ServerMessage
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = message {
            return w::ServerMessage::decode(bytes.as_ref()).expect("decode server message");
        }
        // ignore Ping/Pong/Close/Text noise
    }
}

async fn read_next_tile_pulse<S>(stream: &mut S) -> w::TilePulse
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    loop {
        let message = read_server_message(stream).await;
        if let Some(w::server_message::Body::TilePulse(delta)) = message.body {
            return delta;
        }
    }
}

async fn read_next_chunk_snapshot<S>(stream: &mut S) -> w::MobilityChunkSnapshot
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    loop {
        let message = read_server_message(stream).await;
        if let Some(w::server_message::Body::MobilityChunkSnapshot(snap)) = message.body {
            return snap;
        }
    }
}

async fn subscribe_to_seeded_chunks<S>(stream: &mut S)
where
    S: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
    <S as futures_util::Sink<tokio_tungstenite::tungstenite::Message>>::Error: std::fmt::Debug,
{
    let subscribe = w::ClientMessage {
        body: Some(w::client_message::Body::ChunkSubscribe(w::ChunkSubscribe {
            protocol_version: u32::from(PROTOCOL_VERSION),
            coords: vec![
                w::ChunkCoord { x: 4, y: 4 },
                w::ChunkCoord { x: 5, y: 4 },
                w::ChunkCoord { x: 4, y: 5 },
            ],
        })),
    };
    let bytes = subscribe.encode_to_vec();
    stream
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            bytes.into(),
        ))
        .await
        .expect("send subscribe");
}

async fn send_chunk_subscribe<S>(stream: &mut S, coords: &[w::ChunkCoord])
where
    S: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
    <S as futures_util::Sink<tokio_tungstenite::tungstenite::Message>>::Error: std::fmt::Debug,
{
    let subscribe = w::ClientMessage {
        body: Some(w::client_message::Body::ChunkSubscribe(w::ChunkSubscribe {
            protocol_version: u32::from(PROTOCOL_VERSION),
            coords: coords.to_vec(),
        })),
    };
    let bytes = subscribe.encode_to_vec();
    stream
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            bytes.into(),
        ))
        .await
        .expect("send chunk subscribe");
}

#[tokio::test]
async fn chunk_subscribe_emits_chunk_snapshot_frame() {
    let runtime = SimulationRuntime::new();
    let app = build_app_with_runtime(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("ws://{}/ws", addr);
    let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    // Drain the Hello frame.
    let _ = client.next().await.unwrap().unwrap();

    send_chunk_subscribe(&mut client, &[w::ChunkCoord { x: 4, y: 4 }]).await;

    let mut got_snapshot = false;
    for _ in 0..10 {
        let msg = client.next().await.unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg
            && let Ok(parsed) = w::ServerMessage::decode(bytes.as_ref())
            && let Some(w::server_message::Body::MobilityChunkSnapshot(snap)) = parsed.body
        {
            let coord = snap.chunk.as_ref().expect("chunk coord present");
            assert_eq!(coord.x, 4);
            assert_eq!(coord.y, 4);
            got_snapshot = true;
            break;
        }
    }
    assert!(
        got_snapshot,
        "subscribe should emit a MobilityChunkSnapshot for the new chunk"
    );
}

#[tokio::test]
async fn two_clients_with_different_subscriptions_see_different_entities() {
    // tiny_world places agents on link:walk:default which spans chunk_center(4,4)
    // to chunk_center(5,4).  Agents with progress < 0.5 land in chunk (4,4),
    // agents with progress >= 0.5 land in chunk (5,4), giving two disjoint sets.
    let app = build_app_with_runtime(runtime_with_seeded_mobility());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{addr}/ws");

    // Client A subscribes only to chunk (4,4).
    let (mut client_a, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_a).await; // drain hello
    send_chunk_subscribe(&mut client_a, &[w::ChunkCoord { x: 4, y: 4 }]).await;

    // Client B subscribes only to chunk (5,4).
    let (mut client_b, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_b).await; // drain hello
    send_chunk_subscribe(&mut client_b, &[w::ChunkCoord { x: 5, y: 4 }]).await;

    // Subscribe emits one MobilityChunkSnapshot per subscribed chunk — read it.
    let snap_a = read_next_chunk_snapshot(&mut client_a).await;
    let snap_b = read_next_chunk_snapshot(&mut client_b).await;

    let ids_a: std::collections::HashSet<String> = snap_a
        .agents
        .iter()
        .map(|a| a.id.clone())
        .chain(snap_a.vehicles.iter().map(|v| v.id.clone()))
        .collect();
    let ids_b: std::collections::HashSet<String> = snap_b
        .agents
        .iter()
        .map(|a| a.id.clone())
        .chain(snap_b.vehicles.iter().map(|v| v.id.clone()))
        .collect();

    // Each client must receive at least one entity — otherwise the test is vacuous.
    assert!(
        !ids_a.is_empty(),
        "client A should see entities in chunk (4,4)"
    );
    assert!(
        !ids_b.is_empty(),
        "client B should see entities in chunk (5,4)"
    );

    // Per-chunk snapshots carry only entities in that chunk — sets are disjoint by construction.
    assert!(
        ids_a.intersection(&ids_b).next().is_none(),
        "client A and client B should see disjoint entity sets (A={ids_a:?}, B={ids_b:?})",
    );
}

#[tokio::test]
async fn three_clients_with_disjoint_subscriptions_see_only_their_chunks() {
    // tiny_world places 20 walking agents on link:walk:default (chunk_center(4,4)
    // → chunk_center(5,4)) and 4 tram vehicles split across horizontal and
    // vertical routes. Progress < 0.5 → chunk (4,4); progress >= 0.5 → chunk
    // (5,4) for horizontal/walk entities.  The vertical route runs from
    // chunk_center(4,4) to chunk_center(4,5), so vehicle:seed:3 (progress 0.75)
    // lands in chunk (4,5). Three fully disjoint entity sets — one per chunk.
    //
    // This test exercises the per-chunk channel architecture: each client
    // subscribes to a distinct chunk and receives only entities in that chunk.
    let app = build_app_with_runtime(runtime_with_seeded_mobility());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{addr}/ws");

    // Client A subscribes only to chunk (4,4).
    let (mut client_a, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_a).await; // drain hello
    send_chunk_subscribe(&mut client_a, &[w::ChunkCoord { x: 4, y: 4 }]).await;

    // Client B subscribes only to chunk (5,4).
    let (mut client_b, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_b).await; // drain hello
    send_chunk_subscribe(&mut client_b, &[w::ChunkCoord { x: 5, y: 4 }]).await;

    // Client C subscribes only to chunk (4,5) — vertical-route vehicles end here.
    let (mut client_c, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_c).await; // drain hello
    send_chunk_subscribe(&mut client_c, &[w::ChunkCoord { x: 4, y: 5 }]).await;

    // Subscribe emits one MobilityChunkSnapshot per subscribed chunk — read one per client.
    let snap_a = read_next_chunk_snapshot(&mut client_a).await;
    let snap_b = read_next_chunk_snapshot(&mut client_b).await;
    let snap_c = read_next_chunk_snapshot(&mut client_c).await;

    let ids_a: std::collections::HashSet<String> = snap_a
        .agents
        .iter()
        .map(|a| a.id.clone())
        .chain(snap_a.vehicles.iter().map(|v| v.id.clone()))
        .collect();
    let ids_b: std::collections::HashSet<String> = snap_b
        .agents
        .iter()
        .map(|a| a.id.clone())
        .chain(snap_b.vehicles.iter().map(|v| v.id.clone()))
        .collect();
    let ids_c: std::collections::HashSet<String> = snap_c
        .agents
        .iter()
        .map(|a| a.id.clone())
        .chain(snap_c.vehicles.iter().map(|v| v.id.clone()))
        .collect();

    // Each client must receive at least one entity — otherwise the test is vacuous.
    assert!(
        !ids_a.is_empty(),
        "client A should see entities in chunk (4,4)"
    );
    assert!(
        !ids_b.is_empty(),
        "client B should see entities in chunk (5,4)"
    );
    assert!(
        !ids_c.is_empty(),
        "client C should see entities in chunk (4,5)"
    );

    // Per-chunk snapshots carry only entities in that chunk — sets are disjoint by construction.
    assert!(
        ids_a.intersection(&ids_b).next().is_none(),
        "client A and client B should see disjoint entity sets (A={ids_a:?}, B={ids_b:?})",
    );
    assert!(
        ids_a.intersection(&ids_c).next().is_none(),
        "client A and client C should see disjoint entity sets (A={ids_a:?}, C={ids_c:?})",
    );
    assert!(
        ids_b.intersection(&ids_c).next().is_none(),
        "client B and client C should see disjoint entity sets (B={ids_b:?}, C={ids_c:?})",
    );
}

#[tokio::test]
async fn subscribed_chunk_receives_mobility_chunk_delta_each_tick() {
    // tiny_world agents walk on link:walk:default whose geometry (from the
    // hardcoded mobility_geometry fallback) runs chunk_center(4,4) →
    // chunk_center(5,4).  However, the ECS Position component starts at (0,0)
    // because compute_world_coord_system only runs for Active/Hot chunks and
    // uses the registered link_polylines ECS resource — not the fallback.
    //
    // Workaround: subscribe to chunk (0,0) in addition to (4,4).
    //   • (0,0) becomes Active → advance_agents_system runs on agents at
    //     Position(0,0) → marks them dirty.
    //   • tick_mobility computes their world coord via the fallback →
    //     chunk (4,4) → delta map entry for (4,4).
    //   • chunk_channels has a sender for (4,4) (because the client subscribed
    //     to it) → delta forwarded to the client.
    //
    // The test therefore asserts that ANY MobilityChunkDelta (not necessarily
    // from exactly (4,4)) arrives, confirming the fan-out pipeline is wired.
    let runtime = runtime_with_seeded_mobility();
    let app = build_app_with_runtime(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("ws://{}/ws", addr);
    let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let _ = client.next().await.unwrap().unwrap(); // hello

    // Subscribe only to (4,4), the chunk where seeded tiny_world agents live.
    // (The previous chunk(0,0) workaround was needed because seeded agents had
    // default Position(0,0), which caused LOD to mass-demote them to a single
    // chunk before they ever ticked. Spawn-time Position init makes that
    // workaround unnecessary.)
    send_chunk_subscribe(&mut client, &[w::ChunkCoord { x: 4, y: 4 }]).await;

    let mut snapshot_seen = false;
    let mut delta_seen = false;
    for _ in 0..60 {
        let msg = tokio::time::timeout(Duration::from_secs(1), client.next())
            .await
            .expect("message should arrive within 1s")
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg
            && let Ok(parsed) = w::ServerMessage::decode(bytes.as_ref())
        {
            match parsed.body {
                Some(w::server_message::Body::MobilityChunkSnapshot(_)) => {
                    snapshot_seen = true;
                }
                Some(w::server_message::Body::MobilityChunkDelta(_)) => {
                    delta_seen = true;
                    break;
                }
                _ => {}
            }
        }
    }
    assert!(snapshot_seen, "snapshot should arrive on subscribe");
    assert!(
        delta_seen,
        "per-tick MobilityChunkDelta should arrive within 60 messages"
    );
}
