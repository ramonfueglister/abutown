use std::{sync::Arc, time::Duration};

use abutown_protocol::{
    ChunkSnapshotDto, ClientCommandDto, ClientMessageDto, CommandResponseDto, HealthResponse,
    MobilityChunkDeltaDto, MobilityDeltaDto, MobilitySnapshotDto, ServerMessageDto, WorldSummaryDto,
};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use dashmap::DashMap;
use sim_core::{
    ids::ChunkCoord,
    persistence::{
        ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryChunkSnapshotStore,
        InMemoryMobilitySnapshotStore, MobilitySnapshotStore,
    },
};
use tokio::sync::{Mutex, RwLock, broadcast};
use tower_http::cors::CorsLayer;

use crate::{
    card_hand::{
        AuthVerifier, CardHandError, CardHandResponse, CardHandStore, SaveCardHandRequest,
        card_definitions,
    },
    config::ServerConfig,
    postgres_events::PostgresWorldEventStore,
    postgres_mobility::PostgresMobilitySnapshotStore,
    postgres_snapshots::PostgresChunkSnapshotStore,
    runtime::SimulationRuntime,
};

const DELTA_BROADCAST_CAPACITY: usize = 64;
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_millis(100);
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);
const CITY_NETWORK_DEFAULT_PATH: &str = "data/city/zurich-network.json";

fn resolve_city_network_path() -> String {
    std::env::var("ABUTOWN_CITY_NETWORK_PATH")
        .unwrap_or_else(|_| CITY_NETWORK_DEFAULT_PATH.to_string())
}

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<RwLock<SimulationRuntime>>,
    deltas: broadcast::Sender<ServerMessageDto>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
    snapshot_store: Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>>,
    #[allow(dead_code)]
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>>,
}

impl AppState {
    pub fn new(runtime: SimulationRuntime) -> Self {
        Self::new_with_stores(
            runtime,
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            CardHandStore::memory(),
            AuthVerifier::local_bearer_uuid(),
        )
    }

    pub fn new_with_card_hands(
        runtime: SimulationRuntime,
        card_hands: CardHandStore,
        auth: AuthVerifier,
    ) -> Self {
        Self::new_with_stores(
            runtime,
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            card_hands,
            auth,
        )
    }

    pub fn new_with_stores(
        runtime: SimulationRuntime,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send + Sync>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send + Sync>,
        card_hands: CardHandStore,
        auth: AuthVerifier,
    ) -> Self {
        let (deltas, _) = broadcast::channel(DELTA_BROADCAST_CAPACITY);
        Self {
            runtime: Arc::new(RwLock::new(runtime)),
            deltas,
            card_hands,
            auth,
            snapshot_store: Arc::new(Mutex::new(snapshot_store)),
            mobility_snapshot_store: Arc::new(Mutex::new(mobility_snapshot_store)),
            chunk_channels: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn runtime(&self) -> Arc<RwLock<SimulationRuntime>> {
        Arc::clone(&self.runtime)
    }

    fn snapshot_store(&self) -> Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>> {
        Arc::clone(&self.snapshot_store)
    }

    fn mobility_snapshot_store(&self) -> Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>> {
        Arc::clone(&self.mobility_snapshot_store)
    }

    #[allow(dead_code)]
    pub(crate) fn chunk_channels(&self) -> Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>> {
        Arc::clone(&self.chunk_channels)
    }

    /// Read a chunk snapshot directly from the snapshot store.
    /// Used by tests and diagnostic tooling; not on the hot path.
    pub async fn stored_chunk_snapshot(
        &self,
        coord: ChunkCoord,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        let store = self.snapshot_store();
        let store = store.lock().await;
        store.read_snapshot(coord).await
    }

    fn subscribe_deltas(&self) -> broadcast::Receiver<ServerMessageDto> {
        self.deltas.subscribe()
    }

    fn spawn_delta_loop(&self, tick_interval: Duration) {
        let runtime = self.runtime();
        let deltas = self.deltas.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_interval);
            interval.tick().await;
            loop {
                interval.tick().await;
                let messages = {
                    let mut runtime = runtime.write().await;
                    runtime.next_server_messages()
                };
                for message in messages {
                    let _ = deltas.send(message);
                }
            }
        });
    }

    fn spawn_snapshot_loop(&self, snapshot_interval: Duration) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(snapshot_interval);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(error) = persist_snapshots_once(&state).await {
                    tracing::warn!(%error, "failed to persist chunk snapshots");
                }
            }
        });
    }
}

