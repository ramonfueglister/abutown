use abutown_protocol::{
    ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto, WorldId,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

use sim_server::{
    app::{build_app, build_app_with_runtime},
    runtime::SimulationRuntime,
};

const TEST_USER_A: &str = "00000000-0000-0000-0000-000000000001";
const TEST_USER_B: &str = "00000000-0000-0000-0000-000000000002";

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

    let tiles = json["tiles"].as_array().unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0]["local_index"], 0);
    assert_eq!(tiles[0]["kind"], "road");
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
    assert_eq!(json["agents"].as_array().unwrap().len(), 0);
    assert_eq!(json["vehicles"].as_array().unwrap().len(), 0);
    assert_eq!(json["stops"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn road_vehicles_endpoint_returns_seeded_snapshot() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/road-vehicles")
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
    let vehicles = json["vehicles"].as_array().expect("vehicles array");
    assert!(vehicles.len() >= 80, "seed must populate at least 80 vehicles");
    assert!(vehicles[0]["sprite_key"].is_string());
    assert!(vehicles[0]["direction"].is_string());
    assert!(vehicles[0]["world_coord"]["x"].is_number());
}

#[tokio::test]
async fn command_sets_tile_kind_and_returns_event() {
    let app = build_app();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:http:1".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 11,
        kind: TileKindDto::Water,
    });

    let response = app
        .clone()
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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "accepted");
    assert_eq!(json["event"]["type"], "tile_kind_set");
    assert_eq!(json["event"]["command_id"], "command:http:1");
    assert_eq!(json["event"]["local_index"], 11);
    assert_eq!(json["event"]["kind"], "water");

    let snapshot_response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot_response.status(), StatusCode::OK);
    let body = snapshot_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let snapshot: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        snapshot["tiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tile| tile["local_index"] == 11 && tile["kind"] == "water")
    );
}

#[tokio::test]
async fn command_rejects_unloaded_chunk() {
    let app = build_app();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:http:2".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 9, y: 9 },
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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "rejected");
    assert_eq!(json["code"], "chunk_not_loaded");
    assert_eq!(json["command_id"], "command:http:2");
}

#[tokio::test]
async fn command_rejects_tile_out_of_bounds() {
    let app = build_app();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:http:3".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 1024,
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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "rejected");
    assert_eq!(json["code"], "tile_out_of_bounds");
    assert_eq!(json["command_id"], "command:http:3");
}

#[tokio::test]
async fn command_store_failure_returns_rejection_and_preserves_snapshot() {
    let app = build_app_with_runtime(SimulationRuntime::new_with_event_store(Box::new(
        sim_core::events::FailingWorldEventStore::new("database offline"),
    )));

    let before_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let before_body = before_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let before: Value = serde_json::from_slice(&before_body).unwrap();

    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:http:store-failure".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 11,
        kind: TileKindDto::Water,
    });

    let response = app
        .clone()
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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "rejected");
    assert_eq!(json["code"], "event_store_unavailable");

    let after_response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let after_body = after_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let after: Value = serde_json::from_slice(&after_body).unwrap();
    assert_eq!(after, before);
}

// ---------------------------------------------------------------------------
// Opt-in postgres integration tests. Skipped silently when
// `ABUTOWN_TEST_DATABASE_URL` is unset so they don't break local CI.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn postgres_world_state_survives_runtime_restart() {
    use abutown_protocol::{ChunkCoordDto, ClientCommandDto, SetTileKindCommandDto};
    use sim_core::ids::ChunkCoord;
    use sim_server::postgres_events::PostgresWorldEventStore;
    use sim_server::postgres_mobility::PostgresMobilitySnapshotStore;
    use sim_server::postgres_road_vehicles::PostgresRoadVehicleSnapshotStore;
    use sim_server::postgres_snapshots::PostgresChunkSnapshotStore;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipping postgres_world_state_survives_runtime_restart; \
             ABUTOWN_TEST_DATABASE_URL not set"
        );
        return;
    };

    // Use a unique command_id so re-runs don't collide with leftover rows from
    // earlier test runs (dedup would otherwise short-circuit our mutation).
    let command_id = format!("command:recover-test:{}", uuid::Uuid::now_v7());
    // Pick a unique tile index per run so re-runs against the same DB don't
    // hit `no_state_change` because a previous run already set this tile.
    // Skip index 0 (seeded as Road in chunk (4,4)) by adding 1 and constraining
    // to the valid range below the chunk tile count (1024).
    let local_index: u16 = (((uuid::Uuid::now_v7().as_u128() % 1023) as u16) + 1).min(1023);

    // ---- First runtime: hydrate, apply a command, persist snapshot, drop.
    {
        let event_store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store");
        let snapshot_store = PostgresChunkSnapshotStore::connect(
            &database_url,
            SimulationRuntime::default_world_id(),
        )
        .await
        .expect("connect postgres snapshot store");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres mobility snapshot store");
        let road_vehicle_snapshot_store = PostgresRoadVehicleSnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres road vehicle snapshot store");
        let mut runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            Box::new(road_vehicle_snapshot_store),
        )
        .await
        .expect("hydrate first runtime");

        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            command_id: command_id.clone(),
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index,
            kind: TileKindDto::Water,
        });
        match runtime.apply_client_command(command).await {
            Ok(_) => {}
            Err(rejection) if rejection.code == "no_state_change" => {
                // Tile is already in the target state from a prior run; the
                // restart-recovery assertion below still holds.
            }
            Err(other) => panic!("unexpected rejection: {other:?}"),
        }
        runtime
            .persist_chunk_snapshots()
            .await
            .expect("persist chunk snapshots");
        // runtime drops here, severing the in-memory state from the DB.
    }

    // ---- Second runtime: hydrate fresh from the same database.
    {
        let event_store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store (restart)");
        let snapshot_store = PostgresChunkSnapshotStore::connect(
            &database_url,
            SimulationRuntime::default_world_id(),
        )
        .await
        .expect("connect postgres snapshot store (restart)");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres mobility snapshot store (restart)");
        let road_vehicle_snapshot_store = PostgresRoadVehicleSnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres road vehicle snapshot store (restart)");
        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            Box::new(road_vehicle_snapshot_store),
        )
        .await
        .expect("hydrate restarted runtime");

        let restored = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("chunk (4,4) loaded after restart");
        assert!(
            restored
                .tiles
                .iter()
                .any(|t| t.local_index == local_index && t.kind == TileKindDto::Water),
            "post-restart snapshot must contain tile {local_index}=Water set before restart; \
             got tiles: {:?}",
            restored.tiles
        );
    }
}

