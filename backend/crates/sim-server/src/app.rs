use std::{sync::Arc, time::Duration};

use abutown_protocol::{
    ChunkSnapshotDto, ClientCommandDto, CommandResponseDto, HealthResponse, MobilitySnapshotDto,
    ServerMessageDto, WorldSummaryDto,
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
use sim_core::ids::ChunkCoord;
use tokio::sync::{Mutex, broadcast};
use tower_http::cors::CorsLayer;

use crate::{
    card_hand::{
        AuthVerifier, CardHandError, CardHandResponse, CardHandStore, SaveCardHandRequest,
        card_definitions,
    },
    config::ServerConfig,
    postgres_events::PostgresWorldEventStore,
    postgres_snapshots::PostgresChunkSnapshotStore,
    runtime::SimulationRuntime,
};

const DELTA_BROADCAST_CAPACITY: usize = 64;
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_secs(1);
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<Mutex<SimulationRuntime>>,
    deltas: broadcast::Sender<ServerMessageDto>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
}

impl AppState {
    pub fn new(runtime: SimulationRuntime) -> Self {
        Self::new_with_card_hands(
            runtime,
            CardHandStore::memory(),
            AuthVerifier::local_bearer_uuid(),
        )
    }

    pub fn new_with_card_hands(
        runtime: SimulationRuntime,
        card_hands: CardHandStore,
        auth: AuthVerifier,
    ) -> Self {
        let (deltas, _) = broadcast::channel(DELTA_BROADCAST_CAPACITY);
        Self {
            runtime: Arc::new(Mutex::new(runtime)),
            deltas,
            card_hands,
            auth,
        }
    }

    pub(crate) fn runtime(&self) -> Arc<Mutex<SimulationRuntime>> {
        Arc::clone(&self.runtime)
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
                    let mut runtime = runtime.lock().await;
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
    build_app_with_runtime(SimulationRuntime::new())
}

pub async fn build_app_from_env() -> anyhow::Result<Router> {
    let _ = dotenvy::dotenv();
    let config = ServerConfig::from_env()?;
    build_app_from_config(&config).await
}

pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let event_store = PostgresWorldEventStore::connect(&config.database_url).await?;
    let snapshot_store = PostgresChunkSnapshotStore::connect(
        &config.database_url,
        SimulationRuntime::default_world_id(),
    )
    .await?;
    let card_hands = CardHandStore::postgres(&config.database_url).await?;
    let auth = AuthVerifier::supabase(&config.supabase_url).await;

    let runtime =
        SimulationRuntime::hydrate_from_stores(Box::new(event_store), Box::new(snapshot_store))
            .await?;

    Ok(build_app_with_runtime_and_card_hands(
        runtime, card_hands, auth,
    ))
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
    let runtime = runtime.lock().await;
    Json(runtime.health())
}

async fn world(State(state): State<AppState>) -> Json<WorldSummaryDto> {
    let runtime = state.runtime();
    let runtime = runtime.lock().await;
    Json(runtime.world_summary())
}

async fn mobility(State(state): State<AppState>) -> Json<MobilitySnapshotDto> {
    let runtime = state.runtime();
    let runtime = runtime.lock().await;
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
    let runtime = runtime.lock().await;
    runtime
        .chunk_snapshot(ChunkCoord { x, y })
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn command(State(state): State<AppState>, Json(command): Json<ClientCommandDto>) -> Response {
    let result = {
        let runtime = state.runtime();
        let mut runtime = runtime.lock().await;
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

async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let mut deltas = state.subscribe_deltas();
    let hello = {
        let runtime = state.runtime();
        let runtime = runtime.lock().await;
        runtime.hello()
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    loop {
        let message = match deltas.recv().await {
            Ok(message) => message,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };

        if send_server_message(&mut socket, message).await.is_err() {
            return;
        }
    }
}

async fn persist_snapshots_once(
    state: &AppState,
) -> Result<usize, sim_core::persistence::ChunkSnapshotStoreError> {
    let runtime = state.runtime();
    let mut runtime = runtime.lock().await;
    runtime.persist_chunk_snapshots().await
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

    #[tokio::test]
    async fn persist_snapshots_once_writes_runtime_snapshots() {
        let state = AppState::new(SimulationRuntime::new());

        assert_eq!(persist_snapshots_once(&state).await.unwrap(), 3);

        let runtime = state.runtime();
        let runtime = runtime.lock().await;
        let snapshot = runtime
            .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .await
            .unwrap()
            .expect("visible snapshot stored");
        assert_eq!(snapshot.coord.x, 4);
        assert_eq!(snapshot.coord.y, 4);
    }
}
