use std::time::Duration;

use abutown_protocol::ServerMessageDto;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;

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

    let hello_text = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap()
        .to_string();
    let hello: ServerMessageDto = serde_json::from_str(&hello_text).unwrap();
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let pulse_text = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap()
        .to_string();
    let pulse: ServerMessageDto = serde_json::from_str(&pulse_text).unwrap();

    let ServerMessageDto::TilePulse(delta) = pulse else {
        panic!("second websocket message should be a tile pulse");
    };
    assert_eq!(delta.world_id.0, "abutown-main");
    assert_eq!(delta.coord.x, 4);
    assert_eq!(delta.coord.y, 4);
    assert_eq!(delta.tick, 1);
    assert_eq!(delta.version, 1);
    assert!(delta.local_index < 1024);

    server.abort();
}
