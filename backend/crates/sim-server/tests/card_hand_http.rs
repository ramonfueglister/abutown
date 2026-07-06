use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sim_server::app::{build_app, build_app_with_building_attributes};
use sim_server::building_attributes::{BuildingAttributes, BuildingAttributesStore};
use tower::ServiceExt;

const TEST_USER_A: &str = "00000000-0000-0000-0000-000000000001";
const TEST_USER_B: &str = "00000000-0000-0000-0000-000000000002";

#[tokio::test]
async fn health_reports_ok() {
    let app = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["service"], "abutown-sim");
    // Task 13: the health payload mirrors the world loop's liveness.
    assert_eq!(json["world_tick"], 0);
    assert_eq!(json["audit_ok"], true);
    assert_eq!(json["resumed"], false);
}

#[tokio::test]
async fn card_hand_requires_authenticated_user() {
    let app = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/card-hand")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authenticated_user_gets_default_card_hand() {
    let app = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/card-hand")
                .header("authorization", format!("Bearer {TEST_USER_A}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["user_id"], TEST_USER_A);
    assert_eq!(json["cards"].as_array().unwrap().len(), 5);
    assert_eq!(json["cards"][0]["instance_id"], 1);
    assert!(
        json["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["card_id"] == "strike")
    );
}

#[tokio::test]
async fn saved_card_hand_is_scoped_to_authenticated_user() {
    let app = build_app();
    let body = json!({
        "cards": [
            { "instance_id": 7, "card_id": "focus" },
            { "instance_id": 8, "card_id": "guard" }
        ]
    });

    let save_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/card-hand")
                .header("authorization", format!("Bearer {TEST_USER_A}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(save_response.status(), StatusCode::OK);

    let user_a_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/card-hand")
                .header("authorization", format!("Bearer {TEST_USER_A}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let user_a_body = user_a_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let user_a_json: Value = serde_json::from_slice(&user_a_body).unwrap();
    assert_eq!(user_a_json["cards"].as_array().unwrap().len(), 2);
    assert_eq!(user_a_json["cards"][0]["card_id"], "focus");

    let user_b_response = app
        .oneshot(
            Request::builder()
                .uri("/card-hand")
                .header("authorization", format!("Bearer {TEST_USER_B}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let user_b_body = user_b_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let user_b_json: Value = serde_json::from_slice(&user_b_body).unwrap();
    assert_eq!(user_b_json["cards"].as_array().unwrap().len(), 5);
    assert_ne!(user_b_json["cards"][0]["card_id"], "focus");
}

#[tokio::test]
async fn building_attributes_endpoint_lists_world_rows() {
    let app = build_app(); // in-memory wiring
    // Seeding: build_app's memory store starts empty -> expect 200 + []
    let res = app
        .oneshot(
            Request::builder()
                .uri("/building-attributes?world_id=winterthur")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"[]");
}

#[tokio::test]
async fn building_attributes_endpoint_round_trips_seeded_row_as_camel_case() {
    let store = BuildingAttributesStore::memory();
    store
        .upsert_all(
            "winterthur",
            &[BuildingAttributes {
                building_id: "{ABC}".to_string(),
                egid: Some(150404),
                gwr_category: Some("Wohngebäude".to_string()),
                gwr_class: Some("1110".to_string()),
                bauzone: Some("Wohnzone W3".to_string()),
                bauzone_code: Some("W3".to_string()),
                raw: json!({"egids": [150404]}),
            }],
        )
        .await
        .unwrap();
    let app = build_app_with_building_attributes(store);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/building-attributes?world_id=winterthur")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "{ABC}");
    assert_eq!(rows[0]["egid"], 150404);
    assert_eq!(rows[0]["gwrCategory"], "Wohngebäude");
    assert_eq!(rows[0]["gwrClass"], "1110");
    assert_eq!(rows[0]["bauzone"], "Wohnzone W3");
    assert_eq!(rows[0]["bauzoneCode"], "W3");
    assert_eq!(rows[0]["raw"]["egids"][0], 150404);

    // A different world_id sees no rows.
    let res_other = app
        .oneshot(
            Request::builder()
                .uri("/building-attributes?world_id=other")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let other_body = res_other.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&other_body[..], b"[]");
}
