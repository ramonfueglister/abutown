use abutown_protocol::WorldEventDto;
use async_trait::async_trait;
use serde_json::Value;
use sim_core::events::{WorldEventMetadata, WorldEventStore, WorldEventStoreError};
use sqlx::{PgPool, postgres::PgPoolOptions};

const WORLD_EVENTS_MIGRATION: &str = include_str!("../migrations/202605150001_world_events.sql");

#[derive(Debug, Clone, PartialEq)]
pub struct SqlWorldEventRecord {
    pub metadata: WorldEventMetadata,
    pub payload: Value,
}

impl SqlWorldEventRecord {
    pub fn from_event(event: &WorldEventDto) -> Result<Self, WorldEventStoreError> {
        let payload = serde_json::to_value(event)
            .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        Ok(Self {
            metadata: WorldEventMetadata::from_event(event),
            payload,
        })
    }
}

#[derive(Debug)]
pub struct PostgresWorldEventStore {
    pool: PgPool,
}

impl PostgresWorldEventStore {
    pub async fn connect(database_url: &str) -> Result<Self, WorldEventStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;

        for statement in WORLD_EVENTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }
}

#[async_trait]
impl WorldEventStore for PostgresWorldEventStore {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        let record = SqlWorldEventRecord::from_event(&event)?;
        let tick = i64::try_from(record.metadata.tick)
            .map_err(|_| WorldEventStoreError::unavailable("event tick exceeds i64"))?;
        let version = i64::try_from(record.metadata.version)
            .map_err(|_| WorldEventStoreError::unavailable("event version exceeds i64"))?;

        sqlx::query(
            r#"
            INSERT INTO world_events (
                event_id,
                world_id,
                command_id,
                event_type,
                tick,
                version,
                payload
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(&record.metadata.event_id)
        .bind(&record.metadata.world_id)
        .bind(&record.metadata.command_id)
        .bind(record.metadata.event_type)
        .bind(tick)
        .bind(version)
        .bind(record.payload)
        .execute(&self.pool)
        .await
        .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abutown_protocol::{
        ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldId,
    };

    pub(crate) fn tile_event(event_id: &str, version: u64) -> WorldEventDto {
        WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: event_id.to_string(),
            command_id: format!("command:{event_id}"),
            world_id: WorldId("abutown-main".to_string()),
            tick: version,
            version,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 3,
            kind: TileKindDto::Road,
        })
    }

    #[test]
    fn sql_record_extracts_metadata_and_json_payload() {
        let event = tile_event("event:9", 9);
        let record = SqlWorldEventRecord::from_event(&event).unwrap();

        assert_eq!(record.metadata, WorldEventMetadata::from_event(&event));
        assert_eq!(record.payload["type"], "tile_kind_set");
        assert_eq!(record.payload["event_id"], "event:9");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::{tests::tile_event, *};
    use sim_core::events::WorldEventStore;

    #[tokio::test]
    async fn postgres_store_appends_event_when_database_url_is_set() {
        let Ok(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
            eprintln!("skipping postgres integration test: ABUTOWN_TEST_DATABASE_URL is not set");
            return;
        };

        let mut store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store");
        let event = tile_event(&format!("event:test:{}", uuid::Uuid::now_v7()), 1);

        store.append(event).await.expect("append event");
    }
}
