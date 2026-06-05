use abutown_protocol::{ChunkSnapshotDto, WorldId};
use async_trait::async_trait;
use serde_json::Value;
use sim_core::{
    ids::ChunkCoord,
    persistence::{ChunkSnapshotStore, ChunkSnapshotStoreError, SnapshotCompatibility},
};
use sqlx::{PgPool, Row};

const CHUNK_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605150003_chunk_snapshots.sql");
const SNAPSHOT_COMPATIBILITY_MIGRATION: &str =
    include_str!("../migrations/202605280001_chunk_snapshot_base_world_metadata.sql");

#[derive(Debug, Clone, PartialEq)]
pub struct SqlChunkSnapshotRecord {
    pub world_id: String,
    pub coord: ChunkCoord,
    pub chunk_state: String,
    pub chunk_version: i64,
    pub tile_count: i32,
    pub payload: Value,
}

impl SqlChunkSnapshotRecord {
    pub fn from_snapshot(snapshot: &ChunkSnapshotDto) -> Result<Self, ChunkSnapshotStoreError> {
        let payload = serde_json::to_value(snapshot)
            .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))?;
        let chunk_state = match serde_json::to_value(snapshot.chunk_state)
            .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))?
        {
            Value::String(value) => value,
            _ => {
                return Err(ChunkSnapshotStoreError::unavailable(
                    "chunk state did not serialize to a string",
                ));
            }
        };
        let chunk_version = i64::try_from(snapshot.chunk_version)
            .map_err(|_| ChunkSnapshotStoreError::unavailable("chunk version exceeds i64"))?;

        Ok(Self {
            world_id: snapshot.world_id.0.clone(),
            coord: ChunkCoord {
                x: snapshot.coord.x,
                y: snapshot.coord.y,
            },
            chunk_state,
            chunk_version,
            tile_count: i32::from(snapshot.tile_count),
            payload,
        })
    }

    pub fn into_snapshot(self) -> Result<ChunkSnapshotDto, ChunkSnapshotStoreError> {
        serde_json::from_value(self.payload)
            .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))
    }
}

#[derive(Debug)]
pub struct PostgresChunkSnapshotStore {
    pool: PgPool,
    world_id: WorldId,
}

