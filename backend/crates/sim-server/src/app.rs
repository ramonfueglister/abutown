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
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sim_core::ids::ChunkCoord;
use tokio::sync::{Mutex, broadcast};
use tower_http::cors::CorsLayer;

use crate::runtime::SimulationRuntime;

const DELTA_BROADCAST_CAPACITY: usize = 64;
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_secs(1);
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<Mutex<SimulationRuntime>>,
    deltas: broadcast::Sender<ServerMessageDto>,
}

impl AppState {
    pub fn new(runtime: SimulationRuntime) -> Self {
        let (deltas, _) = broadcast::channel(DELTA_BROADCAST_CAPACITY);
        Self {
            runtime: Arc::new(Mutex::new(runtime)),
            deltas,
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
                let _ = persist_snapshots_once(&state).await;
            }
        });
    }
}

pub fn build_app() -> Router {
    build_app_with_runtime(SimulationRuntime::new())
}

pub fn build_app_with_runtime(runtime: SimulationRuntime) -> Router {
    let state = AppState::new(runtime);
    state.spawn_delta_loop(SIMULATION_TICK_INTERVAL);
    state.spawn_snapshot_loop(SNAPSHOT_INTERVAL);

    Router::new()
        .route("/health", get(health))
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
        runtime.apply_client_command(command)
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

async fn persist_snapshots_once(state: &AppState) -> usize {
    let runtime = state.runtime();
    let mut runtime = runtime.lock().await;
    runtime.persist_chunk_snapshots()
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

        assert_eq!(persist_snapshots_once(&state).await, 3);

        let runtime = state.runtime();
        let runtime = runtime.lock().await;
        let snapshot = runtime
            .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible snapshot stored");
        assert_eq!(snapshot.coord.x, 4);
        assert_eq!(snapshot.coord.y, 4);
    }
}
