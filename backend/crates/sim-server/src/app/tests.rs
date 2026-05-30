use super::*;
use abutown_protocol::ChunkSnapshotDto;
use sim_core::ids::ChunkCoord;
use sim_core::persistence::{
    ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryEconomySnapshotStore,
    MobilitySnapshotStore, MobilitySnapshotStoreError,
};
use std::time::Duration;

/// Wait long enough for the spawned tick_loop to advance the published
/// view at least once. SIMULATION_TICK_INTERVAL is 100 ms; we wait 2.5×
/// to absorb scheduler jitter on slow CI.
const TICK_WAIT: Duration = Duration::from_millis(250);

/// Wait until the published view's mobility_tick advances strictly past
/// `from`, or until the deadline passes. Returns the observed tick.
async fn wait_for_tick_past(state: &AppState, from: u64, deadline: Duration) -> u64 {
    let start = std::time::Instant::now();
    loop {
        let t = state.view().load().mobility_tick;
        if t > from {
            return t;
        }
        if start.elapsed() >= deadline {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn mutate_runtime_tile(runtime: &mut SimulationRuntime, command_id: &str) {
    runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: command_id.to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 11,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect("test mutation applies");
}

#[tokio::test]
async fn concurrent_view_reads_do_not_deadlock() {
    // The new architecture's invariant is stronger than the old
    // "lock-free reads under write contention" — there is no longer any
    // lock at all. Verify many concurrent view.load() calls complete
    // promptly.
    use std::time::Instant;
    let state = AppState::new(SimulationRuntime::new());

    let start = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..100 {
        let s = state.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..50 {
                let _ = s.view().load().world_summary.clone();
            }
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "concurrent view reads took {elapsed:?}"
    );
}

#[tokio::test]
async fn runtime_read_view_updates_after_tick() {
    let state = AppState::new(SimulationRuntime::new());
    let tick0 = state.view().load().mobility_tick;
    let observed = wait_for_tick_past(&state, tick0, TICK_WAIT).await;
    assert!(observed > tick0, "tick should have advanced past {tick0}");

    let view1 = state.view().load();
    assert!(
        !view1.chunk_snapshots.is_empty(),
        "view should include chunk snapshots"
    );
}

#[tokio::test]
async fn view_holds_mobility_chunk_snapshots_for_loaded_chunks() {
    let state = AppState::new(SimulationRuntime::new());
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;
    let view = state.view().load();
    assert!(
        !view.mobility_chunk_snapshots.is_empty(),
        "view should hold mobility chunk snapshots for loaded chunks"
    );
    for coord in view.chunk_snapshots.keys() {
        assert!(
            view.mobility_chunk_snapshots.contains_key(coord),
            "mobility_chunk_snapshots missing chunk {coord:?} (present in chunk_snapshots)"
        );
    }
}

#[tokio::test]
async fn persist_snapshots_once_writes_runtime_snapshots() {
    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:app-persist:1").await;
    let state = AppState::new(runtime);

    assert_eq!(persist_snapshots_once(&state).await.unwrap(), 1);

    let snapshot = state
        .stored_chunk_snapshot(ChunkCoord { x: 0, y: 0 })
        .await
        .unwrap()
        .expect("visible snapshot stored");
    assert_eq!(snapshot.coord.x, 0);
    assert_eq!(snapshot.coord.y, 0);
}

#[tokio::test]
async fn healthy_mobility_persistence_keeps_health_ok() {
    use sim_core::persistence::InMemoryMobilitySnapshotStore;

    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:app-persist-health:1").await;
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    assert_eq!(persist_snapshots_once(&state).await.unwrap(), 1);

    let health = health_response_for_state(&state);
    let persistence = health.persistence.expect("persistence health present");
    assert!(health.ok, "healthy persistence should keep /health OK");
    assert_eq!(
        persistence.status,
        w::PersistenceHealthStatus::Healthy as i32
    );
    assert_eq!(persistence.world_id, "abutopia");
    assert!(persistence.mobility_tick > 0);
    assert!(persistence.last_attempt_unix_ms > 0);
    assert!(persistence.last_success_unix_ms > 0);
    assert_eq!(persistence.consecutive_failures, 0);
    assert_eq!(persistence.last_error, "");
    assert!(persistence.freshness_ms <= 15_000);
}

#[derive(Debug, Default)]
struct FailingMobilitySnapshotStore;

#[async_trait::async_trait]
impl MobilitySnapshotStore for FailingMobilitySnapshotStore {
    async fn write(
        &mut self,
        _world_id: &str,
        _tick: u64,
        _snapshot: &sim_core::mobility::MobilityPersistSnapshot,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<(), MobilitySnapshotStoreError> {
        Err(MobilitySnapshotStoreError::unavailable(
            "postgres://user:password@db.example/abutown sb_secret_test failed",
        ))
    }

    async fn read(
        &self,
        _world_id: &str,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<
        Option<(u64, sim_core::mobility::MobilityPersistSnapshot)>,
        MobilitySnapshotStoreError,
    > {
        Ok(None)
    }
}

#[tokio::test]
async fn failing_mobility_write_marks_health_degraded_with_redacted_error() {
    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:app-persist-health-fail:1").await;
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(FailingMobilitySnapshotStore),
        Box::new(InMemoryEconomySnapshotStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    assert_eq!(
        persist_snapshots_once(&state).await.unwrap(),
        1,
        "mobility write failures should not fail chunk persistence"
    );

    let health = health_response_for_state(&state);
    let persistence = health.persistence.expect("persistence health present");
    assert!(
        !health.ok,
        "degraded persistence should make /health unhealthy"
    );
    assert_eq!(
        persistence.status,
        w::PersistenceHealthStatus::Degraded as i32
    );
    assert_eq!(persistence.world_id, "abutopia");
    assert!(persistence.mobility_tick > 0);
    assert_eq!(persistence.consecutive_failures, 1);
    assert!(persistence.last_attempt_unix_ms > 0);
    assert_eq!(persistence.last_success_unix_ms, 0);
    assert!(persistence.last_error.contains("<redacted>"));
    assert!(!persistence.last_error.contains("password"));
    assert!(!persistence.last_error.contains("sb_secret_test"));
}

#[tokio::test]
async fn health_degrades_when_base_world_agents_are_missing_from_published_mobility() {
    let state = AppState::new(SimulationRuntime::new());
    let mut view = state.view().load().as_ref().clone();
    view.mobility_full_dto.agents.clear();
    state.view().store(Arc::new(view));

    let health = health_response_for_state(&state);

    assert!(
        !health.ok,
        "health must fail when the published mobility view has fewer concrete agents than base-world spawns"
    );
}

#[derive(Debug, Default)]
struct CountingMobilitySnapshotStore {
    writes: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl MobilitySnapshotStore for CountingMobilitySnapshotStore {
    async fn write(
        &mut self,
        _world_id: &str,
        _tick: u64,
        _snapshot: &sim_core::mobility::MobilityPersistSnapshot,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<(), MobilitySnapshotStoreError> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn read(
        &self,
        _world_id: &str,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<
        Option<(u64, sim_core::mobility::MobilityPersistSnapshot)>,
        MobilitySnapshotStoreError,
    > {
        Ok(None)
    }
}

#[tokio::test]
async fn persist_snapshots_once_rejects_mobility_snapshots_below_base_world_agents() {
    let runtime = SimulationRuntime::new();
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let mut invalid_mobility =
        crate::runtime::initial_mobility_snapshot_for_base_world(&base_world)
            .expect("base-world mobility seeds");
    invalid_mobility.agents.clear();

    let (mutation_tx, mut mutation_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(mutation) = mutation_rx.recv().await {
            if let crate::runtime_view::Mutation::CollectPersistData { reply } = mutation {
                let _ = reply.send(crate::runtime_view::PersistPayload {
                    chunk_snapshots: Vec::new(),
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    mobility_tick: 42,
                    mobility_world: invalid_mobility.clone(),
                    economy_tick: 0,
                    economy_world: sim_core::economy::EconomyPersistSnapshot::default(),
                });
            }
        }
    });

    let counted_store = CountingMobilitySnapshotStore::default();
    let write_count = Arc::clone(&counted_store.writes);
    let state = AppState {
        deltas: tokio::sync::broadcast::channel(DELTA_BROADCAST_CAPACITY).0,
        card_hands: CardHandStore::memory(),
        auth: AuthVerifier::local_bearer_uuid(),
        snapshot_store: Arc::new(Mutex::new(Box::new(InMemoryChunkSnapshotStore::default()))),
        mobility_snapshot_store: Arc::new(Mutex::new(Box::new(counted_store))),
        economy_snapshot_store: Arc::new(Mutex::new(Box::new(
            InMemoryEconomySnapshotStore::default(),
        ))),
        chunk_channels: Arc::new(DashMap::new()),
        view: Arc::new(arc_swap::ArcSwap::from_pointee(
            build_read_view_from_runtime(&runtime, &std::collections::HashMap::new(), 0),
        )),
        mutations: mutation_tx,
        base_world: Arc::new(BaseWorldResponse::from(&base_world)),
        mobility_liveness: Arc::new(MobilityPersistenceLiveness::new(
            MOBILITY_PERSISTENCE_FRESHNESS_WINDOW,
        )),
        expected_base_world_agents: crate::runtime::expected_base_world_agent_count(&base_world),
    };

    assert_eq!(persist_snapshots_once(&state).await.unwrap(), 0);

    assert_eq!(
        write_count.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "invalid mobility snapshots must not be written"
    );
    let health = health_response_for_state(&state);
    let persistence = health.persistence.expect("persistence health present");
    assert!(!health.ok);
    assert_eq!(
        persistence.status,
        w::PersistenceHealthStatus::Degraded as i32
    );
    assert!(persistence.last_error.contains("expected at least 300"));
}

/// A snapshot store that sleeps during writes to simulate slow DB I/O.
#[derive(Debug, Default)]
struct SlowSnapshotStore {
    write_delay_ms: u64,
}

#[async_trait::async_trait]
impl ChunkSnapshotStore for SlowSnapshotStore {
    async fn write_snapshot(
        &mut self,
        _snapshot: ChunkSnapshotDto,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<(), ChunkSnapshotStoreError> {
        tokio::time::sleep(std::time::Duration::from_millis(self.write_delay_ms)).await;
        Ok(())
    }

    async fn read_snapshot(
        &self,
        _coord: ChunkCoord,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        Ok(None)
    }
}

#[tokio::test]
async fn concurrent_reads_proceed_during_snapshot_persist() {
    use sim_core::persistence::InMemoryMobilitySnapshotStore;
    use std::time::Instant;

    // Build AppState with a slow snapshot store (100 ms per write, 3 chunks = 300 ms total).
    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:app-persist-fail:1").await;
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(SlowSnapshotStore {
            write_delay_ms: 100,
        }),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );

    // Spawn persist — its DB write holds only the snapshot_store mutex,
    // independent of the runtime.
    let state_for_persist = state.clone();
    let persist = tokio::spawn(async move { persist_snapshots_once(&state_for_persist).await });

    // Briefly wait so persist enters its DB-write phase.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Concurrent reads via the lock-free view — these must complete
    // quickly even while persist's DB write is in flight.
    let read_start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..10 {
        let s = state.clone();
        handles.push(tokio::spawn(async move {
            let _ = s.view().load().health.clone();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let read_elapsed = read_start.elapsed();

    assert!(
        read_elapsed < Duration::from_millis(50),
        "reads blocked during persist: took {}ms (expected < 50ms)",
        read_elapsed.as_millis()
    );

    persist.await.unwrap().unwrap();
}

/// A snapshot store that always fails writes to simulate a DB error.
#[derive(Debug, Default)]
struct FailingSnapshotStore;

#[async_trait::async_trait]
impl ChunkSnapshotStore for FailingSnapshotStore {
    async fn write_snapshot(
        &mut self,
        _snapshot: ChunkSnapshotDto,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<(), ChunkSnapshotStoreError> {
        Err(ChunkSnapshotStoreError::unavailable("test failure"))
    }

    async fn read_snapshot(
        &self,
        _coord: ChunkCoord,
        _compatibility: &sim_core::persistence::SnapshotCompatibility,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        Ok(None)
    }
}

#[tokio::test]
async fn subscription_diff_mutation_returns_snapshots_for_added_chunks() {
    let state = AppState::new(SimulationRuntime::new());
    // Wait one tick so the view is populated.
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    state
        .mutations
        .send(crate::runtime_view::Mutation::SubscriptionDiff {
            added: vec![sim_core::ids::ChunkCoord { x: 0, y: 0 }],
            removed: Vec::new(),
            reply: reply_tx,
        })
        .unwrap();
    // Drain happens at the next tick boundary — wait for the reply.
    let snapshots = tokio::time::timeout(TICK_WAIT, reply_rx)
        .await
        .expect("reply within deadline")
        .expect("reply not dropped");
    assert_eq!(snapshots.len(), 1, "expected one snapshot for added chunk");
    let chunk = snapshots[0].chunk.as_ref().expect("chunk coord present");
    assert_eq!(chunk.x, 0);
    assert_eq!(chunk.y, 0);
}

#[tokio::test]
async fn chunk_subscribe_uses_published_view_snapshots_without_waiting_for_tick_reply() {
    let state = state_with_delayed_subscription_reply(Duration::from_millis(650));
    let coord = sim_core::ids::ChunkCoord { x: 0, y: 0 };
    let message = w::ClientMessage {
        body: Some(w::client_message::Body::ChunkSubscribe(w::ChunkSubscribe {
            protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
            coords: vec![w::ChunkCoord {
                x: coord.x,
                y: coord.y,
            }],
        })),
    };
    let mut connection = ConnectionState::new();

    let outgoing = tokio::time::timeout(
        Duration::from_millis(200),
        handle_client_message(&state, &message, &mut connection),
    )
    .await
    .expect("subscribe should not wait for the tick mutation reply");

    assert_eq!(
        outgoing.len(),
        1,
        "subscribe must emit a published snapshot"
    );
    assert!(
        connection.subscription.contains(&coord),
        "slow tick replies must not roll back the subscription"
    );
    match outgoing[0].body.as_ref() {
        Some(w::server_message::Body::MobilityChunkSnapshot(snapshot)) => {
            let chunk = snapshot.chunk.as_ref().expect("snapshot chunk present");
            assert_eq!((chunk.x, chunk.y), (coord.x, coord.y));
        }
        other => panic!("expected mobility chunk snapshot, got {other:?}"),
    }
}

fn state_with_delayed_subscription_reply(delay: Duration) -> AppState {
    use sim_core::persistence::InMemoryMobilitySnapshotStore;

    let runtime = SimulationRuntime::new();
    let initial_view = build_read_view_from_runtime(&runtime, &std::collections::HashMap::new(), 0);
    let (deltas, _) = tokio::sync::broadcast::channel(DELTA_BROADCAST_CAPACITY);
    let (mutation_tx, mut mutation_rx) = tokio::sync::mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Some(mutation) = mutation_rx.recv().await {
            if let crate::runtime_view::Mutation::SubscriptionDiff { added, reply, .. } = mutation {
                tokio::time::sleep(delay).await;
                let snapshots = added
                    .into_iter()
                    .map(|coord| w::MobilityChunkSnapshot {
                        protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
                        world_id: "test-world".into(),
                        tick: 1,
                        chunk: Some(w::ChunkCoord {
                            x: coord.x,
                            y: coord.y,
                        }),
                        agents: Vec::new(),
                        vehicles: Vec::new(),
                    })
                    .collect();
                let _ = reply.send(snapshots);
            }
        }
    });

    AppState {
        deltas,
        card_hands: CardHandStore::memory(),
        auth: AuthVerifier::local_bearer_uuid(),
        snapshot_store: Arc::new(Mutex::new(Box::new(InMemoryChunkSnapshotStore::default()))),
        mobility_snapshot_store: Arc::new(Mutex::new(Box::new(
            InMemoryMobilitySnapshotStore::default(),
        ))),
        economy_snapshot_store: Arc::new(Mutex::new(Box::new(
            InMemoryEconomySnapshotStore::default(),
        ))),
        chunk_channels: Arc::new(DashMap::new()),
        view: Arc::new(arc_swap::ArcSwap::from_pointee(initial_view)),
        mutations: mutation_tx,
        base_world: Arc::new(BaseWorldResponse::from(
            &BaseWorldBundle::load_from_dir(resolve_base_world_path())
                .expect("base world fixture loads"),
        )),
        mobility_liveness: Arc::new(MobilityPersistenceLiveness::new(
            MOBILITY_PERSISTENCE_FRESHNESS_WINDOW,
        )),
        expected_base_world_agents: 1,
    }
}

#[tokio::test]
async fn dropped_reply_channel_does_not_panic() {
    let state = AppState::new(SimulationRuntime::new());
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    drop(reply_rx); // drop receiver before the mutation is processed
    state
        .mutations
        .send(crate::runtime_view::Mutation::SubscriptionDiff {
            added: vec![sim_core::ids::ChunkCoord { x: 0, y: 0 }],
            removed: Vec::new(),
            reply: reply_tx,
        })
        .unwrap();
    // Wait long enough for the tick task to drain the queue. If a panic
    // bubbled up, the spawned task would have died — exercise the view a
    // couple of ticks later to detect that.
    let t0 = state.view().load().mobility_tick;
    let t1 = wait_for_tick_past(&state, t0, TICK_WAIT).await;
    assert!(t1 > t0, "tick task must still be alive after dropped reply");
}

#[tokio::test]
async fn snapshot_write_failure_preserves_dirty_state() {
    use sim_core::persistence::InMemoryMobilitySnapshotStore;

    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:app-persist-failure:1").await;
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(FailingSnapshotStore),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );

    // First persist attempt must fail because the store always errors.
    let result = persist_snapshots_once(&state).await;
    assert!(result.is_err(), "persist should propagate the store error");

    // The chunks must still be dirty — mark_chunk_snapshots_persisted must
    // NOT have been called after a failed write. We verify by requesting a
    // fresh CollectPersistData — the returned snapshot list must still
    // include dirty chunks.
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    state
        .mutations
        .send(crate::runtime_view::Mutation::CollectPersistData { reply: reply_tx })
        .unwrap();
    let payload = tokio::time::timeout(TICK_WAIT, reply_rx)
        .await
        .expect("reply within deadline")
        .expect("reply not dropped");
    assert!(
        !payload.chunk_snapshots.is_empty(),
        "snapshot write failure must not mark chunks persisted (snapshots remain dirty)"
    );
}

#[tokio::test]
async fn persist_writes_economy_snapshot_to_store() {
    use sim_core::economy::{AccountBook, EconomicActorId, Money, MoneyAccount};
    use sim_core::persistence::{
        InMemoryChunkSnapshotStore, InMemoryMobilitySnapshotStore, SnapshotCompatibility,
    };

    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:econ-persist:1").await;
    // Seed an account so the economy snapshot is non-trivial.
    runtime.world.resource_mut::<AccountBook>().accounts.insert(
        EconomicActorId(1),
        MoneyAccount {
            available: Money(500),
            locked: Money(0),
        },
    );

    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    persist_snapshots_once(&state).await.unwrap();

    let store = state.economy_snapshot_store();
    let store = store.lock().await;
    let compat = SnapshotCompatibility::new(
        base_world.world_id().to_string(),
        base_world.snapshot_compatibility().base_world_schema_version,
    );
    let got = store.read(base_world.world_id(), &compat).await.unwrap();
    assert!(got.is_some(), "economy snapshot persisted");
    let (_tick, snap) = got.unwrap();
    assert!(
        snap.accounts.iter().any(|(a, _)| *a == EconomicActorId(1)),
        "seeded account present in persisted economy snapshot"
    );
}

#[tokio::test]
async fn economy_endpoint_returns_json_snapshot() {
    use sim_core::economy::{
        AccountBook, EconomicActorId, EconomyPersistSnapshot, Money, MoneyAccount,
    };

    let mut runtime = SimulationRuntime::new();
    runtime.world.resource_mut::<AccountBook>().accounts.insert(
        EconomicActorId(5),
        MoneyAccount {
            available: Money(1234),
            locked: Money(0),
        },
    );
    let state = AppState::new(runtime);
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    // Call the handler via the mutation round-trip (app/tests.rs is a child
    // module of app/mod.rs, so the private `mutations` field is in scope).
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .mutations
        .send(crate::runtime_view::Mutation::CollectEconomySnapshot { reply: tx })
        .unwrap();
    let snap = rx.await.unwrap();
    let bytes = serde_json::to_vec(&snap).unwrap();
    let decoded: EconomyPersistSnapshot = serde_json::from_slice(&bytes).unwrap();
    assert!(
        decoded
            .accounts
            .iter()
            .any(|(a, acc)| *a == EconomicActorId(5) && acc.available == Money(1234))
    );
}

#[cfg(test)]
mod cors_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn router_with_origins(origins: &[&str]) -> axum::Router {
        let owned: Vec<String> = origins.iter().map(|o| o.to_string()).collect();
        let cors = cors_layer(&owned).expect("valid origins");
        axum::Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .layer(cors)
    }

    #[tokio::test]
    async fn allowed_origin_is_reflected() {
        let app = router_with_origins(&["http://127.0.0.1:5173"]);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap().to_string()),
            Some("http://127.0.0.1:5173".to_string())
        );
    }

    #[tokio::test]
    async fn disallowed_origin_gets_no_cors_header() {
        let app = router_with_origins(&["http://127.0.0.1:5173"]);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "https://evil.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.headers().get("access-control-allow-origin").is_none());
    }

    #[tokio::test]
    async fn empty_allow_list_is_fail_closed() {
        let app = router_with_origins(&[]);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.headers().get("access-control-allow-origin").is_none());
    }
}
