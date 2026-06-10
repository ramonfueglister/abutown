use super::*;
use abutown_protocol::ChunkSnapshotDto;
use sim_core::ids::ChunkCoord;
use sim_core::persistence::{
    ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryEconomyEventStore,
    InMemoryEconomySnapshotStore, MobilitySnapshotStore, MobilitySnapshotStoreError,
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
        Box::new(InMemoryEconomyEventStore::default()),
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

#[derive(Debug)]
struct RecordingEconomyEventStore {
    recorded: Arc<std::sync::Mutex<Vec<(u64, sim_core::economy::EconomyEvent)>>>,
}

#[async_trait::async_trait]
impl sim_core::persistence::EconomyEventStore for RecordingEconomyEventStore {
    async fn append(
        &mut self,
        _world_id: &str,
        tick: u64,
        events: &[sim_core::economy::EconomyEvent],
    ) -> Result<(), sim_core::persistence::EconomyEventStoreError> {
        self.recorded
            .lock()
            .unwrap()
            .extend(events.iter().map(|e| (tick, e.clone())));
        Ok(())
    }

    async fn prune(
        &mut self,
        _world_id: &str,
        _keep_last: u64,
    ) -> Result<u64, sim_core::persistence::EconomyEventStoreError> {
        Ok(0)
    }
}

#[tokio::test]
async fn economy_audit_flush_appends_pending_then_commit_prevents_reappend() {
    use sim_core::economy::{EconomicActorId, EconomyEvent, MarketId, Money};

    // Sentinel events with actor ids in a reserved high range the economy systems
    // never generate, so the assertions are robust to any organic ledger activity
    // from the live tick task running alongside the test. WagePaid: an
    // audit-DURABLE variant, or the flush filter would (correctly) drop it.
    const SENTINEL_BASE: u64 = 900_000;
    fn sentinel_count(recorded: &Arc<std::sync::Mutex<Vec<(u64, EconomyEvent)>>>) -> usize {
        recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| {
                matches!(e, EconomyEvent::WagePaid { firm, .. } if firm.0 >= SENTINEL_BASE)
            })
            .count()
    }

    let mut runtime = SimulationRuntime::new();
    runtime.push_ledger_events_for_test(
        (0..3)
            .map(|i| EconomyEvent::WagePaid {
                firm: EconomicActorId(SENTINEL_BASE + i),
                market: MarketId(1),
                amount: Money(1),
            })
            .collect(),
    );

    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        Box::new(RecordingEconomyEventStore {
            recorded: Arc::clone(&recorded),
        }),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );

    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    // First flush appends the pending ledger tail, which includes the 3 sentinels.
    persist_snapshots_once(&state).await.unwrap();
    assert_eq!(
        sentinel_count(&recorded),
        3,
        "first flush appends the pending ledger tail"
    );

    // Let the fire-and-forget CommitLedgerAudit mutation apply on the next tick.
    let tick1 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick1, TICK_WAIT).await;

    // Second flush must not re-append the already-committed sentinels: the commit
    // advanced the audit cursor past them.
    persist_snapshots_once(&state).await.unwrap();
    assert_eq!(
        sentinel_count(&recorded),
        3,
        "commit advanced the audit cursor; committed events are not re-appended"
    );
}

/// Records every append attempt, then fails — to prove the flush is best-effort.
#[derive(Debug)]
struct FailingEconomyEventStore {
    attempts: Arc<std::sync::Mutex<Vec<(u64, sim_core::economy::EconomyEvent)>>>,
}

#[async_trait::async_trait]
impl sim_core::persistence::EconomyEventStore for FailingEconomyEventStore {
    async fn append(
        &mut self,
        _world_id: &str,
        tick: u64,
        events: &[sim_core::economy::EconomyEvent],
    ) -> Result<(), sim_core::persistence::EconomyEventStoreError> {
        self.attempts
            .lock()
            .unwrap()
            .extend(events.iter().map(|e| (tick, e.clone())));
        Err(sim_core::persistence::EconomyEventStoreError::unavailable(
            "simulated audit store outage",
        ))
    }

    async fn prune(
        &mut self,
        _world_id: &str,
        _keep_last: u64,
    ) -> Result<u64, sim_core::persistence::EconomyEventStoreError> {
        Err(sim_core::persistence::EconomyEventStoreError::unavailable(
            "simulated audit store outage",
        ))
    }
}

