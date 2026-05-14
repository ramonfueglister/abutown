pub mod app {
    use abutown_protocol::{
        ChunkCoordDto, HealthResponse, PROTOCOL_VERSION, WorldId, WorldSummaryDto,
    };
    use axum::{Json, Router, extract::Path, http::StatusCode, routing::get};
    use sim_core::{
        chunk::Chunk, ids::ChunkCoord, persistence::build_chunk_snapshot, scheduler::ChunkActivity,
        tile::TileKind,
    };

    pub fn build_app() -> Router {
        Router::new()
            .route("/health", get(health))
            .route("/world", get(world))
            .route("/chunks/{x}/{y}", get(chunk))
    }

    async fn health() -> Json<HealthResponse> {
        Json(HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        })
    }

    async fn world() -> Json<WorldSummaryDto> {
        Json(WorldSummaryDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            chunk_size: 32,
            loaded_chunks: vec![ChunkCoordDto { x: 0, y: 0 }],
        })
    }

    async fn chunk(
        Path((x, y)): Path<(i32, i32)>,
    ) -> Result<Json<abutown_protocol::ChunkSnapshotDto>, StatusCode> {
        if x != 0 || y != 0 {
            return Err(StatusCode::NOT_FOUND);
        }

        let mut chunk = Chunk::new(ChunkCoord { x, y }, 32);
        chunk
            .set_tile_kind(0, TileKind::Road)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        Ok(Json(snapshot))
    }
}