#[tokio::test]
async fn postgres_duplicate_command_returns_same_response() {
    use sim_server::app::build_app_from_config;
    use sim_server::config::ServerConfig;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipping postgres_duplicate_command_returns_same_response; \
             ABUTOWN_TEST_DATABASE_URL not set"
        );
        return;
    };

    let config = ServerConfig {
        database_url,
        supabase_url: "http://dummy.local".to_string(),
    };
    let app = build_app_from_config(&config)
        .await
        .expect("build app from postgres config");

    let unique_command_id = format!("command:dup-test:{}", uuid::Uuid::now_v7());
    // Pick a unique tile index per run so the first POST is unlikely to hit
    // `no_state_change` from prior pollution. Indices 0..=1023 are valid.
    let local_index: u16 = ((uuid::Uuid::now_v7().as_u128() % 1024) as u16).max(1);
    let body = format!(
        r#"{{"type":"set_tile_kind","protocol_version":1,"world_id":"abutown-main","command_id":"{unique_command_id}","coord":{{"x":4,"y":4}},"local_index":{local_index},"kind":"water"}}"#
    );

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    let first_status = first.status();
    let first_body = first.into_body().collect().await.unwrap().to_bytes();

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let second_status = second.status();
    let second_body = second.into_body().collect().await.unwrap().to_bytes();

    assert_eq!(
        first_status,
        StatusCode::OK,
        "first command must succeed (body: {})",
        String::from_utf8_lossy(&first_body)
    );
    assert_eq!(
        second_status,
        StatusCode::OK,
        "duplicate command must also succeed idempotently (body: {})",
        String::from_utf8_lossy(&second_body)
    );
    assert_eq!(
        first_body, second_body,
        "duplicate command must return an identical response body"
    );
}

#[tokio::test]
async fn postgres_mobility_state_survives_runtime_restart() {
    use sim_core::events::InMemoryWorldEventStore;
    use sim_core::mobility::seed;
    use sim_core::persistence::{InMemoryChunkSnapshotStore, MobilitySnapshotStore};
    use sim_server::postgres_mobility::PostgresMobilitySnapshotStore;
    use sim_server::runtime::SimulationRuntime;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipping postgres_mobility_state_survives_runtime_restart; \
             ABUTOWN_TEST_DATABASE_URL not set"
        );
        return;
    };

    let world_id = format!("test:mobility:{}", uuid::Uuid::now_v7());

    let persisted_tick;
    let persisted_world;
    {
        let mobility_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect mobility store (first runtime)");
        let mut runtime = SimulationRuntime::new_with_all_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(mobility_store),
        );

        runtime.override_world_id_for_test(&world_id);
        runtime.set_mobility_for_test(seed::initial_world());

        for _ in 0..5 {
            let _ = runtime.next_mobility_delta_for_test();
        }
        persisted_tick = runtime.mobility_tick();
        persisted_world = runtime.mobility_world_clone_for_test();
        runtime
            .persist_mobility_snapshot()
            .await
            .expect("persist mobility snapshot");
    }

    let store = PostgresMobilitySnapshotStore::connect(&database_url)
        .await
        .expect("connect mobility store (second runtime)");
    let (tick, restored) = MobilitySnapshotStore::read(&store, &world_id)
        .await
        .expect("read mobility snapshot")
        .expect("snapshot must be present after restart");

    assert_eq!(tick, persisted_tick);
    assert_eq!(restored, persisted_world);

    let _ = sqlx::query("DELETE FROM mobility_snapshots WHERE world_id = $1")
        .bind(&world_id)
        .execute(store.pool_for_test())
        .await;
}
