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
    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["chunk_size"], 32);
}