pub fn build_app() -> Router {
    let runtime = match sim_core::city_network::CityNetwork::from_path(resolve_city_network_path())
    {
        Ok(network) => SimulationRuntime::new_from_network(&network),
        Err(_) => SimulationRuntime::new(),
    };
    build_app_with_runtime(runtime)
}

pub async fn build_app_from_env() -> anyhow::Result<Router> {
    let _ = dotenvy::dotenv();
    let config = ServerConfig::from_env()?;
    build_app_from_config(&config).await
}

pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let network = sim_core::city_network::CityNetwork::from_path(resolve_city_network_path())?;
    let event_store = PostgresWorldEventStore::connect(&config.database_url).await?;
    let snapshot_store = PostgresChunkSnapshotStore::connect(
        &config.database_url,
        SimulationRuntime::default_world_id(),
    )
    .await?;
    let mobility_snapshot_store =
        PostgresMobilitySnapshotStore::connect(&config.database_url).await?;
    let card_hands = CardHandStore::postgres(&config.database_url).await?;
    let auth = AuthVerifier::supabase(&config.supabase_url).await;

    let (runtime, snapshot_store, mobility_snapshot_store) = SimulationRuntime::hydrate_from_stores(
        Box::new(event_store),
        Box::new(snapshot_store),
        Box::new(mobility_snapshot_store),
        &network,
    )
    .await?;

    let state = AppState::new_with_stores(
        runtime,
        snapshot_store,
        mobility_snapshot_store,
        card_hands,
        auth,
    );
    Ok(build_router_from_state(state))
}

pub fn build_app_with_runtime(runtime: SimulationRuntime) -> Router {
    build_app_with_runtime_and_card_hands(
        runtime,
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    )
}

pub fn build_app_with_runtime_and_card_hands(
    runtime: SimulationRuntime,
    card_hands: CardHandStore,
    auth: AuthVerifier,
) -> Router {
    let state = AppState::new_with_card_hands(runtime, card_hands, auth);
    build_router_from_state(state)
}

fn build_router_from_state(state: AppState) -> Router {
    state.spawn_delta_loop(SIMULATION_TICK_INTERVAL);
    state.spawn_snapshot_loop(SNAPSHOT_INTERVAL);

    Router::new()
        .route("/health", get(health))
        .route("/cards", get(cards))
        .route("/card-hand", get(card_hand).put(save_card_hand))
        .route("/world", get(world))
        .route("/chunks/{x}/{y}", get(chunk))
        .route("/commands", post(command))
        .route("/mobility", get(mobility))
        .route("/ws", get(websocket))
        .with_state(state)
        .layer(CorsLayer::permissive())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let runtime = state.runtime();
    let runtime = runtime.read().await;
    Json(runtime.health())
}

async fn world(State(state): State<AppState>) -> Json<WorldSummaryDto> {
    let runtime = state.runtime();
    let runtime = runtime.read().await;
    Json(runtime.world_summary())
}

async fn mobility(State(state): State<AppState>) -> Json<MobilitySnapshotDto> {
    let runtime = state.runtime();
    let runtime = runtime.read().await;
    Json(runtime.mobility_snapshot())
}

async fn cards() -> Json<Vec<crate::card_hand::CardDefinition>> {
    Json(card_definitions())
}

