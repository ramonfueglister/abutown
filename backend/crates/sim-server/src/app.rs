use std::{sync::Arc, time::Duration};

use abutown_protocol::{
    ChunkSnapshotDto, ClientCommandDto, ClientMessageDto, CommandResponseDto, HealthResponse,
    MobilityChunkDeltaDto, MobilitySnapshotDto, ServerMessageDto, WorldSummaryDto,
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
use tokio_stream::StreamMap;
use tokio_stream::wrappers::BroadcastStream;
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
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>>,
    view: Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
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
        let initial_view = build_read_view_from_runtime(
            &runtime,
            &std::collections::HashMap::new(),
            0,
        );
        Self {
            runtime: Arc::new(RwLock::new(runtime)),
            deltas,
            card_hands,
            auth,
            snapshot_store: Arc::new(Mutex::new(snapshot_store)),
            mobility_snapshot_store: Arc::new(Mutex::new(mobility_snapshot_store)),
            chunk_channels: Arc::new(DashMap::new()),
            view: Arc::new(arc_swap::ArcSwap::from_pointee(initial_view)),
        }
    }

    pub(crate) fn runtime(&self) -> Arc<RwLock<SimulationRuntime>> {
        Arc::clone(&self.runtime)
    }

    pub(crate) fn view(&self) -> Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>> {
        Arc::clone(&self.view)
    }

    fn snapshot_store(&self) -> Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>> {
        Arc::clone(&self.snapshot_store)
    }

    fn mobility_snapshot_store(&self) -> Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>> {
        Arc::clone(&self.mobility_snapshot_store)
    }

    pub(crate) fn chunk_channels(
        &self,
    ) -> Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>> {
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
        let state = self.clone();
        let deltas = self.deltas.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_interval);
            interval.tick().await;
            loop {
                interval.tick().await;
                // Per-chunk fan-out: tick mobility and broadcast deltas to subscribed chunk channels.
                tick_and_fan_out(&state).await;
                // Broadcast tile pulse to all connected clients via the global broadcast channel.
                let pulse = {
                    let runtime_arc = state.runtime();
                    let mut runtime = runtime_arc.write().await;
                    runtime.next_pulse()
                };
                let _ = deltas.send(pulse);
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

fn build_read_view_from_runtime(
    runtime: &SimulationRuntime,
    per_chunk: &std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        sim_core::mobility::MobilityChunkDelta,
    >,
    pulse_sequence: u64,
) -> crate::runtime_view::RuntimeReadView {
    let world_id = runtime.world_id_for_persist().clone();
    let mobility_tick = runtime.mobility_tick();
    let mobility = runtime.mobility();

    let per_chunk_deltas: Vec<abutown_protocol::MobilityChunkDeltaDto> = per_chunk
        .values()
        .map(|delta| chunk_delta_to_dto(delta, mobility, &world_id, mobility_tick))
        .collect();

    // Pre-materialize per-chunk tile snapshots for every loaded chunk so HTTP
    // /chunks/{x}/{y} can read without a lock.
    let mut chunk_snapshots: std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        abutown_protocol::ChunkSnapshotDto,
    > = std::collections::HashMap::new();
    for coord_dto in runtime.world_summary().loaded_chunks.iter() {
        let coord = sim_core::ids::ChunkCoord {
            x: coord_dto.x,
            y: coord_dto.y,
        };
        if let Some(snap) = runtime.chunk_snapshot(coord) {
            chunk_snapshots.insert(coord, snap);
        }
    }

    crate::runtime_view::RuntimeReadView {
        tick: mobility_tick,
        world_id: world_id.clone(),
        mobility_tick,
        health: runtime.health(),
        world_summary: runtime.world_summary(),
        chunk_snapshots,
        mobility_full_dto: runtime.mobility_snapshot(),
        per_chunk_deltas,
        pulse_sequence,
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

    let (runtime, snapshot_store, mobility_snapshot_store) =
        SimulationRuntime::hydrate_from_stores(
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
    Json(state.view().load().health.clone())
}

async fn world(State(state): State<AppState>) -> Json<WorldSummaryDto> {
    Json(state.view().load().world_summary.clone())
}

async fn mobility(State(state): State<AppState>) -> Json<MobilitySnapshotDto> {
    Json(state.view().load().mobility_full_dto.clone())
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
    let coord = sim_core::ids::ChunkCoord { x, y };
    state
        .view()
        .load()
        .chunk_snapshots
        .get(&coord)
        .cloned()
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

struct ConnectionState {
    subscription: std::collections::HashSet<sim_core::ids::ChunkCoord>,
    chunk_streams: StreamMap<sim_core::ids::ChunkCoord, BroadcastStream<MobilityChunkDeltaDto>>,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            subscription: std::collections::HashSet::new(),
            chunk_streams: StreamMap::new(),
        }
    }
}

async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let mut deltas = state.subscribe_deltas();
    let hello = {
        let view = state.view().load();
        ServerMessageDto::Hello(abutown_protocol::ServerHelloDto {
            protocol_version: abutown_protocol::PROTOCOL_VERSION,
            world_id: view.world_id.clone(),
            chunk_size: view.world_summary.chunk_size,
        })
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    let mut connection = ConnectionState::new();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                let Some(Ok(message)) = inbound else { break; };
                let Message::Text(text) = message else { continue; };
                let Ok(client_message) = serde_json::from_str::<ClientMessageDto>(&text) else {
                    tracing::warn!(?text, "invalid client message");
                    continue;
                };
                let outgoing = handle_client_message(&state, &client_message, &mut connection).await;
                let mut errored = false;
                for msg in outgoing {
                    if send_server_message(&mut socket, msg).await.is_err() {
                        errored = true;
                        break;
                    }
                }
                if errored {
                    break;
                }
            }
            Some((chunk, item)) = tokio_stream::StreamExt::next(&mut connection.chunk_streams), if !connection.chunk_streams.is_empty() => {
                use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
                match item {
                    Ok(delta) => {
                        if send_server_message(
                            &mut socket,
                            ServerMessageDto::MobilityChunkDelta(delta),
                        ).await.is_err() {
                            break;
                        }
                    }
                    Err(BroadcastStreamRecvError::Lagged(_)) => {
                        // Recovery: re-send a fresh snapshot for this chunk.
                        let snap = {
                            let runtime_arc = state.runtime();
                            let runtime = runtime_arc.read().await;
                            let snapshot = runtime.mobility().build_chunk_snapshot(chunk);
                            let world_id = runtime.world_id_for_persist().clone();
                            let tick = runtime.mobility_tick();
                            chunk_snapshot_to_dto(&snapshot, runtime.mobility(), &world_id, tick)
                        };
                        if send_server_message(
                            &mut socket,
                            ServerMessageDto::MobilityChunkSnapshot(snap),
                        ).await.is_err() {
                            break;
                        }
                    }
                }
            }
            broadcast = deltas.recv() => {
                let message = match broadcast {
                    Ok(message) => message,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if send_server_message(&mut socket, message).await.is_err() {
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

fn chunk_delta_to_dto(
    delta: &sim_core::mobility::MobilityChunkDelta,
    world: &sim_core::mobility::MobilityWorld,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> abutown_protocol::MobilityChunkDeltaDto {
    abutown_protocol::MobilityChunkDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        chunk: abutown_protocol::ChunkCoordDto {
            x: delta.chunk.x,
            y: delta.chunk.y,
        },
        changed_agents: delta
            .changed_agents
            .iter()
            .filter_map(|r| world.agent_dto_for(&r.id))
            .collect(),
        changed_vehicles: delta
            .changed_vehicles
            .iter()
            .filter_map(|r| world.vehicle_dto_for(&r.id))
            .collect(),
        left_agents: delta
            .left_agents
            .iter()
            .map(|id| abutown_protocol::EntityId(id.0.clone()))
            .collect(),
        left_vehicles: delta
            .left_vehicles
            .iter()
            .map(|id| abutown_protocol::EntityId(id.0.clone()))
            .collect(),
    }
}

async fn tick_and_fan_out(state: &AppState) {
    // Phase 1: tick the world under brief write-lock; collect deltas + metadata.
    let (per_chunk, world_id, tick) = {
        let runtime_arc = state.runtime();
        let mut runtime = runtime_arc.write().await;
        let per_chunk = runtime.tick_world_mobility();
        let world_id = runtime.world_id_for_persist().clone();
        let tick = runtime.mobility_tick();
        (per_chunk, world_id, tick)
    }; // write-lock dropped here

    // Phase 2 + 3 share one read-lock — Phase 2 broadcasts per-chunk delta DTOs
    // (needs &MobilityWorld for record→DTO conversion), Phase 3 builds the
    // RuntimeReadView for lock-free readers (also needs &MobilityWorld).
    let chunk_channels = state.chunk_channels();
    let runtime_arc = state.runtime();
    let runtime = runtime_arc.read().await;

    if !per_chunk.is_empty() {
        let mobility = runtime.mobility();
        for (chunk, delta) in &per_chunk {
            let Some(sender) = chunk_channels.get(chunk).map(|e| e.clone()) else {
                continue;
            };
            let dto = chunk_delta_to_dto(delta, mobility, &world_id, tick);
            let _ = sender.send(dto); // best-effort; ignore receiver count
        }
    }

    // Phase 3 — publish the new RuntimeReadView for lock-free HTTP/WS readers.
    let pulse_sequence = state.view.load().pulse_sequence;
    let view = build_read_view_from_runtime(&runtime, &per_chunk, pulse_sequence);
    state.view.store(Arc::new(view));
}

fn chunk_snapshot_to_dto(
    snapshot: &sim_core::mobility::MobilityChunkSnapshot,
    world: &sim_core::mobility::MobilityWorld,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> abutown_protocol::MobilityChunkSnapshotDto {
    abutown_protocol::MobilityChunkSnapshotDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        chunk: abutown_protocol::ChunkCoordDto {
            x: snapshot.chunk.x,
            y: snapshot.chunk.y,
        },
        agents: snapshot
            .agents
            .iter()
            .filter_map(|record| world.agent_dto_for(&record.id))
            .collect(),
        vehicles: snapshot
            .vehicles
            .iter()
            .filter_map(|record| world.vehicle_dto_for(&record.id))
            .collect(),
    }
}

async fn handle_client_message(
    state: &AppState,
    message: &ClientMessageDto,
    connection: &mut ConnectionState,
) -> Vec<ServerMessageDto> {
    let mut out: Vec<ServerMessageDto> = Vec::new();

    match message {
        ClientMessageDto::ChunkSubscribe(payload) => {
            let added: Vec<sim_core::ids::ChunkCoord> = payload
                .coords
                .iter()
                .map(sim_core::ids::ChunkCoord::from)
                .filter(|c| connection.subscription.insert(*c))
                .collect();

            if !added.is_empty() {
                let chunk_channels = state.chunk_channels();
                let runtime_arc = state.runtime();
                let mut runtime = runtime_arc.write().await;
                runtime.apply_subscription_diff(&added, std::iter::empty());

                let world_id = runtime.world_id_for_persist().clone();
                let tick = runtime.mobility_tick();

                for coord in &added {
                    // Get-or-create broadcast channel for this chunk.
                    let sender = chunk_channels
                        .entry(*coord)
                        .or_insert_with(|| broadcast::channel(8).0)
                        .clone();
                    // Subscribe and store receiver in chunk_streams.
                    let receiver = sender.subscribe();
                    connection
                        .chunk_streams
                        .insert(*coord, BroadcastStream::new(receiver));
                    // Build current snapshot of this chunk and push as MobilityChunkSnapshot.
                    let snapshot = runtime.mobility().build_chunk_snapshot(*coord);
                    let dto = chunk_snapshot_to_dto(&snapshot, runtime.mobility(), &world_id, tick);
                    out.push(ServerMessageDto::MobilityChunkSnapshot(dto));
                }
            }
        }

        ClientMessageDto::ChunkUnsubscribe(payload) => {
            let removed: Vec<sim_core::ids::ChunkCoord> = payload
                .coords
                .iter()
                .map(sim_core::ids::ChunkCoord::from)
                .filter(|c| connection.subscription.remove(c))
                .collect();

            if !removed.is_empty() {
                let chunk_channels = state.chunk_channels();
                let runtime_arc = state.runtime();
                let mut runtime = runtime_arc.write().await;
                runtime.apply_subscription_diff(std::iter::empty(), &removed);

                for coord in &removed {
                    // Drop this connection's receiver for the chunk.
                    connection.chunk_streams.remove(coord);
                    // Reap the channel if no other client is subscribed to this chunk.
                    if runtime.chunk_subscriber_count(*coord) == 0 {
                        chunk_channels.remove(coord);
                    }
                }
            }
        }
    }

    out
}

async fn persist_snapshots_once(
    state: &AppState,
) -> Result<usize, sim_core::persistence::ChunkSnapshotStoreError> {
    // TODO Phase 7c task 6: migrate to view.load() once MobilitySnapshotStore
    // accepts MobilitySnapshotDto directly (via a new write_dto method).
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
        (
            snapshots,
            coords,
            world_id,
            mobility_tick,
            mobility_snapshot,
        )
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
    use abutown_protocol::ChunkSnapshotDto;
    use sim_core::ids::ChunkCoord;
    use sim_core::persistence::{ChunkSnapshotStore, ChunkSnapshotStoreError};

    #[tokio::test]
    async fn http_reads_are_lock_free_while_commands_are_in_flight() {
        use std::time::{Duration, Instant};

        let state = AppState::new(SimulationRuntime::new());

        // Spawn 50 background tasks that take the runtime write-lock.
        let mut command_tasks = Vec::new();
        for _ in 0..50 {
            let state = state.clone();
            command_tasks.push(tokio::spawn(async move {
                let runtime = state.runtime();
                let _w = runtime.write().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }));
        }

        // 50 concurrent reads of /world via the view path — must not wait.
        let start = Instant::now();
        let mut read_tasks = Vec::new();
        for _ in 0..50 {
            let state = state.clone();
            read_tasks.push(tokio::spawn(async move {
                let _ = state.view().load().world_summary.clone();
            }));
        }
        for t in read_tasks {
            t.await.unwrap();
        }
        let read_elapsed = start.elapsed();

        assert!(
            read_elapsed < Duration::from_millis(50),
            "lock-free reads took {read_elapsed:?}, expected < 50ms"
        );

        for t in command_tasks {
            t.await.unwrap();
        }
    }

    #[tokio::test]
    async fn runtime_read_view_updates_after_tick() {
        let state = AppState::new(SimulationRuntime::new());
        let view0 = state.view().load();
        let tick0 = view0.mobility_tick;
        drop(view0);

        tick_and_fan_out(&state).await;

        let view1 = state.view().load();
        assert_eq!(view1.mobility_tick, tick0 + 1, "tick should have advanced");
        assert!(
            !view1.chunk_snapshots.is_empty(),
            "view should include chunk snapshots"
        );
    }

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
        use sim_core::persistence::InMemoryMobilitySnapshotStore;
        use std::time::{Duration, Instant};

        // Build AppState with a slow snapshot store (100 ms per write, 3 chunks = 300 ms total).
        let state = AppState::new_with_stores(
            SimulationRuntime::new(),
            Box::new(SlowSnapshotStore {
                write_delay_ms: 100,
            }),
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
