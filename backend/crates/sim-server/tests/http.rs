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
                world_id: "zurich-river-city-v1".to_string(),
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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/worlds/zurich-river-city-v1"),
    )
    .expect("base world fixture loads")
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
    assert_eq!(health.world_id, "zurich-river-city-v1");
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
    assert_eq!(world.world_id, "zurich-river-city-v1");
    assert_eq!(world.chunk_size, 32);
    assert_eq!(world.loaded_chunks.len(), 64);
    assert_eq!(world.loaded_chunks[0], w::ChunkCoord { x: 0, y: 0 });
    assert!(world.loaded_chunks.contains(&w::ChunkCoord { x: 4, y: 4 }));
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

    assert_eq!(snap.world_id, "zurich-river-city-v1");
    assert_eq!(snap.coord, Some(w::ChunkCoord { x: 4, y: 4 }));
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
    assert_eq!(mobility.world_id, "zurich-river-city-v1");
    assert_eq!(mobility.tick, 0);
    assert!(mobility.agents.len() >= 50);
    assert!(!mobility.vehicles.is_empty());
}

#[tokio::test]
async fn command_sets_tile_kind_and_returns_event() {
    let app = build_app();
    let command = set_tile_kind_proto("command:http:1", 4, 4, 11, w::TileKind::Water);

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
    let snapshot = poll_chunk_until(&app, "/chunks/4/4", |snap| {
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
    let command = set_tile_kind_proto("command:http:3", 4, 4, 1024, w::TileKind::Water);

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
    let before = w::ChunkSnapshot::decode(before_body.as_ref()).unwrap();

    let command = set_tile_kind_proto("command:http:store-failure", 4, 4, 11, w::TileKind::Water);
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
        let base_world = base_world_fixture();
        let event_store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store");
        let snapshot_store = PostgresChunkSnapshotStore::connect(
            &database_url,
            SimulationRuntime::default_world_id(),
            base_world.snapshot_compatibility(),
        )
        .await
        .expect("connect postgres snapshot store");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres mobility snapshot store");
        let (mut runtime, mut snapshot_store_box, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            &base_world,
        )
        .await
        .expect("hydrate first runtime");

        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("zurich-river-city-v1".to_string()),
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
        let base_world = base_world_fixture();
        let event_store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store (restart)");
        let snapshot_store = PostgresChunkSnapshotStore::connect(
            &database_url,
            SimulationRuntime::default_world_id(),
            base_world.snapshot_compatibility(),
        )
        .await
        .expect("connect postgres snapshot store (restart)");
        let mobility_snapshot_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect postgres mobility snapshot store (restart)");
        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            &base_world,
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
        let mut mobility_store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .expect("connect mobility store (first runtime)");
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

    let store = PostgresMobilitySnapshotStore::connect(&database_url)
        .await
        .expect("connect mobility store (second runtime)");
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

#[test]
fn auth_backdoor_env_var_is_not_referenced_in_source() {
    // Regression guard: the Supabase verifier must never contain a runtime
    // env-var bypass that accepts arbitrary tokens. See
    // docs/superpowers/specs/2026-05-29-security-ci-guardrails-design.md
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/card_hand.rs"
    ))
    .expect("read card_hand.rs source");
    assert!(
        !src.contains("TEST_MODE_ACCEPT_ALL_JWTS"),
        "auth backdoor env var must not be referenced in card_hand.rs"
    );
}
