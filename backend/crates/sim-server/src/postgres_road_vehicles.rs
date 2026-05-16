use async_trait::async_trait;
use serde_json::Value;
use sim_core::persistence::{RoadVehicleSnapshotStore, RoadVehicleSnapshotStoreError};
use sim_core::road_vehicles::RoadVehicleWorld;
use sqlx::{PgPool, postgres::PgPoolOptions};

const ROAD_VEHICLE_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160003_road_vehicle_snapshots.sql");

#[derive(Debug)]
pub struct PostgresRoadVehicleSnapshotStore {
    pool: PgPool,
}

impl PostgresRoadVehicleSnapshotStore {
    pub async fn connect(database_url: &str) -> Result<Self, RoadVehicleSnapshotStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        for statement in ROAD_VEHICLE_SNAPSHOTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }

    pub fn pool_for_test(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl RoadVehicleSnapshotStore for PostgresRoadVehicleSnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &RoadVehicleWorld,
    ) -> Result<(), RoadVehicleSnapshotStoreError> {
        let tick_i64 = i64::try_from(tick)
            .map_err(|_| RoadVehicleSnapshotStoreError::unavailable("tick exceeds i64"))?;
        let payload: Value = serde_json::to_value(snapshot)
            .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO road_vehicle_snapshots (world_id, tick, payload)
            VALUES ($1, $2, $3)
            ON CONFLICT (world_id) DO UPDATE
              SET tick = EXCLUDED.tick,
                  payload = EXCLUDED.payload,
                  updated_at = now()
            "#,
        )
        .bind(world_id)
        .bind(tick_i64)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, RoadVehicleWorld)>, RoadVehicleSnapshotStoreError> {
        let row: Option<(i64, Value)> = sqlx::query_as(
            "SELECT tick, payload FROM road_vehicle_snapshots WHERE world_id = $1",
        )
        .bind(world_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| RoadVehicleSnapshotStoreError::unavailable(error.to_string()))?;

        match row {
            None => Ok(None),
            Some((tick, payload)) => {
                let world: RoadVehicleWorld = serde_json::from_value(payload).map_err(|error| {
                    RoadVehicleSnapshotStoreError::unavailable(error.to_string())
                })?;
                let tick = u64::try_from(tick).map_err(|_| {
                    RoadVehicleSnapshotStoreError::unavailable("negative tick in row")
                })?;
                Ok(Some((tick, world)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_road_vehicle_round_trip_when_database_url_is_set() {
        use sim_core::road_vehicles::seed;

        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let mut store = PostgresRoadVehicleSnapshotStore::connect(&database_url).await.unwrap();
        let world = seed::initial_road_vehicles();
        let world_id = format!("test:road_vehicle:{}", uuid::Uuid::now_v7());

        store.write(&world_id, world.tick(), &world).await.unwrap();
        let (tick, restored) = store.read(&world_id).await.unwrap().expect("present");
        assert_eq!(tick, world.tick());
        assert_eq!(restored, world);

        let _ = sqlx::query("DELETE FROM road_vehicle_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .execute(store.pool_for_test())
            .await;
    }
}
