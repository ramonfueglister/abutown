use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, RwLock},
};

use axum::http::HeaderMap;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::Mutex;
use uuid::Uuid;

const CARD_HAND_MIGRATION: &str = include_str!("../migrations/202605150002_card_hand_core.sql");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardDefinition {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub card_type: String,
    pub mana_cost: i32,
    pub description: String,
    pub rarity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandCard {
    pub instance_id: u32,
    pub card_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardHandResponse {
    pub user_id: String,
    pub cards: Vec<HandCard>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveCardHandRequest {
    pub cards: Vec<HandCard>,
}

#[derive(Debug, thiserror::Error)]
pub enum CardHandError {
    #[error("missing bearer token")]
    MissingAuth,
    #[error("invalid bearer token")]
    InvalidAuth,
    #[error("unknown card_id {0}")]
    UnknownCard(String),
    #[error("database unavailable: {0}")]
    Database(String),
}

#[derive(Clone)]
pub struct CardHandStore {
    inner: Arc<CardHandStoreInner>,
}

enum CardHandStoreInner {
    Memory(Mutex<HashMap<Uuid, Vec<HandCard>>>),
    Postgres(PgPool),
}

impl CardHandStore {
    pub fn memory() -> Self {
        Self {
            inner: Arc::new(CardHandStoreInner::Memory(Mutex::new(HashMap::new()))),
        }
    }

    pub async fn postgres(database_url: &str) -> Result<Self, CardHandError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| CardHandError::Database(error.to_string()))?;

        for statement in CARD_HAND_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| CardHandError::Database(error.to_string()))?;
        }

        Ok(Self {
            inner: Arc::new(CardHandStoreInner::Postgres(pool)),
        })
    }

    pub async fn get_or_create(&self, user_id: Uuid) -> Result<Vec<HandCard>, CardHandError> {
        if let Some(cards) = self.load(user_id).await? {
            return Ok(cards);
        }

        let cards = build_default_hand(card_ids(), 5);
        self.save(user_id, cards.clone()).await?;
        Ok(cards)
    }

    pub async fn save(&self, user_id: Uuid, cards: Vec<HandCard>) -> Result<(), CardHandError> {
        validate_hand_cards(&cards)?;
        match self.inner.as_ref() {
            CardHandStoreInner::Memory(hands) => {
                hands.lock().await.insert(user_id, cards);
                Ok(())
            }
            CardHandStoreInner::Postgres(pool) => {
                let cards = serde_json::to_value(cards)
                    .map_err(|error| CardHandError::Database(error.to_string()))?;
                sqlx::query(
                    "insert into user_card_hands (user_id, cards) values ($1::uuid, $2) \
                     on conflict (user_id) do update set cards = excluded.cards, updated_at = now()",
                )
                .bind(user_id.to_string())
                .bind(cards)
                .execute(pool)
                .await
                .map_err(|error| CardHandError::Database(error.to_string()))?;
                Ok(())
            }
        }
    }

    async fn load(&self, user_id: Uuid) -> Result<Option<Vec<HandCard>>, CardHandError> {
        match self.inner.as_ref() {
            CardHandStoreInner::Memory(hands) => Ok(hands.lock().await.get(&user_id).cloned()),
            CardHandStoreInner::Postgres(pool) => {
                let row: Option<(serde_json::Value,)> =
                    sqlx::query_as("select cards from user_card_hands where user_id = $1::uuid")
                        .bind(user_id.to_string())
                        .fetch_optional(pool)
                        .await
                        .map_err(|error| CardHandError::Database(error.to_string()))?;
                row.map(|(cards,)| {
                    serde_json::from_value(cards)
                        .map_err(|error| CardHandError::Database(error.to_string()))
                })
                .transpose()
            }
        }
    }
}

#[derive(Clone)]
pub enum AuthVerifier {
    LocalBearerUuid,
    Supabase(Arc<JwksCache>),
}

impl AuthVerifier {
    pub fn local_bearer_uuid() -> Self {
        Self::LocalBearerUuid
    }

