use abutown_protocol::v1 as w;
use abutown_protocol::{PROTOCOL_VERSION, TileKindDto, WorldId};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use prost::Message;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

use sim_server::{
    app::{build_app, build_app_with_runtime},
    runtime::SimulationRuntime,
};

const TEST_USER_A: &str = "00000000-0000-0000-0000-000000000001";
const TEST_USER_B: &str = "00000000-0000-0000-0000-000000000002";

/// Poll a GET endpoint that returns a binary `ChunkSnapshot` protobuf body
/// until `predicate(&snap)` returns true, or panic after ~1s. Phase 7c made
/// HTTP reads eventually-consistent (view is published every 100 ms tick) —
/// tests that do read-after-write should poll instead of sleeping for a
/// fixed interval.
async fn poll_chunk_until<F>(app: &axum::Router, uri: &str, predicate: F) -> w::ChunkSnapshot
where
    F: Fn(&w::ChunkSnapshot) -> bool,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
    loop {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        if response.status() == StatusCode::OK {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            if let Ok(snap) = w::ChunkSnapshot::decode(body.as_ref())
                && predicate(&snap)
            {
                return snap;
            }
        }
        if std::time::Instant::now() >= deadline {
            panic!("poll_chunk_until timed out after 1s waiting on {uri}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Encode a `ClientCommand` proto and POST it to /commands. Returns the
/// raw HTTP response so tests can assert on status + decode the response
/// proto themselves.
async fn post_command(app: &axum::Router, command: w::ClientCommand) -> axum::response::Response {
    let body = command.encode_to_vec();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/x-protobuf")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn set_tile_kind_proto(
    command_id: &str,
    x: i32,
    y: i32,
    local_index: u32,
    kind: w::TileKind,
) -> w::ClientCommand {
    w::ClientCommand {
        command: Some(w::client_command::Command::SetTileKind(
            w::SetTileKindCommand {
                protocol_version: u32::from(PROTOCOL_VERSION),
                world_id: "abutopia".to_string(),
                command_id: command_id.to_string(),
                coord: Some(w::ChunkCoord { x, y }),
                local_index,
                kind: kind as i32,
            },
        )),
    }
}

fn base_world_fixture() -> sim_core::base_world::BaseWorldBundle {
    sim_core::base_world::BaseWorldBundle::load_from_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/worlds/abutopia"),
    )
    .expect("base world fixture loads")
}

/// The abutopia fixture rebranded under `world_id`. `hydrate_from_stores`
/// derives its world id from the bundle (event replay, mobility/economy
/// snapshot reads, the runtime's own command validation, and the world id
/// stamped onto collected chunk snapshots all key on it), so rebranding the
/// bundle is what isolates a postgres test from the live world's rows.
fn base_world_fixture_with_world_id(world_id: &str) -> sim_core::base_world::BaseWorldBundle {
    let mut bundle = base_world_fixture();
    bundle.manifest.world_id = world_id.to_string();
    bundle.terrain.world_id = world_id.to_string();
    bundle.transport.world_id = world_id.to_string();
    bundle.buildings.world_id = world_id.to_string();
    bundle.decorations.world_id = world_id.to_string();
    bundle.spawns.world_id = world_id.to_string();
    bundle.markets.world_id = world_id.to_string();
    bundle
}

fn expected_abutopia_proto_chunks() -> Vec<w::ChunkCoord> {
    (0..=3)
        .flat_map(|y| (0..=6).map(move |x| w::ChunkCoord { x, y }))
        .collect()
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
    assert_eq!(health.world_id, "abutopia");
    assert!(health.ok);
    let persistence = health.persistence.expect("persistence health present");
    assert_eq!(
        persistence.status,
        w::PersistenceHealthStatus::Starting as i32
    );
    assert_eq!(persistence.world_id, "");

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
    assert_eq!(world.world_id, "abutopia");
    assert_eq!(world.chunk_size, 32);
    assert_eq!(world.loaded_chunks, expected_abutopia_proto_chunks());
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
                .uri("/chunks/3/2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let snap = w::ChunkSnapshot::decode(body.as_ref()).unwrap();

    assert_eq!(snap.world_id, "abutopia");
    assert_eq!(snap.coord, Some(w::ChunkCoord { x: 3, y: 2 }));
    assert_eq!(snap.tile_count, 1024);
    assert_eq!(snap.chunk_state, w::ChunkState::Warm as i32);

    assert!(!snap.tiles.is_empty());
    assert!(
        snap.tiles
            .iter()
            .any(|tile| tile.kind == w::TileKind::Road as i32)
    );
}

#[tokio::test]
async fn every_loaded_chunk_snapshot_is_available() {
    let app = build_app();

    for w::ChunkCoord { x, y } in expected_abutopia_proto_chunks() {
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
        let snap = w::ChunkSnapshot::decode(body.as_ref()).unwrap();
        assert_eq!(snap.coord, Some(w::ChunkCoord { x, y }));
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
    assert_eq!(mobility.world_id, "abutopia");
    assert_eq!(mobility.tick, 0);
    assert_eq!(mobility.agents.len(), 300);
    assert!(mobility.vehicles.is_empty());
}

#[tokio::test]
async fn command_sets_tile_kind_and_returns_event() {
    let app = build_app();
    let command = set_tile_kind_proto("command:http:1", 0, 0, 11, w::TileKind::Water);

    let response = post_command(&app, command).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let resp = w::CommandResponse::decode(body.as_ref()).unwrap();
    let accepted = match resp.outcome {
        Some(w::command_response::Outcome::Accepted(a)) => a,
        other => panic!("expected accepted outcome, got: {other:?}"),
    };
    assert_eq!(accepted.command_id, "command:http:1");
    let event = accepted.event.expect("accepted command must carry event");
    let tk = match event.event {
        Some(w::world_event::Event::TileKindSet(tk)) => tk,
        other => panic!("expected tile_kind_set event, got: {other:?}"),
    };
    assert_eq!(tk.command_id, "command:http:1");
    assert_eq!(tk.local_index, 11);
    assert_eq!(tk.kind, w::TileKind::Water as i32);

    // Phase 7c: /chunks/{x}/{y} reads from the RuntimeReadView which is
    // published once per tick (100 ms). Poll for the mutation to become
    // visible instead of using a fixed sleep — faster + more robust on
    // slow CI than `sleep(150ms)`.
    let snapshot = poll_chunk_until(&app, "/chunks/0/0", |snap| {
        snap.tiles
            .iter()
            .any(|tile| tile.local_index == 11 && tile.kind == w::TileKind::Water as i32)
    })
    .await;

    assert!(
        snapshot
            .tiles
            .iter()
            .any(|tile| tile.local_index == 11 && tile.kind == w::TileKind::Water as i32)
    );
}

#[tokio::test]
async fn command_rejects_unloaded_chunk() {
    let app = build_app();
    let command = set_tile_kind_proto("command:http:2", 9, 9, 11, w::TileKind::Water);

    let response = post_command(&app, command).await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let resp = w::CommandResponse::decode(body.as_ref()).unwrap();
    let rejected = match resp.outcome {
        Some(w::command_response::Outcome::Rejected(r)) => r,
        other => panic!("expected rejected outcome, got: {other:?}"),
    };
    assert_eq!(rejected.code, "chunk_not_loaded");
    assert_eq!(rejected.command_id, "command:http:2");
}

#[tokio::test]
async fn command_rejects_tile_out_of_bounds() {
    let app = build_app();
    let command = set_tile_kind_proto("command:http:3", 0, 0, 1024, w::TileKind::Water);

    let response = post_command(&app, command).await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let resp = w::CommandResponse::decode(body.as_ref()).unwrap();
    let rejected = match resp.outcome {
        Some(w::command_response::Outcome::Rejected(r)) => r,
        other => panic!("expected rejected outcome, got: {other:?}"),
    };
    assert_eq!(rejected.code, "tile_out_of_bounds");
    assert_eq!(rejected.command_id, "command:http:3");
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
                .uri("/chunks/0/0")
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
    let before = w::ChunkSnapshot::decode(before_body.as_ref()).unwrap();

    let command = set_tile_kind_proto("command:http:store-failure", 0, 0, 11, w::TileKind::Water);
    let response = post_command(&app, command).await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let resp = w::CommandResponse::decode(body.as_ref()).unwrap();
    let rejected = match resp.outcome {
        Some(w::command_response::Outcome::Rejected(r)) => r,
        other => panic!("expected rejected outcome, got: {other:?}"),
    };
    assert_eq!(rejected.code, "event_store_unavailable");

    let after_response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/0/0")
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
    let after = w::ChunkSnapshot::decode(after_body.as_ref()).unwrap();
    // Compare only the fields that the failed command should preserve.
    // `chunk_state` is excluded because the background tick loop's LOD
    // reclassifier (Phase 8a Task 8) can legitimately demote an idle chunk
    // (no subscribers, no population) from Active → Asleep between the two
    // HTTP calls — that has nothing to do with command success or failure.
    assert_eq!(after.protocol_version, before.protocol_version);
    assert_eq!(after.world_id, before.world_id);
    assert_eq!(after.coord, before.coord);
    assert_eq!(after.chunk_version, before.chunk_version);
    assert_eq!(after.tile_count, before.tile_count);
    assert_eq!(after.tiles, before.tiles);
}

// ---------------------------------------------------------------------------
// Opt-in postgres integration tests. Skipped silently when
// `ABUTOWN_TEST_DATABASE_URL` is unset so they don't break local CI.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn postgres_world_state_survives_runtime_restart() {
    use abutown_protocol::{ChunkCoordDto, ClientCommandDto, SetTileKindCommandDto};
    use sim_core::ids::ChunkCoord;
    use sim_server::db::connect_shared_pool;
    use sim_server::postgres_economy::PostgresEconomySnapshotStore;
    use sim_server::postgres_events::PostgresWorldEventStore;
    use sim_server::postgres_mobility::PostgresMobilitySnapshotStore;
    use sim_server::postgres_snapshots::PostgresChunkSnapshotStore;

    let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipping postgres_world_state_survives_runtime_restart; \
             ABUTOWN_TEST_DATABASE_URL not set"
        );
        return;
    };

    // Isolated world id: events, chunk snapshots, and command dedup are all
    // keyed on world_id, so a unique id per run keeps the test from reading
    // or mutating the live world's rows. Rows are deleted again at the end.
    let world_id = format!("test:recover:{}", uuid::Uuid::now_v7());
    let command_id = format!("command:recover-test:{}", uuid::Uuid::now_v7());
    let local_index: u16 = 11;

    let pool = connect_shared_pool(&database_url)
        .await
        .expect("connect shared pool");

    // ---- First runtime: hydrate, apply a command, persist snapshot, drop.
    let target_kind;
    {
        let base_world = base_world_fixture_with_world_id(&world_id);
        let event_store = PostgresWorldEventStore::with_pool(pool.clone())
            .await
            .expect("with_pool postgres event store");
        let snapshot_store = PostgresChunkSnapshotStore::with_pool(
            pool.clone(),
            WorldId(world_id.clone()),
            base_world.snapshot_compatibility(),
        )
        .await
        .expect("with_pool postgres snapshot store");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::with_pool(pool.clone())
            .await
            .expect("with_pool postgres mobility snapshot store");
        let economy_snapshot_store = PostgresEconomySnapshotStore::with_pool(pool.clone())
            .await
            .expect("with_pool postgres economy snapshot store");
        let (mut runtime, mut snapshot_store_box, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            Box::new(economy_snapshot_store),
            &base_world,
        )
        .await
        .expect("hydrate first runtime");

        // Flip the tile to a kind it doesn't currently have so the command can
        // never be rejected with `no_state_change` (the fresh world id starts
        // from the base world, never from prior-run pollution). Snapshot tiles
        // are sparse — an index absent from `tiles` holds the default kind
        // (Grass), so the post-restart assertion compares via the same lookup.
        let current_kind = runtime
            .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
            .expect("chunk (0,0) loaded from base world")
            .tiles
            .iter()
            .find(|t| t.local_index == local_index)
            .map(|t| t.kind)
            .unwrap_or(TileKindDto::Grass);
        target_kind = if current_kind == TileKindDto::Water {
            TileKindDto::Grass
        } else {
            TileKindDto::Water
        };

        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId(world_id.clone()),
            command_id: command_id.clone(),
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index,
            kind: target_kind,
        });
        runtime
            .apply_client_command(command)
            .await
            .expect("command must apply cleanly on a fresh isolated world");
        // Persist using the store directly (stores now live outside the runtime).
        let snapshots = runtime.collect_chunk_snapshots();
        let coords: Vec<sim_core::ids::ChunkCoord> = snapshots
            .iter()
            .map(|s| sim_core::ids::ChunkCoord {
                x: s.coord.x,
                y: s.coord.y,
            })
            .collect();
        for snapshot in snapshots {
            snapshot_store_box
                .write_snapshot(snapshot, &base_world.snapshot_compatibility())
                .await
                .expect("persist chunk snapshots");
        }
        runtime.mark_chunk_snapshots_persisted(&coords);
        // runtime drops here, severing the in-memory state from the DB.
    }

    // ---- Second runtime: hydrate fresh from the same database.
    {
        let base_world = base_world_fixture_with_world_id(&world_id);
        let event_store = PostgresWorldEventStore::with_pool(pool.clone())
            .await
            .expect("with_pool postgres event store (restart)");
        let snapshot_store = PostgresChunkSnapshotStore::with_pool(
            pool.clone(),
            WorldId(world_id.clone()),
            base_world.snapshot_compatibility(),
        )
        .await
        .expect("with_pool postgres snapshot store (restart)");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::with_pool(pool.clone())
            .await
            .expect("with_pool postgres mobility snapshot store (restart)");
        let economy_snapshot_store = PostgresEconomySnapshotStore::with_pool(pool.clone())
            .await
            .expect("with_pool postgres economy snapshot store (restart)");
        let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            Box::new(economy_snapshot_store),
            &base_world,
        )
        .await
        .expect("hydrate restarted runtime");

        let restored = runtime
            .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
            .expect("chunk (0,0) loaded after restart");
        let restored_kind = restored
            .tiles
            .iter()
            .find(|t| t.local_index == local_index)
            .map(|t| t.kind)
            .unwrap_or(TileKindDto::Grass);
        assert_eq!(
            restored_kind, target_kind,
            "post-restart snapshot must restore tile {local_index}={target_kind:?} \
             set before restart; got tiles: {:?}",
            restored.tiles
        );
    }

    let _ = sqlx::query("DELETE FROM world_events WHERE world_id = $1")
        .bind(&world_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM chunk_snapshots WHERE world_id = $1")
        .bind(&world_id)
        .execute(&pool)
        .await;
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
        cors_allowed_origins: Vec::new(),
    };
    let app = build_app_from_config(&config)
        .await
        .expect("build app from postgres config");

    let unique_command_id = format!("command:dup-test:{}", uuid::Uuid::now_v7());
    // Pick a unique tile index per run so the first POST is unlikely to hit
    // `no_state_change` from prior pollution. Indices 0..=1023 are valid.
    let local_index: u32 = ((uuid::Uuid::now_v7().as_u128() % 1024) as u32).clamp(1, 1023);
    let command = set_tile_kind_proto(&unique_command_id, 4, 4, local_index, w::TileKind::Water);
    let body = command.encode_to_vec();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/x-protobuf")
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
                .header("content-type", "application/x-protobuf")
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
        "first command must succeed (body: {} bytes)",
        first_body.len()
    );
    assert_eq!(
        second_status,
        StatusCode::OK,
        "duplicate command must also succeed idempotently (body: {} bytes)",
        second_body.len()
    );
    assert_eq!(
        first_body, second_body,
        "duplicate command must return an identical response body"
    );
}

