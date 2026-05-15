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

use sim_server::app::build_app;

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

    assert!(
        tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await
            .is_err(),
        "tile pulse should not arrive immediately after hello"
    );

    let delta = read_next_tile_pulse(&mut stream).await;
    assert_eq!(delta.world_id.0, "abutown-main");
    assert_eq!(delta.coord.x, 4);
    assert_eq!(delta.coord.y, 4);
    assert_eq!(delta.tick, 1);
    assert_eq!(delta.version, 1);
    assert!(delta.local_index < 1024);

    let mobility_after_tile = read_server_message(&mut stream).await;
    assert!(matches!(
        mobility_after_tile,
        ServerMessageDto::MobilityDelta(_)
    ));

    assert!(
        tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await
            .is_err(),
        "tile pulse cadence should remain low frequency"
    );

    let next_delta = read_next_tile_pulse(&mut stream).await;
    assert_eq!(next_delta.tick, 2);

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