async fn card_hand(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match state.auth.authenticate(&headers).await {
        Ok(user_id) => user_id,
        Err(error) => return card_hand_error(error),
    };
    match state.card_hands.get_or_create(user_id).await {
        Ok(cards) => Json(CardHandResponse {
            user_id: user_id.to_string(),
            cards,
        })
        .into_response(),
        Err(error) => card_hand_error(error),
    }
}

async fn save_card_hand(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveCardHandRequest>,
) -> Response {
    let user_id = match state.auth.authenticate(&headers).await {
        Ok(user_id) => user_id,
        Err(error) => return card_hand_error(error),
    };
    match state.card_hands.save(user_id, request.cards.clone()).await {
        Ok(()) => Json(CardHandResponse {
            user_id: user_id.to_string(),
            cards: request.cards,
        })
        .into_response(),
        Err(error) => card_hand_error(error),
    }
}

fn card_hand_error(error: CardHandError) -> Response {
    let status = match error {
        CardHandError::MissingAuth | CardHandError::InvalidAuth => StatusCode::UNAUTHORIZED,
        CardHandError::UnknownCard(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CardHandError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": error.to_string() })),
    )
        .into_response()
}

async fn chunk(
    State(state): State<AppState>,
    Path((x, y)): Path<(i32, i32)>,
) -> Result<Json<ChunkSnapshotDto>, StatusCode> {
    let runtime = state.runtime();
    let runtime = runtime.read().await;
    runtime
        .chunk_snapshot(ChunkCoord { x, y })
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn command(State(state): State<AppState>, Json(command): Json<ClientCommandDto>) -> Response {
    let result = {
        let runtime = state.runtime();
        let mut runtime = runtime.write().await;
        runtime.apply_client_command(command).await
    };

    match result {
        Ok(applied) => {
            let _ = state.deltas.send(ServerMessageDto::WorldEvent {
                event: applied.event.clone(),
            });
            (
                StatusCode::OK,
                Json(CommandResponseDto::Accepted(applied.response)),
            )
                .into_response()
        }
        Err(rejection) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(CommandResponseDto::Rejected(rejection.into_dto())),
        )
            .into_response(),
    }
}

async fn websocket(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_world_deltas(socket, state))
}

#[derive(Default)]
struct ConnectionState {
    subscription: std::collections::HashSet<sim_core::ids::ChunkCoord>,
    last_visible_agents: std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: std::collections::HashSet<abutown_protocol::EntityId>,
}

async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let mut deltas = state.subscribe_deltas();
    let hello = {
        let runtime = state.runtime();
        let runtime = runtime.read().await;
        runtime.hello()
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    let mut connection = ConnectionState::default();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                let Some(Ok(message)) = inbound else { break; };
                let Message::Text(text) = message else { continue; };
                let Ok(client_message) = serde_json::from_str::<ClientMessageDto>(&text) else {
                    tracing::warn!(?text, "invalid client message");
                    continue;
                };
                let synthetic = handle_client_message(&state, &client_message, &mut connection).await;
                if let Some(dto) = synthetic
                    && send_server_message(&mut socket, ServerMessageDto::MobilityDelta(dto))
                        .await
                        .is_err()
                {
                    break;
                }
            }
            broadcast = deltas.recv() => {
                let message = match broadcast {
                    Ok(message) => message,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let outbound = match message {
                    ServerMessageDto::MobilityDelta(raw_delta) => {
                        let dto = {
                            let runtime = state.runtime();
                            let runtime = runtime.read().await;
                            runtime.filtered_mobility_delta_from_dto(
                                &raw_delta,
                                &connection.subscription,
                                &mut connection.last_visible_agents,
                                &mut connection.last_visible_vehicles,
                            )
                        };
                        if dto.changed_agents.is_empty()
                            && dto.changed_vehicles.is_empty()
                            && dto.left_agents.is_empty()
                            && dto.left_vehicles.is_empty()
                        {
                            continue;
                        }
                        ServerMessageDto::MobilityDelta(dto)
                    }
                    other => other,
                };
                if send_server_message(&mut socket, outbound).await.is_err() {
                    break;
                }
            }
        }
    }

    // Cleanup: drop this connection's chunk subscriptions so the shared
    // ChunkSubscribers count stays consistent after disconnect.
    if !connection.subscription.is_empty() {
        let runtime = state.runtime();
        let mut runtime = runtime.write().await;
        runtime.apply_subscription_diff(std::iter::empty(), connection.subscription.iter());
    }
}

