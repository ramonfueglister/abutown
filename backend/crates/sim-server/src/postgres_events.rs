use abutown_protocol::WorldEventDto;
use async_trait::async_trait;
use serde_json::Value;
use sim_core::events::{WorldEventMetadata, WorldEventStore, WorldEventStoreError};
use sqlx::{PgPool, postgres::PgPoolOptions};

const WORLD_EVENTS_MIGRATION: &str = include_str!("../migrations/202605150001_world_events.sql");
const CHUNK_RECOVERY_MIGRATION: &str =
    include_str!("../migrations/202605160001_chunk_recovery.sql");

#[derive(Debug, Clone, PartialEq)]
pub struct SqlWorldEventRecord {
    pub metadata: WorldEventMetadata,
    pub payload: Value,
}

impl SqlWorldEventRecord {
    pub fn from_event(event: &WorldEventDto) -> Result<Self, WorldEventStoreError> {
        match event {}
    }
}

#[derive(Debug)]
pub struct PostgresWorldEventStore {
    _pool: PgPool,
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

        for statement in CHUNK_RECOVERY_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { _pool: pool })
    }
}

#[async_trait]
impl WorldEventStore for PostgresWorldEventStore {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        match event {}
        #[allow(unreachable_code)]
        Ok(())
    }

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        let _ = (world_id, command_id);
        Ok(None)
    }
    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        let _ = (world_id, coord, after_chunk_version);
        Ok(Vec::new())
    }
    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        let _ = world_id;
        Ok(None)
    }
    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        let _ = world_id;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_record_type_remains_available_for_empty_event_transition() {
        let record = SqlWorldEventRecord {
            metadata: WorldEventMetadata {
                event_id: "event:none".to_string(),
                world_id: "abutown-main".to_string(),
                command_id: "command:none".to_string(),
                event_type: "none",
                tick: 0,
                version: 0,
            },
            payload: serde_json::json!({}),
        };

        assert_eq!(record.metadata.event_type, "none");
        assert_eq!(record.payload, serde_json::json!({}));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use sim_core::events::WorldEventStore;

    #[tokio::test]
    async fn postgres_store_reads_empty_event_transition_when_database_url_is_set() {
        let Ok(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
            eprintln!("skipping postgres integration test: ABUTOWN_TEST_DATABASE_URL is not set");
            return;
        };

        let store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store");

        assert_eq!(
            store
                .read_chunk_events_since(
                    "abutown-main",
                    abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    0,
                )
                .await
                .expect("read events"),
            Vec::<WorldEventDto>::new()
        );
    }
}