    pub async fn supabase(supabase_url: &str) -> Self {
        Self::Supabase(Arc::new(JwksCache::new(supabase_url).await))
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<Uuid, CardHandError> {
        let token = bearer_token(headers).ok_or(CardHandError::MissingAuth)?;
        match self {
            AuthVerifier::LocalBearerUuid => {
                Uuid::parse_str(token).map_err(|_| CardHandError::InvalidAuth)
            }
            AuthVerifier::Supabase(jwks) => jwks.validate(token).await,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JwksKey {
    kid: String,
    kty: String,
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwksKey>,
}

pub struct JwksCache {
    keys: Arc<RwLock<Vec<(String, Algorithm, DecodingKey)>>>,
    jwks_url: String,
    expected_iss: String,
    http: reqwest::Client,
}

impl JwksCache {
    async fn new(supabase_url: &str) -> Self {
        let cache = Self {
            keys: Arc::new(RwLock::new(Vec::new())),
            jwks_url: format!("{}/auth/v1/.well-known/jwks.json", supabase_url),
            expected_iss: format!("{}/auth/v1", supabase_url),
            http: reqwest::Client::new(),
        };
        let _ = cache.refresh().await;
        cache
    }

    async fn validate(&self, token: &str) -> Result<Uuid, CardHandError> {
        if let Ok(user_id) = self.try_validate(token) {
            return Ok(user_id);
        }
        self.refresh().await?;
        self.try_validate(token)
    }

    async fn refresh(&self) -> Result<(), CardHandError> {
        let resp: JwksResponse = self
            .http
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| CardHandError::InvalidAuth)?
            .json()
            .await
            .map_err(|_| CardHandError::InvalidAuth)?;

        let mut next = Vec::new();
        for key in resp.keys {
            let decoded = match key.kty.as_str() {
                "EC" => {
                    let (Some(x), Some(y)) = (key.x.as_deref(), key.y.as_deref()) else {
                        continue;
                    };
                    DecodingKey::from_ec_components(x, y).map(|key| (key, Algorithm::ES256))
                }
                "RSA" => {
                    let (Some(n), Some(e)) = (key.n.as_deref(), key.e.as_deref()) else {
                        continue;
                    };
                    DecodingKey::from_rsa_components(n, e).map(|key| (key, Algorithm::RS256))
                }
                _ => continue,
            };
            if let Ok((decoding_key, algorithm)) = decoded {
                next.push((key.kid, algorithm, decoding_key));
            }
        }
        *self.keys.write().map_err(|_| CardHandError::InvalidAuth)? = next;
        Ok(())
    }

    fn try_validate(&self, token: &str) -> Result<Uuid, CardHandError> {
        let keys = self.keys.read().map_err(|_| CardHandError::InvalidAuth)?;
        for (_, algorithm, decoding_key) in keys.iter() {
            let mut validation = Validation::new(*algorithm);
            validation.set_audience(&["authenticated"]);
            validation.set_issuer(&[&self.expected_iss]);
            if let Ok(data) = decode::<Claims>(token, decoding_key, &validation) {
                return Uuid::parse_str(&data.claims.sub).map_err(|_| CardHandError::InvalidAuth);
            }
        }
        Err(CardHandError::InvalidAuth)
    }
}

pub fn card_definitions() -> Vec<CardDefinition> {
    [
        ("strike", "Strike", "attack", 1, "Deal damage.", "starter"),
        ("defend", "Defend", "skill", 1, "Gain block.", "starter"),
        (
            "bash",
            "Bash",
            "attack",
            2,
            "A heavier starter attack.",
            "starter",
        ),
        (
            "guard",
            "Guard",
            "skill",
            1,
            "Prepare a stable defense.",
            "common",
        ),
        (
            "focus",
            "Focus",
            "power",
            1,
            "Keep this card as reusable content.",
            "common",
        ),
    ]
    .into_iter()
    .map(
        |(id, name, card_type, mana_cost, description, rarity)| CardDefinition {
            id: id.to_string(),
            name: name.to_string(),
            card_type: card_type.to_string(),
            mana_cost,
            description: description.to_string(),
            rarity: rarity.to_string(),
        },
    )
    .collect()
}

fn card_ids() -> Vec<String> {
    card_definitions().into_iter().map(|card| card.id).collect()
}

fn build_default_hand(mut ids: Vec<String>, size: usize) -> Vec<HandCard> {
    ids.sort();
    let preferred = ["strike", "defend", "bash", "guard", "focus"];
    let mut selected = Vec::new();
    for wanted in preferred {
        if selected.len() >= size {
            break;
        }
        if ids.iter().any(|id| id == wanted) {
            selected.push(wanted.to_string());
        }
    }
    for id in ids {
        if selected.len() >= size {
            break;
        }
        if !selected.contains(&id) {
            selected.push(id);
        }
    }
    selected
        .into_iter()
        .enumerate()
        .map(|(index, card_id)| HandCard {
            instance_id: (index + 1) as u32,
            card_id,
        })
        .collect()
}

fn validate_hand_cards(cards: &[HandCard]) -> Result<(), CardHandError> {
    let allowed: BTreeSet<String> = card_ids().into_iter().collect();
    for card in cards {
        if !allowed.contains(&card.card_id) {
            return Err(CardHandError::UnknownCard(card.card_id.clone()));
        }
    }
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hand_uses_stable_starter_cards() {
        let hand = build_default_hand(card_ids(), 5);

        assert_eq!(hand.len(), 5);
        assert_eq!(hand[0].instance_id, 1);
        assert_eq!(hand[0].card_id, "strike");
        assert!(hand.iter().any(|card| card.card_id == "defend"));
    }

    #[test]
    fn validation_rejects_unknown_card_ids() {
        let err = validate_hand_cards(&[HandCard {
            instance_id: 1,
            card_id: "missing".to_string(),
        }])
        .unwrap_err();

        assert!(matches!(err, CardHandError::UnknownCard(id) if id == "missing"));
    }
}
