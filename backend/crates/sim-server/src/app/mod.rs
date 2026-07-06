use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::{
    Json, Router,
    extract::State,
    http::{
        HeaderMap, StatusCode,
        header::{self, HeaderValue},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{
    card_hand::{
        AuthVerifier, CardHandError, CardHandResponse, CardHandStore, SaveCardHandRequest,
        card_definitions,
    },
    config::ServerConfig,
    db::connect_shared_pool,
};

/// Shared world liveness mirror for `/health` (Task 13): the tick loop stores
/// the current world tick + audit state after every tick, and boot records
/// whether the world resumed from a persisted snapshot. Atomics, not the ECS
/// world — the HTTP task must never touch sim state.
#[derive(Debug)]
pub struct WorldHealth {
    pub world_tick: AtomicU64,
    pub audit_ok: AtomicBool,
    pub resumed: AtomicBool,
}

impl Default for WorldHealth {
    fn default() -> Self {
        WorldHealth {
            world_tick: AtomicU64::new(0),
            // Fail-fast audit: the process being alive means no violation.
            audit_ok: AtomicBool::new(true),
            resumed: AtomicBool::new(false),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    card_hands: CardHandStore,
    auth: AuthVerifier,
    health: Arc<WorldHealth>,
}

impl AppState {
    pub fn new(card_hands: CardHandStore, auth: AuthVerifier, health: Arc<WorldHealth>) -> Self {
        Self {
            card_hands,
            auth,
            health,
        }
    }
}

/// Production wiring: Supabase JWT auth + Postgres-backed card hands.
pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let pool = connect_shared_pool(&config.database_url).await?;
    build_app_with_shared_pool(config, pool, Arc::new(WorldHealth::default())).await
}

/// Production wiring over an ALREADY-connected shared pool (the sim-server
/// boot connects once and hands the same pool to the card-hand store and the
/// world snapshot store), plus the shared `/health` world mirror.
pub async fn build_app_with_shared_pool(
    config: &ServerConfig,
    pool: PgPool,
    health: Arc<WorldHealth>,
) -> anyhow::Result<Router> {
    let card_hands = CardHandStore::with_pool(pool).await?;
    let auth = AuthVerifier::supabase(&config.supabase_url).await;
    let cors = cors_layer(&config.cors_allowed_origins)?;
    Ok(build_router(AppState::new(card_hands, auth, health), cors))
}

/// Test/dev wiring: in-memory card hands + local bearer-as-UUID auth.
pub fn build_app_with_card_hands(
    card_hands: CardHandStore,
    auth: AuthVerifier,
    health: Arc<WorldHealth>,
) -> Router {
    // infallible: hardcoded empty origin slice can never contain a malformed origin
    let cors = cors_layer(&[]).expect("empty origin list is always valid");
    build_router(AppState::new(card_hands, auth, health), cors)
}

/// Convenience default used by the integration tests.
pub fn build_app() -> Router {
    build_app_with_card_hands(
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
        Arc::new(WorldHealth::default()),
    )
}

/// Fail-closed CORS from an explicit allow-list. Empty list allows no
/// cross-origin requests; a malformed origin is a startup error.
fn cors_layer(allowed_origins: &[String]) -> anyhow::Result<CorsLayer> {
    use axum::http::Method;

    let origins = allowed_origins
        .iter()
        .map(|origin| {
            origin
                .parse::<HeaderValue>()
                .map_err(|err| anyhow::anyhow!("invalid CORS origin {origin:?}: {err}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
}

fn build_router(state: AppState, cors: CorsLayer) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/cards", get(cards))
        .route("/card-hand", get(card_hand).put(save_card_hand))
        .with_state(state)
        .layer(cors)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "abutown-sim",
        "ok": true,
        "world_tick": state.health.world_tick.load(Ordering::Relaxed),
        "audit_ok": state.health.audit_ok.load(Ordering::Relaxed),
        "resumed": state.health.resumed.load(Ordering::Relaxed),
    }))
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
