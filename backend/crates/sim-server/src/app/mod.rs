use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, StatusCode,
        header::{self, HeaderValue},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{
    card_hand::{
        AuthVerifier, CardHandError, CardHandResponse, CardHandStore, SaveCardHandRequest,
        card_definitions,
    },
    config::ServerConfig,
    db::connect_shared_pool,
};

#[derive(Clone)]
pub struct AppState {
    card_hands: CardHandStore,
    auth: AuthVerifier,
}

impl AppState {
    pub fn new(card_hands: CardHandStore, auth: AuthVerifier) -> Self {
        Self { card_hands, auth }
    }
}

/// Production wiring: Supabase JWT auth + Postgres-backed card hands.
pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let pool = connect_shared_pool(&config.database_url).await?;
    let card_hands = CardHandStore::with_pool(pool).await?;
    let auth = AuthVerifier::supabase(&config.supabase_url).await;
    let cors = cors_layer(&config.cors_allowed_origins)?;
    Ok(build_router(AppState::new(card_hands, auth), cors))
}

/// Test/dev wiring: in-memory card hands + local bearer-as-UUID auth.
pub fn build_app_with_card_hands(card_hands: CardHandStore, auth: AuthVerifier) -> Router {
    // infallible: hardcoded empty origin slice can never contain a malformed origin
    let cors = cors_layer(&[]).expect("empty origin list is always valid");
    build_router(AppState::new(card_hands, auth), cors)
}

/// Convenience default used by the integration tests.
pub fn build_app() -> Router {
    build_app_with_card_hands(CardHandStore::memory(), AuthVerifier::local_bearer_uuid())
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
        .route("/ws", get(websocket))
        .with_state(state)
        .layer(cors)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "abutown-cards", "ok": true }))
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

/// WebSocket stub — accepts the upgrade and closes immediately. Extension
/// point for the next simulation's live channel; intentionally carries no
/// simulation logic today.
async fn websocket(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut socket: WebSocket| async move {
        let _ = socket.send(Message::Close(None)).await;
    })
}
