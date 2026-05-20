use std::{sync::Arc, time::Duration};

use abutown_protocol::v1 as w;
use abutown_protocol::ChunkSnapshotDto;
use axum::{
    Json, Router,
    extract::{
        FromRequest, Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{self, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use dashmap::DashMap;
use prost::Message as _;
use sim_core::{
    ids::ChunkCoord,
    persistence::{
        ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryChunkSnapshotStore,
        InMemoryMobilitySnapshotStore, MobilitySnapshotStore,
    },
};
use tokio::sync::{Mutex, broadcast};
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
    deltas: broadcast::Sender<w::ServerMessage>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
    snapshot_store: Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>>,
    view: Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
    mutations: tokio::sync::mpsc::UnboundedSender<crate::runtime_view::Mutation>,
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
        let (mutation_tx, mutation_rx) = tokio::sync::mpsc::unbounded_channel();
        let view = Arc::new(arc_swap::ArcSwap::from_pointee(initial_view));
        let chunk_channels: Arc<DashMap<_, _>> = Arc::new(DashMap::new());

        let state = Self {
            deltas: deltas.clone(),
            card_hands,
            auth,
            snapshot_store: Arc::new(Mutex::new(snapshot_store)),
            mobility_snapshot_store: Arc::new(Mutex::new(mobility_snapshot_store)),
            chunk_channels: Arc::clone(&chunk_channels),
            view: Arc::clone(&view),
            mutations: mutation_tx,
        };

        // Panic supervisor: if tick_loop panics, every reader is stuck on
        // the last-published view forever and every Mutation::send fails.
        // Log loudly and abort so an external supervisor (systemd, k8s,
        // container restart policy) can recover us instead of running a
        // zombie server that serves stale data with no recovery path.
        let tick_fut = tick_loop(
            runtime,
            mutation_rx,
            view,
            deltas,
            chunk_channels,
            SIMULATION_TICK_INTERVAL,
        );
        let supervised = tokio::spawn(tick_fut);
        tokio::spawn(async move {
            match supervised.await {
                Ok(()) => {
                    tracing::error!("tick_loop exited normally — should run forever");
                    std::process::abort();
                }
                Err(join_err) => {
                    if join_err.is_panic() {
                        let panic = join_err.into_panic();
                        let msg = panic
                            .downcast_ref::<&'static str>()
                            .copied()
                            .or_else(|| panic.downcast_ref::<String>().map(|s| s.as_str()))
                            .unwrap_or("<non-string panic>");
                        tracing::error!(panic = %msg, "tick_loop panicked");
                    } else {
                        tracing::error!(?join_err, "tick_loop task cancelled");
                    }
                    std::process::abort();
                }
            }
        });

        state
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
    ) -> Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>> {
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

    fn subscribe_deltas(&self) -> broadcast::Receiver<w::ServerMessage> {
        self.deltas.subscribe()
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

    let per_chunk_deltas: Vec<w::MobilityChunkDelta> = per_chunk
        .values()
        .map(|delta| chunk_delta_to_dto(delta, mobility, &world_id, mobility_tick))
        .collect();

    // Pre-materialize per-chunk tile + mobility snapshots for every loaded
    // chunk so HTTP /chunks/{x}/{y} and WS lagged-recovery can read without
    // a lock. Single pass over loaded_chunks builds both maps.
    //
    // NOTE: this rebuilds snapshots for every loaded chunk every tick, even
    // for chunks that didn't change. Bench stayed within the 5% tolerance
    // (see plan §Task 4 step 5 — the optimization to only rebuild changed
    // chunks is a documented escape hatch if bench regresses).
    let world_summary_legacy = runtime.world_summary();
    let world_summary = world_summary_dto_to_proto(&world_summary_legacy);
    let mut chunk_snapshots: std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        w::ChunkSnapshot,
    > = std::collections::HashMap::new();
    let mut mobility_chunk_snapshots: std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        w::MobilityChunkSnapshot,
    > = std::collections::HashMap::new();
    for coord_dto in world_summary_legacy.loaded_chunks.iter() {
        let coord = sim_core::ids::ChunkCoord {
            x: coord_dto.x,
            y: coord_dto.y,
        };
        if let Some(snap) = runtime.chunk_snapshot(coord) {
            chunk_snapshots.insert(coord, chunk_snapshot_dto_to_proto(&snap));
        }
        let mob_snapshot = mobility.build_chunk_snapshot(coord);
        let mob_dto = chunk_snapshot_to_dto(&mob_snapshot, mobility, &world_id, mobility_tick);
        mobility_chunk_snapshots.insert(coord, mob_dto);
    }

    let chunk_subscriber_counts = mobility.chunk_subscriber_counts_snapshot();
    let mobility_full_legacy = runtime.mobility_snapshot();
    let mobility_full_dto = mobility_snapshot_dto_to_proto(&mobility_full_legacy);
    let health_legacy = runtime.health();
    let health = health_dto_to_proto(&health_legacy);

    crate::runtime_view::RuntimeReadView {
        tick: mobility_tick,
        world_id: world_id.clone(),
        mobility_tick,
        health,
        world_summary,
        chunk_snapshots,
        mobility_chunk_snapshots,
        mobility_full_dto,
        per_chunk_deltas,
        pulse_sequence,
        chunk_subscriber_counts,
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
    // tick_loop is already running (spawned in new_with_stores). Only the
    // periodic persist loop needs to be spawned here, since it depends on the
    // AppState clone (view + mutations).
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

async fn health(State(state): State<AppState>) -> Response {
    proto_response(state.view().load().health.clone())
}

async fn world(State(state): State<AppState>) -> Response {
    proto_response(state.view().load().world_summary.clone())
}

async fn mobility(State(state): State<AppState>) -> Response {
    proto_response(state.view().load().mobility_full_dto.clone())
}

/// Encode any prost message as an `application/x-protobuf` HTTP response.
fn proto_response<M: prost::Message>(message: M) -> Response {
    let bytes = message.encode_to_vec();
    (
        [(http::header::CONTENT_TYPE, "application/x-protobuf")],
        bytes,
    )
        .into_response()
}

/// Extractor that decodes the request body as a prost message. Mirrors the
/// shape of `axum::Json` but for `application/x-protobuf` bodies. Used by
/// `POST /commands` after Task 6.
pub struct ProtoBody<M>(pub M);

impl<S, M> FromRequest<S> for ProtoBody<M>
where
    S: Send + Sync,
    M: prost::Message + Default,
{
    type Rejection = (StatusCode, String);

    async fn from_request(
        req: http::Request<axum::body::Body>,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let msg = M::decode(bytes.as_ref())
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        Ok(ProtoBody(msg))
    }
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
) -> Response {
    let coord = sim_core::ids::ChunkCoord { x, y };
    match state.view().load().chunk_snapshots.get(&coord).cloned() {
        Some(snap) => proto_response(snap),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn command(
    State(state): State<AppState>,
    ProtoBody(command): ProtoBody<w::ClientCommand>,
) -> Response {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .mutations
        .send(crate::runtime_view::Mutation::ApplyCommand {
            command,
            reply: reply_tx,
        })
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    let result = match reply_rx.await {
        Ok(r) => r,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    match result {
        Ok(applied) => {
            let event_proto = world_event_dto_to_proto(&applied.event);
            let msg = w::ServerMessage {
                body: Some(w::server_message::Body::WorldEvent(event_proto.clone())),
            };
            let _ = state.deltas.send(msg);
            (
                StatusCode::OK,
                proto_response(w::CommandResponse {
                    outcome: Some(w::command_response::Outcome::Accepted(w::CommandAccepted {
                        protocol_version: u32::from(applied.response.protocol_version),
                        world_id: applied.response.world_id.0.clone(),
                        command_id: applied.response.command_id.clone(),
                        event: Some(event_proto),
                    })),
                }),
            )
                .into_response()
        }
        Err(rejection) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            proto_response(w::CommandResponse {
                outcome: Some(w::command_response::Outcome::Rejected(w::CommandRejected {
                    protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
                    world_id: rejection
                        .world_id
                        .as_ref()
                        .map(|w| w.0.clone())
                        .unwrap_or_default(),
                    command_id: rejection.command_id.clone().unwrap_or_default(),
                    code: rejection.code.to_string(),
                    message: rejection.message.clone(),
                })),
            }),
        )
            .into_response(),
    }
}

async fn websocket(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_world_deltas(socket, state))
}

struct ConnectionState {
    subscription: std::collections::HashSet<sim_core::ids::ChunkCoord>,
    chunk_streams: StreamMap<sim_core::ids::ChunkCoord, BroadcastStream<w::MobilityChunkDelta>>,
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
        w::ServerMessage {
            body: Some(w::server_message::Body::Hello(w::Hello {
                protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
                world_id: view.world_id.0.clone(),
                chunk_size: view.world_summary.chunk_size,
            })),
        }
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    let mut connection = ConnectionState::new();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                let Some(Ok(message)) = inbound else { break; };
                let Message::Binary(bytes) = message else { continue; };
                let client_message = match w::ClientMessage::decode(bytes.as_ref()) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(?e, "invalid client message");
                        continue;
                    }
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
                        let msg = w::ServerMessage {
                            body: Some(w::server_message::Body::MobilityChunkDelta(delta)),
                        };
                        if send_server_message(&mut socket, msg).await.is_err() {
                            break;
                        }
                    }
                    Err(BroadcastStreamRecvError::Lagged(_)) => {
                        // Recovery: re-send a fresh snapshot for this chunk
                        // from the lock-free RuntimeReadView. If the chunk
                        // isn't in the view (e.g., it just unloaded), skip.
                        let snap = state.view().load().mobility_chunk_snapshots.get(&chunk).cloned();
                        if let Some(snap) = snap {
                            let msg = w::ServerMessage {
                                body: Some(w::server_message::Body::MobilityChunkSnapshot(snap)),
                            };
                            if send_server_message(&mut socket, msg).await.is_err() {
                                break;
                            }
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
        let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
        let _ = state
            .mutations
            .send(crate::runtime_view::Mutation::SubscriptionDiff {
                added: Vec::new(),
                removed: connection.subscription.iter().copied().collect(),
                reply: reply_tx,
            });
    }
}

// ===== legacy serde DTO → proto wire helpers =====
//
// These convert the runtime's existing serde-DTO API surface into the
// protobuf wire types now used on the WS hot path. They are internal
// to sim-server; the legacy DTOs in `abutown_protocol::*` survive
// until Task 7 because HTTP / DB persistence still consume them.

fn direction_to_proto(d: abutown_protocol::DirectionDto) -> w::Direction {
    use abutown_protocol::DirectionDto as L;
    match d {
        L::N => w::Direction::N,
        L::Ne => w::Direction::Ne,
        L::E => w::Direction::E,
        L::Se => w::Direction::Se,
        L::S => w::Direction::S,
        L::Sw => w::Direction::Sw,
        L::W => w::Direction::W,
        L::Nw => w::Direction::Nw,
    }
}

fn tile_kind_to_proto(k: abutown_protocol::TileKindDto) -> w::TileKind {
    use abutown_protocol::TileKindDto as L;
    match k {
        L::Grass => w::TileKind::Grass,
        L::Water => w::TileKind::Water,
        L::Road => w::TileKind::Road,
        L::BuildingFootprint => w::TileKind::BuildingFootprint,
    }
}

fn chunk_state_to_proto(s: abutown_protocol::ChunkStateDto) -> w::ChunkState {
    use abutown_protocol::ChunkStateDto as L;
    match s {
        L::Asleep => w::ChunkState::Asleep,
        L::Warm => w::ChunkState::Warm,
        L::Active => w::ChunkState::Active,
        L::Hot => w::ChunkState::Hot,
    }
}

fn agent_dto_to_proto(dto: abutown_protocol::AgentMobilityDto) -> w::AgentMobility {
    use abutown_protocol::AgentMobilityStateDto as Legacy;
    let state = match dto.state {
        Legacy::Walking { link_id, progress } => w::agent_state::State::Walking(w::Walking {
            link_id,
            progress,
        }),
        Legacy::WaitingAtStop { stop_id } => {
            w::agent_state::State::WaitingAtStop(w::WaitingAtStop { stop_id })
        }
        Legacy::InVehicle {
            vehicle_id,
            seat_index,
        } => w::agent_state::State::InVehicle(w::InVehicle {
            vehicle_id: vehicle_id.0,
            seat_index: seat_index as u32,
        }),
        Legacy::Boarding {
            vehicle_id,
            stop_id,
        } => w::agent_state::State::Boarding(w::Boarding {
            vehicle_id: vehicle_id.0,
            stop_id,
        }),
        Legacy::Alighting {
            vehicle_id,
            stop_id,
        } => w::agent_state::State::Alighting(w::Alighting {
            vehicle_id: vehicle_id.0,
            stop_id,
        }),
        Legacy::AtActivity { activity_id } => {
            w::agent_state::State::AtActivity(w::AtActivity { activity_id })
        }
    };
    w::AgentMobility {
        id: dto.id.0,
        state: Some(w::AgentState { state: Some(state) }),
        world_coord: Some(w::WorldCoord {
            x: dto.world_coord.x,
            y: dto.world_coord.y,
        }),
        direction: direction_to_proto(dto.direction) as i32,
        sprite_key: dto.sprite_key,
        plan_cursor: dto.plan_cursor as u32,
    }
}

fn vehicle_dto_to_proto(dto: abutown_protocol::VehicleMobilityDto) -> w::VehicleMobility {
    use abutown_protocol::VehicleKindDto;
    let kind = match dto.kind {
        VehicleKindDto::Car => w::VehicleKind::Car,
        VehicleKindDto::Tram => w::VehicleKind::Tram,
    };
    w::VehicleMobility {
        id: dto.id.0,
        kind: kind as i32,
        route_id: dto.route_id,
        link_index: dto.link_index as u32,
        progress: dto.progress,
        capacity: dto.capacity as u32,
        occupants: dto.occupants.into_iter().map(|e| e.0).collect(),
        dwell_ticks_remaining: dto.dwell_ticks_remaining as u32,
        world_coord: Some(w::WorldCoord {
            x: dto.world_coord.x,
            y: dto.world_coord.y,
        }),
        direction: direction_to_proto(dto.direction) as i32,
        sprite_key: dto.sprite_key,
    }
}

fn stop_dto_to_proto(s: &abutown_protocol::StopMobilityDto) -> w::Stop {
    w::Stop {
        id: s.id.clone(),
        route_id: s.route_id.clone(),
        link_index: s.link_index as u32,
        progress: s.progress,
        waiting_agents: s.waiting_agents.iter().map(|e| e.0.clone()).collect(),
    }
}

fn world_summary_dto_to_proto(s: &abutown_protocol::WorldSummaryDto) -> w::WorldSummary {
    w::WorldSummary {
        protocol_version: u32::from(s.protocol_version),
        world_id: s.world_id.0.clone(),
        chunk_size: u32::from(s.chunk_size),
        loaded_chunks: s
            .loaded_chunks
            .iter()
            .map(|c| w::ChunkCoord { x: c.x, y: c.y })
            .collect(),
        tick_period_ms: s.tick_period_ms,
    }
}

fn health_dto_to_proto(h: &abutown_protocol::HealthResponse) -> w::HealthResponse {
    w::HealthResponse {
        protocol_version: u32::from(h.protocol_version),
        service: h.service.clone(),
        world_id: h.world_id.0.clone(),
        ok: h.ok,
    }
}

fn chunk_snapshot_dto_to_proto(c: &abutown_protocol::ChunkSnapshotDto) -> w::ChunkSnapshot {
    w::ChunkSnapshot {
        protocol_version: u32::from(c.protocol_version),
        world_id: c.world_id.0.clone(),
        coord: Some(w::ChunkCoord {
            x: c.coord.x,
            y: c.coord.y,
        }),
        chunk_version: c.chunk_version,
        chunk_state: chunk_state_to_proto(c.chunk_state) as i32,
        tile_count: u32::from(c.tile_count),
        tiles: c
            .tiles
            .iter()
            .map(|t| w::TileMutation {
                local_index: u32::from(t.local_index),
                kind: tile_kind_to_proto(t.kind) as i32,
                version: t.version,
            })
            .collect(),
    }
}

fn mobility_snapshot_dto_to_proto(s: &abutown_protocol::MobilitySnapshotDto) -> w::MobilitySnapshot {
    w::MobilitySnapshot {
        protocol_version: u32::from(s.protocol_version),
        world_id: s.world_id.0.clone(),
        tick: s.tick,
        agents: s.agents.iter().cloned().map(agent_dto_to_proto).collect(),
        vehicles: s
            .vehicles
            .iter()
            .cloned()
            .map(vehicle_dto_to_proto)
            .collect(),
        stops: s.stops.iter().map(stop_dto_to_proto).collect(),
    }
}

fn tile_pulse_dto_to_proto(p: &abutown_protocol::TilePulseDeltaDto) -> w::TilePulse {
    w::TilePulse {
        protocol_version: u32::from(p.protocol_version),
        world_id: p.world_id.0.clone(),
        tick: p.tick,
        version: p.version,
        coord: Some(w::ChunkCoord {
            x: p.coord.x,
            y: p.coord.y,
        }),
        local_index: u32::from(p.local_index),
    }
}

fn world_event_dto_to_proto(e: &abutown_protocol::WorldEventDto) -> w::WorldEvent {
    use abutown_protocol::WorldEventDto as L;
    match e {
        L::TileKindSet(tk) => w::WorldEvent {
            event: Some(w::world_event::Event::TileKindSet(w::TileKindSetEvent {
                protocol_version: u32::from(tk.protocol_version),
                event_id: tk.event_id.clone(),
                command_id: tk.command_id.clone(),
                world_id: tk.world_id.0.clone(),
                tick: tk.tick,
                version: tk.version,
                coord: Some(w::ChunkCoord {
                    x: tk.coord.x,
                    y: tk.coord.y,
                }),
                local_index: u32::from(tk.local_index),
                kind: tile_kind_to_proto(tk.kind) as i32,
            })),
        },
    }
}

// ===== per-chunk delta + snapshot builders (proto outputs) =====

fn chunk_delta_to_dto(
    delta: &sim_core::mobility::MobilityChunkDelta,
    world: &sim_core::mobility::MobilityWorld,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> w::MobilityChunkDelta {
    w::MobilityChunkDelta {
        protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
        world_id: world_id.0.clone(),
        tick,
        chunk: Some(w::ChunkCoord {
            x: delta.chunk.x,
            y: delta.chunk.y,
        }),
        changed_agents: delta
            .changed_agents
            .iter()
            .filter_map(|r| world.agent_dto_for(&r.id))
            .map(agent_dto_to_proto)
            .collect(),
        changed_vehicles: delta
            .changed_vehicles
            .iter()
            .filter_map(|r| world.vehicle_dto_for(&r.id))
            .map(vehicle_dto_to_proto)
            .collect(),
        left_agents: delta.left_agents.iter().map(|id| id.0.clone()).collect(),
        left_vehicles: delta.left_vehicles.iter().map(|id| id.0.clone()).collect(),
    }
}

async fn apply_mutation_owned(
    runtime: &mut SimulationRuntime,
    mutation: crate::runtime_view::Mutation,
) {
    use crate::runtime_view::Mutation;
    match mutation {
        Mutation::ApplyCommand { command, reply } => {
            let result = match abutown_protocol::ClientCommandDto::try_from(command) {
                Ok(dto) => runtime.apply_client_command(dto).await,
                Err((code, message)) => Err(crate::commands::CommandRejection {
                    world_id: None,
                    command_id: None,
                    code,
                    message: message.to_string(),
                }),
            };
            let _ = reply.send(result);
        }
        Mutation::SubscriptionDiff {
            added,
            removed,
            reply,
        } => {
            runtime.apply_subscription_diff(added.iter(), removed.iter());
            let world_id = runtime.world_id_for_persist().clone();
            let tick = runtime.mobility_tick();
            let snapshots: Vec<w::MobilityChunkSnapshot> = added
                .iter()
                .map(|coord| {
                    let snapshot = runtime.mobility().build_chunk_snapshot(*coord);
                    chunk_snapshot_to_dto(&snapshot, runtime.mobility(), &world_id, tick)
                })
                .collect();
            let _ = reply.send(snapshots);
        }
        Mutation::MarkChunkSnapshotsPersisted { coords } => {
            runtime.mark_chunk_snapshots_persisted(&coords);
        }
        Mutation::CollectPersistData { reply } => {
            let payload = crate::runtime_view::PersistPayload {
                chunk_snapshots: runtime.collect_chunk_snapshots(),
                world_id: runtime.world_id_for_persist().clone(),
                mobility_tick: runtime.mobility_tick(),
                mobility_world: runtime.mobility_world_clone_for_persist(),
            };
            let _ = reply.send(payload);
        }
    }
}

/// Sole owner of the SimulationRuntime. Drains pending mutations, ticks the
/// world, fans out per-chunk deltas, publishes a new RuntimeReadView, and
/// emits the legacy global tile pulse. All in one task so no lock is needed.
async fn tick_loop(
    mut runtime: SimulationRuntime,
    mut mutation_rx: tokio::sync::mpsc::UnboundedReceiver<crate::runtime_view::Mutation>,
    view: Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
    deltas: broadcast::Sender<w::ServerMessage>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        tick_once(
            &mut runtime,
            &mut mutation_rx,
            &view,
            &deltas,
            &chunk_channels,
        )
        .await;
    }
}

/// One iteration of the tick loop, extracted so tests can advance the world
/// without waiting on the real-time scheduler.
async fn tick_once(
    runtime: &mut SimulationRuntime,
    mutation_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::runtime_view::Mutation>,
    view: &Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
    deltas: &broadcast::Sender<w::ServerMessage>,
    chunk_channels: &Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>>,
) {
    // Phase 0: drain the entire mutation queue. We own the runtime exclusively
    // — no lock acquisition between mutations.
    while let Ok(mutation) = mutation_rx.try_recv() {
        apply_mutation_owned(runtime, mutation).await;
    }

    // Phase 1: tick mobility.
    let per_chunk = runtime.tick_world_mobility();
    let world_id = runtime.world_id_for_persist().clone();
    let mobility_tick = runtime.mobility_tick();

    // Phase 2: broadcast per-chunk delta DTOs to subscribed chunk channels.
    if !per_chunk.is_empty() {
        let mobility = runtime.mobility();
        for (chunk, delta) in &per_chunk {
            let Some(sender) = chunk_channels.get(chunk).map(|e| e.clone()) else {
                continue;
            };
            let dto = chunk_delta_to_dto(delta, mobility, &world_id, mobility_tick);
            let _ = sender.send(dto); // best-effort
        }
    }

    // Phase 3: publish the new RuntimeReadView for lock-free HTTP/WS readers.
    let prev_pulse = view.load().pulse_sequence;
    let new_view = build_read_view_from_runtime(runtime, &per_chunk, prev_pulse + 1);
    view.store(Arc::new(new_view));

    // Phase 4: legacy global tile pulse via the broadcast channel.
    // next_pulse() still returns the serde DTO; convert it to the wire
    // ServerMessage envelope here so the broadcast channel carries proto.
    let pulse_legacy = runtime.next_pulse();
    let pulse_msg = match pulse_legacy {
        abutown_protocol::ServerMessageDto::TilePulse(p) => w::ServerMessage {
            body: Some(w::server_message::Body::TilePulse(tile_pulse_dto_to_proto(
                &p,
            ))),
        },
        // next_pulse is documented to only return TilePulse; any other variant
        // would be a programming error — emit an explicit Error envelope so
        // it's visible on the wire rather than silently dropped.
        other => {
            tracing::error!(?other, "next_pulse returned non-TilePulse variant");
            w::ServerMessage {
                body: Some(w::server_message::Body::Error(w::ServerError {
                    protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
                    world_id: String::new(),
                    code: "internal".into(),
                    message: "next_pulse returned non-TilePulse variant".into(),
                })),
            }
        }
    };
    let _ = deltas.send(pulse_msg);
}

fn chunk_snapshot_to_dto(
    snapshot: &sim_core::mobility::MobilityChunkSnapshot,
    world: &sim_core::mobility::MobilityWorld,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> w::MobilityChunkSnapshot {
    w::MobilityChunkSnapshot {
        protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
        world_id: world_id.0.clone(),
        tick,
        chunk: Some(w::ChunkCoord {
            x: snapshot.chunk.x,
            y: snapshot.chunk.y,
        }),
        agents: snapshot
            .agents
            .iter()
            .filter_map(|record| world.agent_dto_for(&record.id))
            .map(agent_dto_to_proto)
            .collect(),
        vehicles: snapshot
            .vehicles
            .iter()
            .filter_map(|record| world.vehicle_dto_for(&record.id))
            .map(vehicle_dto_to_proto)
            .collect(),
    }
}

async fn handle_client_message(
    state: &AppState,
    message: &w::ClientMessage,
    connection: &mut ConnectionState,
) -> Vec<w::ServerMessage> {
    use w::client_message::Body;
    let mut out: Vec<w::ServerMessage> = Vec::new();

    match &message.body {
        Some(Body::ChunkSubscribe(payload)) => {
            let added: Vec<sim_core::ids::ChunkCoord> = payload
                .coords
                .iter()
                .map(|c| sim_core::ids::ChunkCoord { x: c.x, y: c.y })
                .filter(|c| connection.subscription.insert(*c))
                .collect();

            if !added.is_empty() {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if state
                    .mutations
                    .send(crate::runtime_view::Mutation::SubscriptionDiff {
                        added: added.clone(),
                        removed: Vec::new(),
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    // Mutation channel closed (tick task gone). Roll back the
                    // optimistic insert into connection.subscription so the
                    // disconnect cleanup doesn't try to un-subscribe chunks
                    // the runtime never registered.
                    for coord in &added {
                        connection.subscription.remove(coord);
                    }
                    return out;
                }

                // Set up per-chunk channel subscriptions while we wait for the
                // reply — latency optimization, the channel setup is
                // independent of the runtime mutation.
                let chunk_channels = state.chunk_channels();
                for coord in &added {
                    let sender = chunk_channels
                        .entry(*coord)
                        .or_insert_with(|| broadcast::channel(8).0)
                        .clone();
                    let receiver = sender.subscribe();
                    connection
                        .chunk_streams
                        .insert(*coord, BroadcastStream::new(receiver));
                }

                // Receive initial snapshots from the tick task. Timeout at
                // 5× tick interval protects against a stalled tick task
                // hanging the WS handler indefinitely.
                let reply = tokio::time::timeout(
                    SIMULATION_TICK_INTERVAL.saturating_mul(5),
                    reply_rx,
                )
                .await;
                match reply {
                    Ok(Ok(snapshots)) => {
                        for snap in snapshots {
                            out.push(w::ServerMessage {
                                body: Some(w::server_message::Body::MobilityChunkSnapshot(snap)),
                            });
                        }
                    }
                    Ok(Err(_)) | Err(_) => {
                        // Reply channel dropped OR timed out — tick task
                        // crashed mid-drain, oneshot was canceled, or the
                        // task is stalled. Roll back so we don't have
                        // streams pointing at chunks the runtime never
                        // registered as ours.
                        tracing::warn!(
                            "chunk_subscribe reply unavailable; rolling back local state"
                        );
                        for coord in &added {
                            connection.subscription.remove(coord);
                            connection.chunk_streams.remove(coord);
                        }
                    }
                }
            }
        }

        Some(Body::ChunkUnsubscribe(payload)) => {
            let removed: Vec<sim_core::ids::ChunkCoord> = payload
                .coords
                .iter()
                .map(|c| sim_core::ids::ChunkCoord { x: c.x, y: c.y })
                .filter(|c| connection.subscription.remove(c))
                .collect();

            if !removed.is_empty() {
                let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
                let _ = state
                    .mutations
                    .send(crate::runtime_view::Mutation::SubscriptionDiff {
                        added: Vec::new(),
                        removed: removed.clone(),
                        reply: reply_tx,
                    });

                let chunk_channels = state.chunk_channels();
                for coord in &removed {
                    connection.chunk_streams.remove(coord);
                    // View was published last tick — counts reflect the previous
                    // state. At worst we keep a chunk_channel for one extra tick
                    // before reaping; correctness unaffected.
                    let count = state
                        .view()
                        .load()
                        .chunk_subscriber_counts
                        .get(coord)
                        .copied()
                        .unwrap_or(0);
                    if count == 0 {
                        chunk_channels.remove(coord);
                    }
                }
            }
        }

        None => {}
    }

    out
}

async fn persist_snapshots_once(
    state: &AppState,
) -> Result<usize, sim_core::persistence::ChunkSnapshotStoreError> {
    // Phase 1: ask the tick task to collect everything we need. The reply
    // arrives at the next mutation-drain (≤ one tick interval).
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .mutations
        .send(crate::runtime_view::Mutation::CollectPersistData { reply: reply_tx })
        .is_err()
    {
        return Err(sim_core::persistence::ChunkSnapshotStoreError::unavailable(
            "tick task gone",
        ));
    }
    // Timeout at 5× tick interval — generous (500 ms at 100 ms tick) but
    // protects against a stalled tick task hanging the persist loop
    // indefinitely. A real outage will return an error and let the snapshot
    // loop schedule a retry on its next interval.
    let payload = match tokio::time::timeout(
        SIMULATION_TICK_INTERVAL.saturating_mul(5),
        reply_rx,
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(_)) => {
            return Err(sim_core::persistence::ChunkSnapshotStoreError::unavailable(
                "collect-persist-data reply dropped",
            ));
        }
        Err(_) => {
            tracing::warn!(
                "collect-persist-data timed out waiting for tick task; will retry next cycle"
            );
            return Err(sim_core::persistence::ChunkSnapshotStoreError::unavailable(
                "collect-persist-data timed out",
            ));
        }
    };

    let crate::runtime_view::PersistPayload {
        chunk_snapshots: snapshots,
        world_id,
        mobility_tick,
        mobility_world,
    } = payload;

    let coords: Vec<ChunkCoord> = snapshots
        .iter()
        .map(|s| ChunkCoord {
            x: s.coord.x,
            y: s.coord.y,
        })
        .collect();
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
            .write(&world_id.0, mobility_tick, &mobility_world)
            .await
        {
            tracing::warn!(%error, "failed to persist mobility snapshot");
        }
    }

    // Phase 3: send a fire-and-forget Mutation to mark snapshots persisted.
    let _ = state
        .mutations
        .send(crate::runtime_view::Mutation::MarkChunkSnapshotsPersisted {
            coords: coords.clone(),
        });

    Ok(written)
}

async fn send_server_message(
    socket: &mut WebSocket,
    message: w::ServerMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bytes = message.encode_to_vec();
    socket.send(Message::Binary(bytes.into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use abutown_protocol::ChunkSnapshotDto;
    use sim_core::ids::ChunkCoord;
    use sim_core::persistence::{ChunkSnapshotStore, ChunkSnapshotStoreError};
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
        use std::time::Instant;

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
    async fn subscription_diff_mutation_returns_snapshots_for_added_chunks() {
        let state = AppState::new(SimulationRuntime::new());
        // Wait one tick so the view is populated.
        let tick0 = state.view().load().mobility_tick;
        wait_for_tick_past(&state, tick0, TICK_WAIT).await;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        state
            .mutations
            .send(crate::runtime_view::Mutation::SubscriptionDiff {
                added: vec![sim_core::ids::ChunkCoord { x: 4, y: 4 }],
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
        assert_eq!(chunk.x, 4);
        assert_eq!(chunk.y, 4);
    }

    #[tokio::test]
    async fn dropped_reply_channel_does_not_panic() {
        let state = AppState::new(SimulationRuntime::new());
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        drop(reply_rx); // drop receiver before the mutation is processed
        state
            .mutations
            .send(crate::runtime_view::Mutation::SubscriptionDiff {
                added: vec![sim_core::ids::ChunkCoord { x: 4, y: 4 }],
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
}
