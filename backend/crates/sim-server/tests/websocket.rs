use std::time::Duration;

use abutown_protocol::{
    ClientCommandDto, PROTOCOL_VERSION, ServerMessageDto, SetTileKindCommandDto, TileKindDto,
    TilePulseDeltaDto, WorldEventDto, WorldId,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tower::ServiceExt;

use sim_server::{
    app::{build_app, build_app_with_runtime},
    runtime::SimulationRuntime,
};

#[tokio::test]
async fn websocket_sends_hello_and_tile_pulse() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    // 10 Hz tick: the first tile pulse arrives within roughly one tick period.
    // Use 250 ms to absorb scheduler jitter on slow CI without weakening intent.
    let first_pulse = tokio::time::timeout(
        Duration::from_millis(250),
        read_next_tile_pulse(&mut stream),
    )
    .await
    .expect("first tile pulse must arrive within one tick window");
    assert_eq!(first_pulse.world_id.0, "abutown-main");
    assert_eq!(first_pulse.coord.x, 4);
    assert_eq!(first_pulse.coord.y, 4);
    assert_eq!(first_pulse.tick, 1);
    assert_eq!(first_pulse.version, 1);
    assert!(first_pulse.local_index < 1024);

    let mobility_after_tile = read_server_message(&mut stream).await;
    assert!(matches!(
        mobility_after_tile,
        ServerMessageDto::MobilityDelta(_)
    ));

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
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let first_delta = read_next_tile_pulse(&mut stream).await;
    let second_delta = read_next_tile_pulse(&mut stream).await;
    let third_delta = read_next_tile_pulse(&mut stream).await;

    assert_eq!(
        first_delta.coord,
        abutown_protocol::ChunkCoordDto { x: 4, y: 4 }
    );
    assert_eq!(
        second_delta.coord,
        abutown_protocol::ChunkCoordDto { x: 5, y: 4 }
    );
    assert_eq!(
        third_delta.coord,
        abutown_protocol::ChunkCoordDto { x: 4, y: 5 }
    );

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
    assert!(matches!(first_hello, ServerMessageDto::Hello(_)));
    assert!(matches!(second_hello, ServerMessageDto::Hello(_)));

    let first_delta = read_next_tile_pulse(&mut first_stream).await;
    let second_delta = read_next_tile_pulse(&mut second_stream).await;

    assert_eq!(second_delta.tick, first_delta.tick);
    assert_eq!(second_delta.version, first_delta.version);
    assert_eq!(second_delta.local_index, first_delta.local_index);

    server.abort();
}

#[tokio::test]
async fn websocket_sends_mobility_deltas_after_hello() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let first = read_server_message(&mut stream).await;
    let second = read_server_message(&mut stream).await;

    let mobility_delta = match (first, second) {
        (ServerMessageDto::MobilityDelta(delta), _) => delta,
        (_, ServerMessageDto::MobilityDelta(delta)) => delta,
        _ => panic!("expected one mobility delta among first two broadcast messages"),
    };

    assert_eq!(mobility_delta.world_id.0, "abutown-main");
    assert_eq!(mobility_delta.tick, 1);
    assert!(mobility_delta.changed_agents.is_empty());
    assert!(mobility_delta.changed_vehicles.is_empty());

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
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:ws:1".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 12,
        kind: TileKindDto::BuildingFootprint,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&command).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.into_body().collect().await.unwrap();

    loop {
        let message = read_server_message(&mut stream).await;
        if let ServerMessageDto::WorldEvent {
            event: WorldEventDto::TileKindSet(event),
        } = message
        {
            assert_eq!(event.command_id, "command:ws:1");
            assert_eq!(event.coord, abutown_protocol::ChunkCoordDto { x: 4, y: 4 });
            assert_eq!(event.local_index, 12);
            assert_eq!(event.kind, TileKindDto::BuildingFootprint);
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

    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:ws:store-failure".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 11,
        kind: TileKindDto::Water,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&command).unwrap()))
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
                let text = match message.expect("ws message") {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                    _ => continue,
                };
                let parsed: ServerMessageDto = serde_json::from_str(&text).expect("server message");
                assert!(
                    !matches!(parsed, ServerMessageDto::WorldEvent { .. }),
                    "failed command must not broadcast a WorldEvent, got: {parsed:?}"
                );
            }
        }
    }

    server.abort();
}

async fn read_server_message<S>(stream: &mut S) -> ServerMessageDto
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let text = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap()
        .to_string();
    serde_json::from_str(&text).unwrap()
}

async fn read_next_tile_pulse<S>(stream: &mut S) -> TilePulseDeltaDto
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
        if let ServerMessageDto::TilePulse(delta) = message {
            return delta;
        }
    }
}
