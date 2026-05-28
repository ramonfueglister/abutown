use abutown_protocol::PROTOCOL_VERSION;
use abutown_protocol::v1 as w;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use prost::Message;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

use sim_server::{app::build_app, runtime::SimulationRuntime};

const TEST_USER_A: &str = "00000000-0000-0000-0000-000000000001";
const TEST_USER_B: &str = "00000000-0000-0000-0000-000000000002";

fn empty_test_network() -> sim_core::city_network::CityNetwork {
    sim_core::city_network::CityNetwork {
        version: 1,
        world_id: "test".to_string(),
        chunk_size: 32,
        world_tiles: sim_core::city_network::WorldTiles {
            width: 256,
            height: 256,
        },
        arterial_paths: vec![],
        pedestrian_corridors: vec![],
    }
}

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
    let health_body = health_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let health = w::HealthResponse::decode(health_body.as_ref()).unwrap();
    assert_eq!(health.protocol_version, u32::from(PROTOCOL_VERSION));
    assert_eq!(health.world_id, "abutown-main");
    assert!(health.ok);

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
    let world = w::WorldSummary::decode(body.as_ref()).unwrap();
    assert_eq!(world.protocol_version, u32::from(PROTOCOL_VERSION));
    assert_eq!(world.world_id, "abutown-main");
    assert_eq!(world.chunk_size, 32);
    let expected_chunks: Vec<w::ChunkCoord> = (0..8)
        .flat_map(|y| (0..8).map(move |x| w::ChunkCoord { x, y }))
        .collect();
    assert_eq!(world.loaded_chunks, expected_chunks);
    assert_eq!(world.tick_period_ms, 100);
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
    let snap = w::ChunkSnapshot::decode(body.as_ref()).unwrap();

    assert_eq!(snap.world_id, "abutown-main");
    assert_eq!(snap.coord, Some(w::ChunkCoord { x: 4, y: 4 }));
    assert_eq!(snap.tile_count, 1024);
    assert_eq!(snap.chunk_state, w::ChunkState::Active as i32);
    assert!(
        snap.tiles
            .iter()
            .any(|tile| tile.base == w::TileBase::Water as i32)
    );
    assert!(
        snap.tiles
            .iter()
            .any(|tile| tile.surface == w::TileSurface::Street as i32)
    );
}

#[tokio::test]
async fn every_loaded_chunk_snapshot_is_available() {
    let app = build_app();

    let world_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/world")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let world_body = world_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let world = w::WorldSummary::decode(world_body.as_ref()).unwrap();

    for coord in world.loaded_chunks {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/chunks/{}/{}", coord.x, coord.y))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let snap = w::ChunkSnapshot::decode(body.as_ref()).unwrap();
        assert_eq!(snap.coord, Some(coord));
        assert_eq!(snap.tile_count, 1024);
    }
}

#[tokio::test]
async fn unloaded_chunk_returns_not_found() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/8/8")
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
    let mobility = w::MobilitySnapshot::decode(body.as_ref()).unwrap();

    assert_eq!(mobility.protocol_version, u32::from(PROTOCOL_VERSION));
    assert_eq!(mobility.world_id, "abutown-main");
    assert_eq!(mobility.tick, 0);
    assert!(mobility.agents.is_empty());
    assert!(mobility.vehicles.is_empty());
    assert!(mobility.stops.is_empty());
}

#[tokio::test]
async fn cors_allows_local_vite_origin() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/health")
                .header(header::ORIGIN, "http://127.0.0.1:5175")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&"http://127.0.0.1:5175".parse().unwrap())
    );
}

#[tokio::test]
async fn cors_does_not_allow_unconfigured_origin() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/health")
                .header(header::ORIGIN, "https://attacker.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&"*".parse().unwrap())
    );
    assert!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}

#[tokio::test]
async fn commands_route_is_not_exposed() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Opt-in postgres integration tests. Skipped silently when
// `ABUTOWN_TEST_DATABASE_URL` is unset so they don't break local CI.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn postgres_world_state_survives_runtime_restart() {
    use abutown_protocol::TileBaseDto;
    use sim_core::chunk::Chunk;
    use sim_core::ids::ChunkCoord;
    use sim_core::persistence::{ChunkSnapshotStore, build_chunk_snapshot};
    use sim_core::scheduler::ChunkActivity;
    use sim_core::tile::{TileBase, TileRecord};
    use sim_server::postgres_mobility::PostgresMobilitySnapshotStore;
    use sim_server::postgres_snapshots::PostgresChunkSnapshotStore;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipping postgres_world_state_survives_runtime_restart; \
             ABUTOWN_TEST_DATABASE_URL not set"
        );
        return;
    };

    let world_id = abutown_protocol::WorldId(format!(
        "test:terrain-restart:{}",
        uuid::Uuid::now_v7()
    ));
    let marker = format!("test:recover:{}", uuid::Uuid::now_v7());
    let local_index: u16 = (((uuid::Uuid::now_v7().as_u128() % 1023) as u16) + 1).min(1023);

    // ---- First phase: persist a layered terrain snapshot authored directly from TileRecord.
    {
        let mut snapshot_store =
            PostgresChunkSnapshotStore::connect(&database_url, world_id.clone())
                .await
                .expect("connect postgres snapshot store");

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        chunk
            .set_tile_record(
                local_index,
                TileRecord {
                    base: TileBase::Water,
                    display: Some(marker.clone()),
                    ..TileRecord::default()
                },
            )
            .expect("local index is in bounds");
        let snapshot = build_chunk_snapshot(world_id.0.clone(), &chunk, ChunkActivity::Active);
        ChunkSnapshotStore::write_snapshot(&mut snapshot_store, snapshot)
            .await
            .expect("persist chunk snapshot");
    }

    // ---- Second runtime: hydrate fresh from the same database.
    {
        let snapshot_store =
            PostgresChunkSnapshotStore::connect(&database_url, world_id.clone())
                .await
                .expect("connect postgres snapshot store (restart)");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres mobility snapshot store (restart)");
        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            &empty_test_network(),
        )
        .await
        .expect("hydrate restarted runtime");

        let restored = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("chunk (4,4) loaded after restart");
        assert!(
            restored.tiles.iter().any(|t| {
                t.local_index == local_index
                    && t.base == TileBaseDto::Water
                    && t.display.as_deref() == Some(marker.as_str())
            }),
            "post-restart snapshot must contain persisted layered tile {local_index}; \
             got tiles: {:?}",
            restored.tiles
        );
    }
}

#[tokio::test]
async fn postgres_mobility_state_survives_runtime_restart() {
    use sim_core::mobility::seed;
    use sim_core::persistence::MobilitySnapshotStore;
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
        let mut mobility_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect mobility store (first runtime)");
        let mut runtime = SimulationRuntime::new();

        runtime.override_world_id_for_test(&world_id);
        runtime.set_mobility_for_test(seed::initial_world());

        for _ in 0..5 {
            runtime.advance_mobility_tick_for_test();
        }
        persisted_tick = runtime.mobility_tick();
        persisted_world = runtime.mobility_persist_snapshot();
        // Persist mobility directly through the store (stores now live outside the runtime).
        MobilitySnapshotStore::write(
            &mut mobility_store,
            &world_id,
            persisted_tick,
            &runtime.mobility_persist_snapshot(),
        )
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
