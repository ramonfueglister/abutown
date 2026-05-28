use async_trait::async_trait;
use serde_json::Value;
use sim_core::mobility::MobilityPersistSnapshot;
use sim_core::persistence::{
    MobilitySnapshotStore, MobilitySnapshotStoreError, SnapshotCompatibility,
};
use sqlx::{PgPool, postgres::PgPoolOptions};

const MOBILITY_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160002_mobility_snapshots.sql");
const DROP_ROAD_VEHICLE_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160005_drop_road_vehicle_snapshots.sql");
const SNAPSHOT_COMPATIBILITY_MIGRATION: &str =
    include_str!("../migrations/202605280001_snapshot_base_world_metadata.sql");

#[derive(Debug)]
pub struct PostgresMobilitySnapshotStore {
    pool: PgPool,
}

impl PostgresMobilitySnapshotStore {
    pub async fn connect(database_url: &str) -> Result<Self, MobilitySnapshotStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        for statement in MOBILITY_SNAPSHOTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;
        }

        for statement in DROP_ROAD_VEHICLE_SNAPSHOTS_MIGRATION
            .split(';')
            .chain(SNAPSHOT_COMPATIBILITY_MIGRATION.split(';'))
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }

    pub fn pool_for_test(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

#[async_trait]
impl MobilitySnapshotStore for PostgresMobilitySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), MobilitySnapshotStoreError> {
        let tick_i64 = i64::try_from(tick)
            .map_err(|_| MobilitySnapshotStoreError::unavailable("tick exceeds i64"))?;
        let schema_version =
            i32::try_from(compatibility.base_world_schema_version).map_err(|_| {
                MobilitySnapshotStoreError::unavailable("base world schema version exceeds i32")
            })?;
        let payload: Value = serde_json::to_value(snapshot)
            .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO mobility_snapshots (
                world_id,
                tick,
                base_world_id,
                base_world_schema_version,
                payload
            )
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (world_id) DO UPDATE
              SET tick = EXCLUDED.tick,
                  base_world_id = EXCLUDED.base_world_id,
                  base_world_schema_version = EXCLUDED.base_world_schema_version,
                  payload = EXCLUDED.payload,
                  updated_at = now()
            "#,
        )
        .bind(world_id)
        .bind(tick_i64)
        .bind(&compatibility.base_world_id)
        .bind(schema_version)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, MobilityPersistSnapshot)>, MobilitySnapshotStoreError> {
        let schema_version =
            i32::try_from(compatibility.base_world_schema_version).map_err(|_| {
                MobilitySnapshotStoreError::unavailable("base world schema version exceeds i32")
            })?;
        let row: Option<(i64, Value)> = sqlx::query_as(
            r#"
                SELECT tick, payload
                FROM mobility_snapshots
                WHERE world_id = $1
                  AND base_world_id = $2
                  AND base_world_schema_version = $3
                "#,
        )
        .bind(world_id)
        .bind(&compatibility.base_world_id)
        .bind(schema_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;

        match row {
            None => Ok(None),
            Some((tick, payload)) => {
                let world: MobilityPersistSnapshot = serde_json::from_value(payload)
                    .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;
                let tick = u64::try_from(tick)
                    .map_err(|_| MobilitySnapshotStoreError::unavailable("negative tick in row"))?;
                Ok(Some((tick, world)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_mobility_store_round_trip_when_database_url_is_set() {
        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let mut store = PostgresMobilitySnapshotStore::connect(&database_url)
            .await
            .unwrap();
        let snap = crate::runtime::SimulationRuntime::new().mobility_persist_snapshot();
        let world_id = format!("test:mobility:{}", uuid::Uuid::now_v7());
        let compatibility = SnapshotCompatibility::new(&world_id, 1);

        store
            .write(&world_id, 7, &snap, &compatibility)
            .await
            .unwrap();
        let (tick, restored) = store
            .read(&world_id, &compatibility)
            .await
            .unwrap()
            .expect("snapshot exists");

        assert_eq!(tick, 7);
        assert_eq!(restored, snap);

        // Best-effort cleanup of the test row
        let _ = sqlx::query("DELETE FROM mobility_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .execute(&store.pool)
            .await;
    }
}
