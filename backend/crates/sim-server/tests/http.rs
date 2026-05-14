use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use sim_server::app::build_app;

#[tokio::test]
async fn health_and_world_summary_are_available() {
    let app = build_app();

    let health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health_response.status(), StatusCode::OK);

    let world_response = app
        .oneshot(
            Request::builder()
                .uri("/world")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(world_response.status(), StatusCode::OK);

    let body = world_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["protocol_version"], 1);
    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["chunk_size"], 32);
    assert_eq!(json["loaded_chunks"].as_array().unwrap().len(), 3);
    assert_eq!(json["loaded_chunks"][0]["x"], 4);
    assert_eq!(json["loaded_chunks"][0]["y"], 4);
    assert_eq!(json["loaded_chunks"][1]["x"], 5);
    assert_eq!(json["loaded_chunks"][1]["y"], 4);
    assert_eq!(json["loaded_chunks"][2]["x"], 4);
    assert_eq!(json["loaded_chunks"][2]["y"], 5);
}

#[tokio::test]
async fn chunk_snapshot_is_available_for_loaded_chunk() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["coord"]["x"], 4);
    assert_eq!(json["coord"]["y"], 4);
    assert_eq!(json["tile_count"], 1024);
    assert_eq!(json["chunk_state"], "active");

    let dirty_tiles = json["dirty_tiles"].as_array().unwrap();
    assert_eq!(dirty_tiles.len(), 1);
    assert_eq!(dirty_tiles[0]["local_index"], 0);
    assert_eq!(dirty_tiles[0]["kind"], "road");
}

#[tokio::test]
async fn every_loaded_chunk_snapshot_is_available() {
    let app = build_app();

    for (x, y) in [(4, 4), (5, 4), (4, 5)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/chunks/{x}/{y}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["coord"]["x"], x);
        assert_eq!(json["coord"]["y"], y);
        assert_eq!(json["tile_count"], 1024);
    }
}

#[tokio::test]
async fn unloaded_chunk_returns_not_found() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/0/0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mobility_snapshot_is_available() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/mobility")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["protocol_version"], 1);
    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["tick"], 0);
    assert_eq!(json["agents"][0]["id"], "agent:seed:0");
    assert_eq!(json["agents"][0]["state"]["type"], "walking");
    assert_eq!(
        json["agents"][0]["state"]["link_id"],
        "link:home-to-old-town-stop"
    );
    assert_eq!(json["vehicles"][0]["id"], "vehicle:shuttle:0");
    assert_eq!(json["vehicles"][0]["capacity"], 4);
    assert_eq!(json["stops"].as_array().unwrap().len(), 2);
}