async fn handle_client_message(
    state: &AppState,
    message: &ClientMessageDto,
    connection: &mut ConnectionState,
) -> Option<MobilityDeltaDto> {
    let (added, removed): (
        Vec<sim_core::ids::ChunkCoord>,
        Vec<sim_core::ids::ChunkCoord>,
    ) = match message {
        ClientMessageDto::ChunkSubscribe(payload) => {
            let added: Vec<_> = payload
                .coords
                .iter()
                .map(sim_core::ids::ChunkCoord::from)
                .filter(|c| connection.subscription.insert(*c))
                .collect();
            (added, Vec::new())
        }
        ClientMessageDto::ChunkUnsubscribe(payload) => {
            let removed: Vec<_> = payload
                .coords
                .iter()
                .map(sim_core::ids::ChunkCoord::from)
                .filter(|c| connection.subscription.remove(c))
                .collect();
            (Vec::new(), removed)
        }
    };
    let runtime = state.runtime();
    let mut runtime = runtime.write().await;
    runtime.apply_subscription_diff(&added, &removed);
    let dto = runtime.synthetic_mobility_delta_for_subscription(
        &connection.subscription,
        &mut connection.last_visible_agents,
        &mut connection.last_visible_vehicles,
    );
    if dto.changed_agents.is_empty()
        && dto.changed_vehicles.is_empty()
        && dto.left_agents.is_empty()
        && dto.left_vehicles.is_empty()
    {
        None
    } else {
        Some(dto)
    }
}

async fn persist_snapshots_once(
    state: &AppState,
) -> Result<usize, sim_core::persistence::ChunkSnapshotStoreError> {
    // Phase 1: collect everything under a brief read-lock — mobility snapshot
    // is cloned here so the lock can be released before any DB write.
    let (snapshots, coords, world_id, mobility_tick, mobility_snapshot) = {
        let runtime_arc = state.runtime();
        let runtime = runtime_arc.read().await;
        let snapshots = runtime.collect_chunk_snapshots();
        let coords: Vec<ChunkCoord> = snapshots
            .iter()
            .map(|s| ChunkCoord {
                x: s.coord.x,
                y: s.coord.y,
            })
            .collect();
        let world_id = runtime.world_id_for_persist().clone();
        let mobility_tick = runtime.mobility_tick();
        let mobility_snapshot = runtime.mobility_world_clone_for_persist();
        (snapshots, coords, world_id, mobility_tick, mobility_snapshot)
        // read-lock released here — no DB write has happened yet
    };

    let written = coords.len();

    // Phase 2a: chunk DB writes — store-mutex only, no runtime lock held.
    {
        let store = state.snapshot_store();
        let mut store = store.lock().await;
        for snapshot in snapshots {
            store.write_snapshot(snapshot).await?;
        }
    }

    // Phase 2b: mobility DB write — store-mutex only, no runtime lock held.
    {
        let mob_store = state.mobility_snapshot_store();
        let mut mob_store = mob_store.lock().await;
        if let Err(error) = mob_store
            .write(&world_id.0, mobility_tick, &mobility_snapshot)
            .await
        {
            tracing::warn!(%error, "failed to persist mobility snapshot");
        }
    }

    // Phase 3: brief write-lock to mark chunk snapshots as persisted.
    {
        let runtime_arc = state.runtime();
        let mut runtime = runtime_arc.write().await;
        runtime.mark_chunk_snapshots_persisted(&coords);
    }

    Ok(written)
}

