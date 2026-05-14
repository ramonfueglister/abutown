use std::sync::Arc;

use abutown_protocol::{ChunkSnapshotDto, HealthResponse, WorldSummaryDto};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
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
