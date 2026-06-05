use abutown_protocol::WorldEventDto;
use async_trait::async_trait;
use serde_json::Value;
use sim_core::events::{WorldEventMetadata, WorldEventStore, WorldEventStoreError};
use sqlx::PgPool;

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
    pub async fn with_pool(pool: PgPool) -> Result<Self, WorldEventStoreError> {
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

        Ok(Self { pool })
    }

    pub fn pool_for_test(&self) -> &sqlx::PgPool {
        &self.pool
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

        let (chunk_x, chunk_y, chunk_version) = match &event {
            WorldEventDto::TileKindSet(payload) => {
                (payload.coord.x, payload.coord.y, payload.version)
            }
        };
        let chunk_version_i64 = i64::try_from(chunk_version)
            .map_err(|_| WorldEventStoreError::unavailable("chunk_version exceeds i64"))?;

        let rows_affected = sqlx::query(
            r#"
            INSERT INTO world_events (
                event_id,
                world_id,
                command_id,
                event_type,
                tick,
                version,
                chunk_x,
                chunk_y,
                chunk_version,
                payload
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (world_id, command_id) DO NOTHING
            "#,
        )
        .bind(&record.metadata.event_id)
        .bind(&record.metadata.world_id)
        .bind(&record.metadata.command_id)
        .bind(record.metadata.event_type)
        .bind(tick)
        .bind(version)
        .bind(chunk_x)
        .bind(chunk_y)
        .bind(chunk_version_i64)
        .bind(record.payload)
        .execute(&self.pool)
        .await
        .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(WorldEventStoreError::duplicate_command(
                &record.metadata.command_id,
            ));
        }

        Ok(())
    }

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        let row: Option<(Value,)> = sqlx::query_as(
            r#"
            SELECT payload
              FROM world_events
             WHERE world_id = $1 AND command_id = $2
             LIMIT 1
            "#,
        )
        .bind(world_id)
        .bind(command_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;

        match row {
            None => Ok(None),
            Some((payload,)) => {
                let event: WorldEventDto = serde_json::from_value(payload)
                    .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
                Ok(Some(event))
            }
        }
    }
    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        let after = i64::try_from(after_chunk_version)
            .map_err(|_| WorldEventStoreError::unavailable("after_chunk_version exceeds i64"))?;

        let rows: Vec<(Value,)> = sqlx::query_as(
            r#"
            SELECT payload
              FROM world_events
             WHERE world_id = $1
               AND chunk_x = $2
               AND chunk_y = $3
               AND chunk_version > $4
             ORDER BY chunk_version ASC
            "#,
        )
        .bind(world_id)
        .bind(coord.x)
        .bind(coord.y)
        .bind(after)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;

        rows.into_iter()
            .map(|(payload,)| {
                serde_json::from_value::<WorldEventDto>(payload)
                    .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))
            })
            .collect()
    }
    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT MAX(tick) FROM world_events WHERE world_id = $1")
                .bind(world_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        Ok(row.and_then(|(opt,)| opt).map(|v| v as u64))
    }
    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT MAX(version) FROM world_events WHERE world_id = $1")
                .bind(world_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        Ok(row.and_then(|(opt,)| opt).map(|v| v as u64))
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
            world_id: WorldId("abutopia".to_string()),
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

        let pool = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool");
        let mut store = PostgresWorldEventStore::with_pool(pool)
            .await
            .expect("with_pool postgres event store");
        let event = tile_event(&format!("event:{}", uuid::Uuid::now_v7()), 1);

        store.append(event).await.expect("append event");
    }
}
