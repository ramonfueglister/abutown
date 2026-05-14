use std::{sync::Arc, time::Duration};

use abutown_protocol::{ChunkSnapshotDto, HealthResponse, ServerMessageDto, WorldSummaryDto};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use sim_core::ids::ChunkCoord;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

use crate::runtime::SimulationRuntime;

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<Mutex<SimulationRuntime>>,
}

impl AppState {
    pub fn new(runtime: SimulationRuntime) -> Self {
        Self {
            runtime: Arc::new(Mutex::new(runtime)),
        }
    }

    pub(crate) fn runtime(&self) -> Arc<Mutex<SimulationRuntime>> {
        Arc::clone(&self.runtime)
    }
}

pub fn build_app() -> Router {
    build_app_with_runtime(SimulationRuntime::new())
}

pub fn build_app_with_runtime(runtime: SimulationRuntime) -> Router {
    let state = AppState::new(runtime);

    Router::new()
        .route("/health", get(health))
        .route("/world", get(world))
        .route("/chunks/{x}/{y}", get(chunk))
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

async fn websocket(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_world_deltas(socket, state))
}

// Temporary visible-slice stream: each connection drives its own pulses. Production
// must move ticking to one scheduler/broadcast source so client count cannot advance time.
async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let hello = {
        let runtime = state.runtime();
        let runtime = runtime.lock().await;
        runtime.hello()
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.tick().await;
    loop {
        interval.tick().await;
        let pulse = {
            let runtime = state.runtime();
            let mut runtime = runtime.lock().await;
            runtime.next_pulse()
        };

        if send_server_message(&mut socket, pulse).await.is_err() {
            return;
        }
    }
}

async fn send_server_message(
    socket: &mut WebSocket,
    message: ServerMessageDto,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let text = serde_json::to_string(&message)?;

    socket.send(Message::Text(text.into())).await?;
    Ok(())
}