#[tokio::test]
async fn economy_audit_flush_failure_is_best_effort_and_retries() {
    use sim_core::economy::{EconomicActorId, EconomyEvent, MarketId, Money};

    const SENTINEL_BASE: u64 = 910_000;
    fn sentinel_count(attempts: &Arc<std::sync::Mutex<Vec<(u64, EconomyEvent)>>>) -> usize {
        attempts
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| {
                matches!(e, EconomyEvent::WagePaid { firm, .. } if firm.0 >= SENTINEL_BASE)
            })
            .count()
    }

    let mut runtime = SimulationRuntime::new();
    runtime.push_ledger_events_for_test(
        (0..2)
            .map(|i| EconomyEvent::WagePaid {
                firm: EconomicActorId(SENTINEL_BASE + i),
                market: MarketId(1),
                amount: Money(1),
            })
            .collect(),
    );

    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let attempts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        Box::new(FailingEconomyEventStore {
            attempts: Arc::clone(&attempts),
        }),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );

    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    // A failed audit append must not fail the persistence cycle (best-effort).
    persist_snapshots_once(&state).await.unwrap();
    assert_eq!(
        sentinel_count(&attempts),
        2,
        "first flush attempts the pending events"
    );

    let tick1 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick1, TICK_WAIT).await;

    // The failed append sent no commit, so the cursor never advanced: the same
    // sentinels are attempted again on the next cycle.
    persist_snapshots_once(&state).await.unwrap();
    assert_eq!(
        sentinel_count(&attempts),
        4,
        "a failed flush does not advance the cursor; events are retried"
    );
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
async fn failing_mobility_write_marks_health_stale_with_redacted_error() {
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
        Box::new(InMemoryEconomyEventStore::default()),
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
        "stale persistence (never succeeded) should make /health unhealthy"
    );
    assert_eq!(persistence.status, w::PersistenceHealthStatus::Stale as i32);
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
async fn degraded_persistence_keeps_health_ok() {
    use sim_core::persistence::InMemoryMobilitySnapshotStore;

    // Scenario: one successful persist followed by >PERSIST_FAILURE_TOLERANCE failures
    // while the freshness window has not expired → status Degraded, health.ok still true.
    let mut runtime = SimulationRuntime::new();
    mutate_runtime_tile(&mut runtime, "command:app-persist-health-degraded:1").await;
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");

    // First pass: succeed with an InMemory store to record a success.
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        Box::new(InMemoryEconomyEventStore::default()),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;
    persist_snapshots_once(&state).await.unwrap();

    // Directly exercise the liveness tracker: record a prior success, then 3 failures.
    let liveness = state.mobility_liveness();
    let a = liveness.begin_attempt("abutopia", 99, SystemTime::now());
    liveness.record_success(a, SystemTime::now());
    for _ in 0..3 {
        let a = liveness.begin_attempt("abutopia", 100, SystemTime::now());
        liveness.record_failure(a, "transient error", SystemTime::now());
    }

    let health = health_response_for_state(&state);
    let persistence = health.persistence.expect("persistence health present");
    assert!(
        health.ok,
        "degraded persistence (prior success + >tolerance failures, still fresh) should keep /health OK"
    );
    assert_eq!(
        persistence.status,
        w::PersistenceHealthStatus::Degraded as i32
    );
}

#[tokio::test]
async fn health_degrades_when_published_mobility_is_empty() {
    let state = AppState::new(SimulationRuntime::new());
    let mut view = state.view().load().as_ref().clone();
    view.mobility_full_dto.agents.clear();
    state.view().store(Arc::new(view));

    let health = health_response_for_state(&state);

    assert!(
        !health.ok,
        "health must fail when the published mobility view is empty (0 agents)"
    );
}

