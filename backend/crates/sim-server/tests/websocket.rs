use std::time::Duration;

use abutown_protocol::PROTOCOL_VERSION;
use abutown_protocol::v1 as w;
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
    SimulationRuntime::new()
}

fn seeded_mobility_chunk() -> w::ChunkCoord {
    w::ChunkCoord { x: 0, y: 0 }
}

#[tokio::test]
async fn websocket_sends_hello() {
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

    // Subscribe emits one MobilityChunkSnapshot per subscribed chunk.
    let mut snapshots: Vec<w::MobilityChunkSnapshot> = Vec::new();
    while snapshots.is_empty() {
        let msg = read_server_message(&mut stream).await;
        if let Some(w::server_message::Body::MobilityChunkSnapshot(snap)) = msg.body {
            assert_eq!(snap.world_id, "abutopia");
            snapshots.push(snap);
        }
    }
    // At least one snapshot must carry agents from the authored base-world seed.
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
                world_id: "abutopia".to_string(),
                command_id: "command:ws:1".to_string(),
                coord: Some(w::ChunkCoord { x: 0, y: 0 }),
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
            assert_eq!(tk.coord, Some(w::ChunkCoord { x: 0, y: 0 }));
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
                world_id: "abutopia".to_string(),
                command_id: "command:ws:store-failure".to_string(),
                coord: Some(w::ChunkCoord { x: 0, y: 0 }),
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

    // Drain any background messages (mobility deltas, economy snapshots) for one tick
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

async fn read_economy_snapshot<S>(stream: &mut S) -> w::EconomySnapshot
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
        if let Some(w::server_message::Body::EconomySnapshot(snap)) = message.body {
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
            coords: vec![seeded_mobility_chunk()],
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

    send_chunk_subscribe(&mut client, &[w::ChunkCoord { x: 0, y: 0 }]).await;

    let mut got_snapshot = false;
    for _ in 0..10 {
        let msg = client.next().await.unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg
            && let Ok(parsed) = w::ServerMessage::decode(bytes.as_ref())
            && let Some(w::server_message::Body::MobilityChunkSnapshot(snap)) = parsed.body
        {
            let coord = snap.chunk.as_ref().expect("chunk coord present");
            assert_eq!(coord.x, 0);
            assert_eq!(coord.y, 0);
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
async fn two_clients_subscribed_to_abutopia_chunk_see_the_same_seeded_pedestrian() {
    let app = build_app_with_runtime(runtime_with_seeded_mobility());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{addr}/ws");

    let (mut client_a, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_a).await; // drain hello
    send_chunk_subscribe(&mut client_a, &[seeded_mobility_chunk()]).await;

    let (mut client_b, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_b).await; // drain hello
    send_chunk_subscribe(&mut client_b, &[seeded_mobility_chunk()]).await;

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

    assert!(
        !ids_a.is_empty(),
        "client A should see entities in the abutopia chunk"
    );
    assert!(
        !ids_b.is_empty(),
        "client B should see entities in the abutopia chunk"
    );
    assert_eq!(ids_a, ids_b);
}

#[tokio::test]
async fn three_clients_subscribed_to_abutopia_chunk_each_receive_snapshot() {
    let app = build_app_with_runtime(runtime_with_seeded_mobility());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{addr}/ws");

    let (mut client_a, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_a).await; // drain hello
    send_chunk_subscribe(&mut client_a, &[seeded_mobility_chunk()]).await;

    let (mut client_b, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_b).await; // drain hello
    send_chunk_subscribe(&mut client_b, &[seeded_mobility_chunk()]).await;

    let (mut client_c, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_c).await; // drain hello
    send_chunk_subscribe(&mut client_c, &[seeded_mobility_chunk()]).await;

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

    assert!(
        !ids_a.is_empty(),
        "client A should see entities in the abutopia chunk"
    );
    assert!(
        !ids_b.is_empty(),
        "client B should see entities in the abutopia chunk"
    );
    assert!(
        !ids_c.is_empty(),
        "client C should see entities in the abutopia chunk"
    );
    assert_eq!(ids_a, ids_b);
    assert_eq!(ids_a, ids_c);
}

#[tokio::test]
async fn websocket_clients_receive_the_same_broadcast_tick() {
    // Both clients must observe an EconomySnapshot for the same tick with
    // identical payload — proving that the global per-tick broadcast channel
    // delivers the same frame to every connected client.
    let app = build_app_with_runtime(runtime_with_seeded_mobility());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{addr}/ws");

    let (mut client_a, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_a).await; // drain Hello
    // Drain the immediate EconomySnapshot sent on connect.
    let _initial_a = read_economy_snapshot(&mut client_a).await;

    let (mut client_b, _) = connect_async(&url).await.unwrap();
    let _ = read_server_message(&mut client_b).await; // drain Hello
    // Drain the immediate EconomySnapshot sent on connect.
    let _initial_b = read_economy_snapshot(&mut client_b).await;

    // Client B subscribed later, so anchor on its first per-tick snapshot.
    // Client A subscribed earlier and is guaranteed to have received every
    // frame that B received, so scan A until we find the matching tick.
    let snap_b = read_economy_snapshot(&mut client_b).await;

    // Collect EconomySnapshots on client A until we reach or pass snap_b.tick.
    // Because A connected before B, A's stream is a superset of B's stream,
    // so snap_b.tick must appear in A's frames.
    let mut snaps_a: Vec<w::EconomySnapshot> = Vec::new();
    loop {
        let snap = read_economy_snapshot(&mut client_a).await;
        let tick = snap.tick;
        snaps_a.push(snap);
        if tick >= snap_b.tick {
            break;
        }
    }

    // At least one frame on client A must share the exact same tick as snap_b,
    // and its world_id, markets, and goods must be identical (same broadcast frame).
    let matching = snaps_a.iter().find(|s| s.tick == snap_b.tick);
    assert!(
        matching.is_some(),
        "client A must receive an EconomySnapshot for tick {} (got ticks: {:?})",
        snap_b.tick,
        snaps_a.iter().map(|s| s.tick).collect::<Vec<_>>()
    );
    let snap_a = matching.unwrap();
    assert_eq!(
        snap_b.world_id, snap_a.world_id,
        "world_id must match across clients for the same tick"
    );
    assert_eq!(
        snap_b.markets, snap_a.markets,
        "markets must be identical for the same broadcast tick"
    );
    assert_eq!(
        snap_b.goods, snap_a.goods,
        "goods must be identical for the same broadcast tick"
    );
}

#[tokio::test]
async fn subscribed_chunk_receives_mobility_chunk_delta_each_tick() {
    // The test asserts that a MobilityChunkDelta arrives, confirming the
    // per-chunk fan-out pipeline is wired.
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

    // Subscribe only to the abutopia chunk with authored base-world mobility.
    send_chunk_subscribe(&mut client, &[seeded_mobility_chunk()]).await;

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