async fn send_server_message(
    socket: &mut WebSocket,
    message: ServerMessageDto,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let text = serde_json::to_string(&message)?;

    socket.send(Message::Text(text.into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::ids::ChunkCoord;
    use sim_core::persistence::{ChunkSnapshotStore, ChunkSnapshotStoreError};
    use abutown_protocol::ChunkSnapshotDto;

    #[tokio::test]
    async fn persist_snapshots_once_writes_runtime_snapshots() {
        let state = AppState::new(SimulationRuntime::new());

        assert_eq!(persist_snapshots_once(&state).await.unwrap(), 3);

        let snapshot = state
            .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .await
            .unwrap()
            .expect("visible snapshot stored");
        assert_eq!(snapshot.coord.x, 4);
        assert_eq!(snapshot.coord.y, 4);
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
        ) -> Result<(), ChunkSnapshotStoreError> {
            tokio::time::sleep(std::time::Duration::from_millis(self.write_delay_ms)).await;
            Ok(())
        }

        async fn read_snapshot(
            &self,
            _coord: ChunkCoord,
        ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn concurrent_reads_proceed_during_snapshot_persist() {
        use std::time::{Duration, Instant};
        use sim_core::persistence::InMemoryMobilitySnapshotStore;

        // Build AppState with a slow snapshot store (100 ms per write, 3 chunks = 300 ms total).
        let state = AppState::new_with_stores(
            SimulationRuntime::new(),
            Box::new(SlowSnapshotStore { write_delay_ms: 100 }),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            CardHandStore::memory(),
            AuthVerifier::local_bearer_uuid(),
        );

        // Spawn persist; it should release the runtime lock during the slow DB write.
        let state_for_persist = state.clone();
        let persist = tokio::spawn(async move { persist_snapshots_once(&state_for_persist).await });

        // Briefly wait so persist enters its DB-write phase (still inside the 100 ms sleep).
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Now do concurrent reads. They MUST complete quickly because the
        // runtime read-lock should NOT be held during the DB write.
        let read_start = Instant::now();
        let mut handles = Vec::new();
        for _ in 0..10 {
            let s = state.clone();
            handles.push(tokio::spawn(async move {
                let runtime_arc = s.runtime();
                let runtime = runtime_arc.read().await;
                let _health = runtime.health();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let read_elapsed = read_start.elapsed();

        // Before the fix: reads serialized behind persist's write lock → ~280 ms wait.
        // After the fix: reads proceed in parallel → a few ms total.
        // Use a generous threshold (50 ms) so the test is not flaky on slow CI.
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
        ) -> Result<(), ChunkSnapshotStoreError> {
            Err(ChunkSnapshotStoreError::unavailable("test failure"))
        }

        async fn read_snapshot(
            &self,
            _coord: ChunkCoord,
        ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn snapshot_write_failure_preserves_dirty_state() {
        use sim_core::persistence::InMemoryMobilitySnapshotStore;

        let state = AppState::new_with_stores(
            SimulationRuntime::new(),
            Box::new(FailingSnapshotStore),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            CardHandStore::memory(),
            AuthVerifier::local_bearer_uuid(),
        );

        // First persist attempt must fail because the store always errors.
        let result = persist_snapshots_once(&state).await;
        assert!(result.is_err(), "persist should propagate the store error");

        // The chunks must still be dirty — mark_chunk_snapshots_persisted must
        // NOT have been called after a failed write.
        let runtime_arc = state.runtime();
        let runtime = runtime_arc.read().await;
        let still_dirty = runtime.collect_chunk_snapshots();
        assert!(
            !still_dirty.is_empty(),
            "snapshot write failure must not mark chunks persisted (snapshots remain dirty)"
        );
    }
}