#[tokio::test]
async fn below_seed_count_population_is_healthy_empty_is_not() {
    // A living population below the base-world seed count is valid — the
    // guard must only reject a completely empty world (0 agents).
    let make_state_with_agents = |n: usize| {
        let state = AppState::new(SimulationRuntime::new());
        let mut view = state.view().load().as_ref().clone();
        view.mobility_full_dto.agents = (0..n)
            .map(|i| w::AgentMobility {
                id: format!("agent-{i}"),
                ..Default::default()
            })
            .collect();
        state.view().store(Arc::new(view));
        state
    };

    let state_285 = make_state_with_agents(285);
    assert!(
        health_response_for_state(&state_285).ok,
        "285 agents (<300 seed) but >0 must be healthy"
    );

    let state_0 = make_state_with_agents(0);
    assert!(
        !health_response_for_state(&state_0).ok,
        "0 agents must be unhealthy"
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
async fn persist_snapshots_once_rejects_empty_mobility_snapshots() {
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
                    economy_audit_tick: 0,
                    economy_audit_pending: Vec::new(),
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
        economy_event_store: Arc::new(Mutex::new(Box::new(InMemoryEconomyEventStore::default()))),
        chunk_channels: Arc::new(DashMap::new()),
        view: Arc::new(arc_swap::ArcSwap::from_pointee(
            build_read_view_from_runtime(&runtime, &std::collections::HashMap::new(), None),
        )),
        mutations: mutation_tx,
        base_world: Arc::new(BaseWorldResponse::from(&base_world)),
        mobility_liveness: Arc::new(MobilityPersistenceLiveness::new(
            MOBILITY_PERSISTENCE_FRESHNESS_WINDOW,
        )),
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
    assert_eq!(persistence.status, w::PersistenceHealthStatus::Stale as i32);
    assert!(persistence.last_error.contains("0 agents"));
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
        Box::new(InMemoryEconomyEventStore::default()),
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
    let initial_view =
        build_read_view_from_runtime(&runtime, &std::collections::HashMap::new(), None);
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
        economy_event_store: Arc::new(Mutex::new(Box::new(InMemoryEconomyEventStore::default()))),
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
    }
}

/// Per-tick cost breakdown. Not a real assertion test — an evidence-gathering
/// harness for the 2026-06-10 "tick is ~250ms" investigation. Run with:
///   scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml \
///     -p sim-server profile_tick_phases -- --ignored --nocapture
#[test]
#[ignore = "profiling harness; run explicitly with --ignored --nocapture"]
fn profile_tick_phases() {
    use std::time::Instant;

    fn median_ms(mut samples: Vec<f64>) -> f64 {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        samples[samples.len() / 2]
    }
    fn time_n<F: FnMut()>(k: usize, mut f: F) -> f64 {
        let mut s = Vec::with_capacity(k);
        for _ in 0..k {
            let t = Instant::now();
            f();
            s.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        median_ms(s)
    }

    let mut runtime = SimulationRuntime::new_from_base_world_dir(resolve_base_world_path())
        .expect("base world fixture loads");

    // Warm up: get agents mid-route and chunks dirtied like a live world.
    for _ in 0..100 {
        let _ = runtime.tick_world_mobility();
    }

    let world_id = runtime.world_id_for_persist().clone();
    let loaded = runtime.world_summary().loaded_chunks.len();
    let agents = runtime.mobility_snapshot().agents.len();
    const K: usize = 40;

    // --- tick_world_mobility (the actual sim step; mutates) ---
    let t_tick = time_n(K, || {
        let _ = runtime.tick_world_mobility();
    });

    // Freeze on a post-tick state and measure each read sub-phase repeatedly.
    let per_chunk = runtime.tick_world_mobility();
    let mobility = runtime.mobility();
    let mobility_tick = runtime.mobility_tick();

    let t_full = time_n(K, || {
        let v = build_read_view_from_runtime(&runtime, &per_chunk, None);
        std::hint::black_box(v);
    });
    let prev_view = build_read_view_from_runtime(&runtime, &per_chunk, None);
    let t_incremental = time_n(K, || {
        let v = build_read_view_from_runtime(&runtime, &per_chunk, Some(&prev_view));
        std::hint::black_box(v);
    });
    let t_world_summary = time_n(K, || {
        let s = runtime.world_summary();
        std::hint::black_box(world_summary_dto_to_proto(&s));
    });
    let summary = runtime.world_summary();
    let t_chunk_loop = time_n(K, || {
        for coord_dto in summary.loaded_chunks.iter() {
            let coord = sim_core::ids::ChunkCoord {
                x: coord_dto.x,
                y: coord_dto.y,
            };
            if let Some(snap) = runtime.chunk_snapshot(coord) {
                std::hint::black_box(chunk_snapshot_dto_to_proto(&snap));
            }
            let mob = sim_core::mobility::api::build_mobility_chunk_snapshot(mobility, coord);
            std::hint::black_box(chunk_snapshot_to_dto(
                &mob,
                mobility,
                &world_id,
                mobility_tick,
            ));
        }
    });
    let t_mobility_full = time_n(K, || {
        let m = runtime.mobility_snapshot();
        std::hint::black_box(mobility_snapshot_dto_to_proto(&m));
    });
    let t_economy = time_n(K, || {
        std::hint::black_box(build_economy_snapshot(mobility, &world_id, mobility_tick));
    });
    let t_subscriber_counts = time_n(K, || {
        std::hint::black_box(sim_core::mobility::api::chunk_subscriber_counts_snapshot(
            mobility,
        ));
    });

    eprintln!(
        "=== tick-phase profile (loaded_chunks={loaded}, agents={agents}, K={K}, median ms) ==="
    );
    eprintln!("  tick_world_mobility .......... {t_tick:8.3}");
    eprintln!("  build_read_view (cold/full) .. {t_full:8.3}");
    eprintln!("  build_read_view (incremental)  {t_incremental:8.3}");
    eprintln!("    └ world_summary ............ {t_world_summary:8.3}");
    eprintln!(
        "    └ legacy per-chunk loop .... {t_chunk_loop:8.3}  (pre-fix pattern, for comparison)"
    );
    eprintln!("    └ mobility_full_dto ........ {t_mobility_full:8.3}");
    eprintln!("    └ economy_snapshot ......... {t_economy:8.3}");
    eprintln!("    └ subscriber_counts ........ {t_subscriber_counts:8.3}");
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
        Box::new(InMemoryEconomyEventStore::default()),
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
        Box::new(InMemoryEconomyEventStore::default()),
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
        base_world
            .snapshot_compatibility()
            .base_world_schema_version,
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

#[tokio::test]
async fn read_view_economy_snapshot_exposes_four_markets_and_known_goods() {
    // After one tick, the published read view must carry a pre-built
    // EconomySnapshot with the 4 demo markets seeded from the abutopia bundle
    // and at least the three opening-priced goods (market 9002 TOOLS/FOOD,
    // market 9004 FOOD).
    let state = AppState::new(SimulationRuntime::new());
    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    let view = state.view().load();
    assert_eq!(
        view.economy.markets.len(),
        4,
        "economy snapshot must expose exactly 4 demo markets"
    );
    // The three opening-priced goods (market_id, good_id): (9002,4), (9002,1), (9004,1).
    let goods: std::collections::HashSet<(u32, u32)> = view
        .economy
        .goods
        .iter()
        .map(|g| (g.market_id, g.good_id))
        .collect();
    assert!(
        goods.contains(&(9002, 4)),
        "view.economy.goods must include (market=9002, good=TOOLS=4)"
    );
    assert!(
        goods.contains(&(9002, 1)),
        "view.economy.goods must include (market=9002, good=FOOD=1)"
    );
    assert!(
        goods.contains(&(9004, 1)),
        "view.economy.goods must include (market=9004, good=FOOD=1)"
    );
}

#[test]
fn economy_snapshot_includes_flow_rates() {
    // build_economy_snapshot must ship the FlowRateEwma resource as the
    // snapshot's `flows` field, one entry per (src, dst, good) edge.
    let mut runtime = SimulationRuntime::new();
    runtime
        .world
        .insert_resource(sim_core::economy::FlowRateEwma(
            [(
                (
                    sim_core::economy::MarketId(9003),
                    sim_core::economy::MarketId(9004),
                    sim_core::economy::GOOD_FOOD,
                ),
                sim_core::economy::Money(250),
            )]
            .into_iter()
            .collect(),
        ));
    let world_id = runtime.world_id_for_persist().clone();
    let snapshot = build_economy_snapshot(&runtime.world, &world_id, 0);
    assert_eq!(snapshot.flows.len(), 1);
    let flow = &snapshot.flows[0];
    assert_eq!(
        (
            flow.src_market_id,
            flow.dst_market_id,
            flow.good_id,
            flow.rate
        ),
        (9003, 9004, 1, 250)
    );
}

#[test]
fn economy_snapshot_includes_producers() {
    // build_economy_snapshot must ship one EconomyProducer entry per InputPools
    // key: recipe (in/out good + qty), firm cash, participation bound, and the
    // working-capital target (0 while the bound is undiscovered — the dividend
    // path's conservative semantics).
    use sim_core::economy::{EconomicActorId, InputPools, Money, capita::CapitaFactor};

    let mut runtime = SimulationRuntime::new();
    let world_id = runtime.world_id_for_persist().clone();

    // Seed state: producer 8031's participation bound starts at ZERO (discovered
    // by the order-generation pass) → max_bid = 0 and wc_target = 0.
    let snapshot = build_economy_snapshot(&runtime.world, &world_id, 0);
    assert_eq!(
        snapshot.producers.len(),
        1,
        "abutopia markets.json seeds exactly one producer (8031)"
    );
    let p = &snapshot.producers[0];
    assert_eq!(p.actor_id, 8031);
    assert_eq!(p.market_id, 9001);
    assert_eq!(
        (p.in_good, p.out_good, p.in_qty, p.out_qty),
        (2, 4, 10, 10),
        "recipe: 10 WOOD → 10 TOOLS"
    );
    // opening_cash 1_000_000 × seed capita factor 30 (300 agents / baseline 10).
    assert_eq!(p.retained_earnings, 30_000_000);
    assert_eq!(
        (p.max_bid, p.wc_target),
        (0, 0),
        "unpriced pool → both zero"
    );

    // Priced path: write a positive bound + a pinned factor, rebuild.
    runtime
        .world
        .resource_mut::<InputPools>()
        .0
        .get_mut(&EconomicActorId(8031))
        .expect("producer 8031 seeded from markets.json")
        .max_price = Money(400);
    runtime.world.insert_resource(CapitaFactor(30));
    let snapshot = build_economy_snapshot(&runtime.world, &world_id, 0);
    let p = &snapshot.producers[0];
    assert_eq!(p.max_bid, 400);
    // wc_target = max_price · (batches_target·in_qty·factor) / ECONOMY_SCALE
    //           = 400 · (2·10·30) / 1000 = 240 (same arithmetic as settlement).
    assert_eq!(p.wc_target, 240);
}

/// Verify that `build_economy_snapshot` always populates the `vitals` field.
///
/// The fixture is a fully-seeded `SimulationRuntime::new()` world (abutopia bundle
/// with 300 agents, capita_baseline=10, so capita_factor=30).  The economy is seeded
/// by `seed_from_markets_layer`:
///   - 3 demand actors + producer 8031 each receive
///     `opening_cash=1_000_000 × factor=30 = 30_000_000`
///   - Total seeded money = 4 × 30_000_000 = 120_000_000
///
/// At seeding time no ticks have run so no supply/demand actors have traded: the
/// full 120_000_000 sits in `available` balances.  The route stats are all-zero because
/// the route-assignment system hasn't been invoked, and there are no
/// CitizenEconomicTargets yet (attribution runs later).
#[test]
fn economy_snapshot_carries_vitals() {
    use sim_core::economy::AccountBook;

    let runtime = SimulationRuntime::new();
    let world = runtime.mobility();

    // Sanity: the AccountBook exists and has the expected seeded total.
    // 300 agents / 10 capita_baseline = factor 30.
    // (3 demand specs + producer 8031) × 1_000_000 × 30.
    let expected_total_money: i64 = 4 * 1_000_000 * 30;
    let actual_total = world
        .resource::<AccountBook>()
        .total_money()
        .expect("AccountBook summation must not overflow on a freshly seeded world")
        .0;
    assert_eq!(
        actual_total, expected_total_money,
        "seeded total_money must equal 3 × opening_cash × capita_factor"
    );

    let world_id = abutown_protocol::WorldId("abutopia".to_string());
    let snapshot = build_economy_snapshot(world, &world_id, 7);

    let vitals = snapshot
        .vitals
        .expect("build_economy_snapshot must always populate vitals");
    assert_eq!(
        vitals.total_money, expected_total_money,
        "vitals.total_money must match the AccountBook sum"
    );
    // No CitizenEconomicTargets written yet (attribution runs during ticks).
    assert_eq!(
        vitals.routed_citizens, 0,
        "no routed citizens before first tick"
    );
    // 300 agents seeded from the abutopia bundle.
    assert_eq!(
        vitals.population, 300,
        "300 agents seeded from abutopia spawns.json"
    );
    // Route-assignment stats are zero before any tick runs.
    assert_eq!(vitals.routes_assigned, 0);
    assert_eq!(vitals.routes_failed, 0);
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

mod tick_pacing_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Regression test for the 2026-06-09 production outage: once per-tick
    /// work exceeds SIMULATION_TICK_INTERVAL the interval is permanently
    /// overdue, so `Interval::tick()` resolves immediately on every call and
    /// the tick loop never returns Pending. On a 1-vCPU machine (Fly
    /// shared-cpu-1x → `#[tokio::main]` = multi_thread with ONE worker) that
    /// starved the HTTP accept loop indefinitely: the kernel kept completing
    /// TCP handshakes into the listen backlog while no handler ever answered,
    /// so Fly's /health check timed out ("awaiting headers") for 10+ hours
    /// while the sim kept ticking and logging. pace_tick must force a
    /// scheduler pass per iteration so concurrent tasks (the accept loop)
    /// always make progress.
    ///
    /// The runtime flavor matters: on `current_thread` (the tokio::test
    /// default) the starvation does NOT reproduce, so this test builds the
    /// production configuration explicitly.
    ///
    /// Measured against the unfixed pacing on multi_thread(1): an overdue
    /// `Interval::tick()` still yields *sporadically* (timer-wheel artifacts,
    /// ~4 yields per 100 iterations, irregular). At production tick cost
    /// (~250 ms) that is one scheduler pass every 6–15 s, while a request
    /// needs several polls and Fly's check times out at 5 s — hence zero
    /// healthy checks for 10+ hours. The assertion therefore demands a yield
    /// per iteration (observer advancing in lockstep), not mere nonzero
    /// progress, which sporadic yields would satisfy.
    #[test]
    fn saturated_tick_loop_still_schedules_other_tasks() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build single-worker runtime");

        let progress = Arc::new(AtomicU32::new(0));
        let observed = Arc::clone(&progress);
        rt.spawn(async move {
            loop {
                observed.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        });

        // Mirror tick_loop's pacing with per-tick work that always overruns
        // the interval (the production failure mode). The saturated loop runs
        // a bounded number of iterations and reports the observer's progress
        // over exactly that window, so a broken pacing fails fast instead of
        // hanging the test binary.
        const ITERATIONS: u32 = 100;
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let watched = Arc::clone(&progress);
        rt.spawn(async move {
            let mut ticker = simulation_ticker(Duration::from_millis(1));
            ticker.tick().await;
            std::thread::sleep(Duration::from_millis(5)); // enter permanent overrun
            watched.store(0, Ordering::SeqCst);
            for _ in 0..ITERATIONS {
                pace_tick(&mut ticker).await;
                std::thread::sleep(Duration::from_millis(2)); // tick work > interval
            }
            let _ = done_tx.send(watched.load(Ordering::SeqCst));
        });

        let observed_progress = done_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("saturated loop must finish its bounded iterations");
        rt.shutdown_background();

        assert!(
            observed_progress >= ITERATIONS / 2,
            "tick pacing starved concurrent tasks: a saturated tick loop must yield to the \
             scheduler every iteration, but the observer advanced only {observed_progress} times \
             across {ITERATIONS} iterations"
        );
    }

    /// Under sustained overload missed ticks must not accrue catch-up debt:
    /// Burst (the tokio default) replays every missed tick back-to-back once
    /// load drops, sprinting sim-time at max CPU until the backlog drains.
    /// Delay reschedules from the present, so sim-time degrades gracefully.
    #[tokio::test]
    async fn simulation_ticker_does_not_accrue_catchup_debt() {
        let ticker = simulation_ticker(Duration::from_millis(100));
        assert_eq!(
            ticker.missed_tick_behavior(),
            tokio::time::MissedTickBehavior::Delay,
            "simulation ticker must use Delay so overload slows sim-time instead of queueing a burst backlog"
        );
    }
}

/// A1 (2026-06-10 tick-cost design): the read view must NOT rebuild tile
/// snapshots for chunks whose ChunkVersion is unchanged — it reuses the
/// previous view's Arc. A mutated chunk gets a fresh snapshot; everyone
/// else stays pointer-identical.
#[tokio::test]
async fn read_view_reuses_unchanged_tile_snapshots_and_rebuilds_dirty_ones() {
    let empty = std::collections::HashMap::new();
    let mut runtime = SimulationRuntime::new();

    let view1 = build_read_view_from_runtime(&runtime, &empty, None);
    assert!(
        !view1.chunk_snapshots.is_empty(),
        "fixture world must have loaded chunks"
    );

    // A plain mobility tick does not touch tiles: every tile snapshot must be
    // reused (pointer-equal), not rebuilt.
    let _ = runtime.tick_world_mobility();
    let view2 = build_read_view_from_runtime(&runtime, &empty, Some(&view1));
    for (coord, snap) in &view1.chunk_snapshots {
        let snap2 = view2
            .chunk_snapshots
            .get(coord)
            .expect("chunk present in next view");
        assert!(
            Arc::ptr_eq(snap, snap2),
            "unchanged chunk {coord:?} must reuse the cached snapshot Arc"
        );
    }

    // Mutating a tile bumps that chunk's version: it must be rebuilt, while
    // all other chunks keep their cached Arc.
    let dirty = ChunkCoord { x: 0, y: 0 };
    mutate_runtime_tile(&mut runtime, "command:view-reuse:1").await;
    let view3 = build_read_view_from_runtime(&runtime, &empty, Some(&view2));
    let before = view2.chunk_snapshots.get(&dirty).expect("dirty chunk");
    let after = view3.chunk_snapshots.get(&dirty).expect("dirty chunk");
    assert!(
        !Arc::ptr_eq(before, after),
        "mutated chunk must get a freshly built snapshot"
    );
    assert!(
        after.chunk_version > before.chunk_version,
        "rebuilt snapshot must carry the bumped version"
    );
    for (coord, snap) in &view2.chunk_snapshots {
        if *coord == dirty {
            continue;
        }
        assert!(
            Arc::ptr_eq(snap, view3.chunk_snapshots.get(coord).expect("present")),
            "non-mutated chunk {coord:?} must still reuse its cached Arc"
        );
    }
}

/// B1 (2026-06-10 design): the audit flush appends only the DURABLE subset of
/// pending events, yet commits the FULL pending count — transient events are
/// consumed (never re-pended, ledger still trims) without ever hitting the DB.
#[tokio::test]
async fn economy_audit_flush_writes_only_durable_events_but_consumes_all() {
    use sim_core::economy::{EconomicActorId, EconomyEvent, MarketId, Money};

    const SENTINEL_BASE: u64 = 920_000;
    fn sentinel_events(recorded: &Arc<std::sync::Mutex<Vec<(u64, EconomyEvent)>>>) -> Vec<String> {
        recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| match e {
                EconomyEvent::WagePaid { firm, .. } => firm.0 >= SENTINEL_BASE,
                EconomyEvent::CashLocked { actor, .. } => actor.0 >= SENTINEL_BASE,
                _ => false,
            })
            .map(|(_, e)| e.event_type().to_string())
            .collect()
    }

    let mut runtime = SimulationRuntime::new();
    runtime.push_ledger_events_for_test(vec![
        EconomyEvent::CashLocked {
            actor: EconomicActorId(SENTINEL_BASE),
            amount: Money(1),
        },
        EconomyEvent::WagePaid {
            firm: EconomicActorId(SENTINEL_BASE + 1),
            market: MarketId(1),
            amount: Money(5),
        },
        EconomyEvent::CashLocked {
            actor: EconomicActorId(SENTINEL_BASE + 2),
            amount: Money(2),
        },
    ]);

    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
        .expect("base world bundle present for test");
    let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        Box::new(RecordingEconomyEventStore {
            recorded: Arc::clone(&recorded),
        }),
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    );

    let tick0 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick0, TICK_WAIT).await;

    // First flush: only the durable WagePaid sentinel reaches the store.
    persist_snapshots_once(&state).await.unwrap();
    assert_eq!(
        sentinel_events(&recorded),
        vec!["wage_paid".to_string()],
        "transient sentinels must be filtered from the durable append"
    );

    // Let the fire-and-forget CommitLedgerAudit apply, then flush again: the
    // cursor advanced past the transient sentinels too — nothing re-appends.
    let tick1 = state.view().load().mobility_tick;
    wait_for_tick_past(&state, tick1, TICK_WAIT).await;
    persist_snapshots_once(&state).await.unwrap();
    assert_eq!(
        sentinel_events(&recorded),
        vec!["wage_paid".to_string()],
        "transient events are consumed, not retried"
    );
}