#[tokio::test]
async fn postgres_mobility_state_survives_runtime_restart() {
    use sim_core::persistence::MobilitySnapshotStore;
    use sim_server::db::connect_shared_pool;
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
    let compatibility = base_world_fixture().snapshot_compatibility();

    let persisted_tick;
    let persisted_world;
    {
        let pool = connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool (mobility first)");
        let mut mobility_store = PostgresMobilitySnapshotStore::with_pool(pool)
            .await
            .expect("with_pool mobility store (first runtime)");
        let mut runtime = SimulationRuntime::new();

        runtime.override_world_id_for_test(&world_id);

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
            &compatibility,
        )
        .await
        .expect("persist mobility snapshot");
    }

    let pool2 = connect_shared_pool(&database_url)
        .await
        .expect("connect shared pool (mobility second)");
    let store = PostgresMobilitySnapshotStore::with_pool(pool2)
        .await
        .expect("with_pool mobility store (second runtime)");
    let (tick, restored) = MobilitySnapshotStore::read(&store, &world_id, &compatibility)
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

/// Poll /world until `predicate(&world_summary)` returns true, or panic after ~1s.
async fn poll_world_until<F>(app: &axum::Router, predicate: F) -> w::WorldSummary
where
    F: Fn(&w::WorldSummary) -> bool,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
    loop {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/world")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() == StatusCode::OK {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            if let Ok(summary) = w::WorldSummary::decode(body.as_ref())
                && predicate(&summary)
            {
                return summary;
            }
        }
        if std::time::Instant::now() >= deadline {
            panic!("poll_world_until timed out after 1s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn world_summary_sim_time_is_present_and_advances() {
    let app = build_app();

    // Capture the initial sim_time.
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/world")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = first.into_body().collect().await.unwrap().to_bytes();
    let first_summary = w::WorldSummary::decode(first_body.as_ref()).unwrap();
    let initial_sim_time = first_summary.sim_time;
    // sim_time is a u64 — always non-negative; document that it is present.

    // Wait until sim_time advances (the background tick loop runs every 100 ms).
    let advanced = poll_world_until(&app, |s| s.sim_time > initial_sim_time).await;
    assert!(
        advanced.sim_time > initial_sim_time,
        "sim_time must advance after ticks: got {} expected > {}",
        advanced.sim_time,
        initial_sim_time
    );
}

#[tokio::test]
async fn mobility_snapshot_agent_age_seconds_is_present() {
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
    assert!(
        !mobility.agents.is_empty(),
        "at least one agent must be present for the age_seconds assertion"
    );
    for agent in &mobility.agents {
        // age_seconds is u64 — presence means we can decode and read it (>= 0 always).
        let _ = agent.age_seconds;
    }
    assert!(
        mobility.agents.iter().any(|agent| agent.age_seconds > 0),
        "seeded agents now have deterministic pre-epoch birth ticks, so at least one age_seconds value should be positive"
    );
}

#[test]
fn auth_backdoor_env_var_is_not_referenced_in_source() {
    // Regression guard: the Supabase verifier must never contain a runtime
    // env-var bypass that accepts arbitrary tokens. See
    // docs/superpowers/specs/2026-05-29-security-ci-guardrails-design.md
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/card_hand.rs"))
        .expect("read card_hand.rs source");
    assert!(
        !src.contains("TEST_MODE_ACCEPT_ALL_JWTS"),
        "auth backdoor env var must not be referenced in card_hand.rs"
    );
}