impl PostgresChunkSnapshotStore {
    pub async fn with_pool(
        pool: PgPool,
        world_id: WorldId,
        _compatibility: SnapshotCompatibility,
    ) -> Result<Self, ChunkSnapshotStoreError> {
        for statement in CHUNK_SNAPSHOTS_MIGRATION
            .split(';')
            .chain(SNAPSHOT_COMPATIBILITY_MIGRATION.split(';'))
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool, world_id })
    }

    pub fn pool_for_test(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

#[async_trait]
impl ChunkSnapshotStore for PostgresChunkSnapshotStore {
    async fn write_snapshot(
        &mut self,
        snapshot: ChunkSnapshotDto,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), ChunkSnapshotStoreError> {
        let record = SqlChunkSnapshotRecord::from_snapshot(&snapshot)?;

        sqlx::query(
            r#"
            INSERT INTO chunk_snapshots (
                world_id,
                chunk_x,
                chunk_y,
                chunk_state,
                chunk_version,
                tile_count,
                base_world_id,
                base_world_schema_version,
                payload
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (world_id, chunk_x, chunk_y)
            DO UPDATE SET
                chunk_state = EXCLUDED.chunk_state,
                chunk_version = EXCLUDED.chunk_version,
                tile_count = EXCLUDED.tile_count,
                base_world_id = EXCLUDED.base_world_id,
                base_world_schema_version = EXCLUDED.base_world_schema_version,
                payload = EXCLUDED.payload,
                updated_at = now()
            "#,
        )
        .bind(&record.world_id)
        .bind(record.coord.x)
        .bind(record.coord.y)
        .bind(record.chunk_state)
        .bind(record.chunk_version)
        .bind(record.tile_count)
        .bind(&compatibility.base_world_id)
        .bind(
            i32::try_from(compatibility.base_world_schema_version).map_err(|_| {
                ChunkSnapshotStoreError::unavailable("base world schema version exceeds i32")
            })?,
        )
        .bind(record.payload)
        .execute(&self.pool)
        .await
        .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn read_snapshot(
        &self,
        coord: ChunkCoord,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        let row = sqlx::query(
            r#"
            SELECT payload
            FROM chunk_snapshots
            WHERE world_id = $1
              AND chunk_x = $2
              AND chunk_y = $3
              AND base_world_id = $4
              AND base_world_schema_version = $5
            "#,
        )
        .bind(&self.world_id.0)
        .bind(coord.x)
        .bind(coord.y)
        .bind(&compatibility.base_world_id)
        .bind(
            i32::try_from(compatibility.base_world_schema_version).map_err(|_| {
                ChunkSnapshotStoreError::unavailable("base world schema version exceeds i32")
            })?,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))?;

        let Some(row) = row else {
            return Ok(None);
        };
        let payload = row
            .try_get::<Value, _>("payload")
            .map_err(|error| ChunkSnapshotStoreError::unavailable(error.to_string()))?;
        SqlChunkSnapshotRecord {
            world_id: self.world_id.0.clone(),
            coord,
            chunk_state: String::new(),
            chunk_version: 0,
            tile_count: 0,
            payload,
        }
        .into_snapshot()
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abutown_protocol::{
        ChunkCoordDto, ChunkStateDto, PROTOCOL_VERSION, TileKindDto, TileMutationDto,
    };

    pub(crate) fn snapshot(world_id: WorldId) -> ChunkSnapshotDto {
        ChunkSnapshotDto {
            protocol_version: PROTOCOL_VERSION,
            world_id,
            coord: ChunkCoordDto { x: 4, y: 4 },
            chunk_state: ChunkStateDto::Active,
            chunk_version: 7,
            tile_count: 1024,
            tiles: vec![TileMutationDto {
                local_index: 3,
                kind: TileKindDto::Road,
                version: 7,
            }],
        }
    }

    #[test]
    fn sql_record_extracts_snapshot_metadata_and_json_payload() {
        let snapshot = snapshot(WorldId("abutopia".to_string()));
        let record = SqlChunkSnapshotRecord::from_snapshot(&snapshot).unwrap();

        assert_eq!(record.world_id, "abutopia");
        assert_eq!(record.coord, ChunkCoord { x: 4, y: 4 });
        assert_eq!(record.chunk_state, "active");
        assert_eq!(record.chunk_version, 7);
        assert_eq!(record.tile_count, 1024);
        assert_eq!(record.payload["world_id"], "abutopia");
        assert_eq!(record.payload["coord"]["x"], 4);
        assert_eq!(record.payload["chunk_state"], "active");
    }

    #[test]
    fn chunk_snapshot_schema_migrations_do_not_touch_mobility_snapshots() {
        assert!(!SNAPSHOT_COMPATIBILITY_MIGRATION.contains("mobility_snapshots"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::{tests::snapshot, *};

    #[tokio::test]
    async fn postgres_store_writes_and_reads_snapshot_when_database_url_is_set() {
        let Ok(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
            eprintln!("skipping postgres integration test: ABUTOWN_TEST_DATABASE_URL is not set");
            return;
        };
        let world_id = WorldId(format!("test:{}", uuid::Uuid::now_v7()));
        let compatibility = SnapshotCompatibility::new(world_id.0.clone(), 1);
        let pool = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool");
        let mut store =
            PostgresChunkSnapshotStore::with_pool(pool, world_id.clone(), compatibility.clone())
                .await
                .expect("with_pool postgres snapshot store");
        let snapshot = snapshot(world_id);
        let coord = ChunkCoord { x: 4, y: 4 };

        ChunkSnapshotStore::write_snapshot(&mut store, snapshot.clone(), &compatibility)
            .await
            .expect("write snapshot");

        let stored = ChunkSnapshotStore::read_snapshot(&store, coord, &compatibility)
            .await
            .expect("read snapshot")
            .expect("snapshot exists");
        assert_eq!(stored, snapshot);
    }
}
