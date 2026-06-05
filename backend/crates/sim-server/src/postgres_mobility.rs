use async_trait::async_trait;
use serde_json::Value;
use sim_core::ids::AgentId;
use sim_core::mobility::MobilityPersistSnapshot;
use sim_core::mobility::seed::seeded_birth_tick_for_agent_id;
use sim_core::persistence::{
    MobilitySnapshotStore, MobilitySnapshotStoreError, SnapshotCompatibility,
};
use sim_core::time::SimClock;
use sqlx::PgPool;

const MOBILITY_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160002_mobility_snapshots.sql");
const DROP_ROAD_VEHICLE_SNAPSHOTS_MIGRATION: &str =
    include_str!("../migrations/202605160005_drop_road_vehicle_snapshots.sql");
const SNAPSHOT_COMPATIBILITY_MIGRATION: &str =
    include_str!("../migrations/202605280002_mobility_snapshot_base_world_metadata.sql");
const LAST_PROCESSED_MONTH_MIGRATION: &str =
    include_str!("../migrations/202606010001_mobility_snapshot_last_processed_month.sql");
const LEGACY_AGENT_BIRTH_TICK_MIGRATION_NAME: &str =
    "202606010002_mobility_snapshot_agent_birth_tick";

#[derive(Debug)]
pub struct PostgresMobilitySnapshotStore {
    pool: PgPool,
}

impl PostgresMobilitySnapshotStore {
    pub async fn with_pool(pool: PgPool) -> Result<Self, MobilitySnapshotStoreError> {
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
            .chain(LAST_PROCESSED_MONTH_MIGRATION.split(';'))
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| MobilitySnapshotStoreError::unavailable(error.to_string()))?;
        }

        migrate_legacy_agent_birth_ticks(&pool).await?;

        Ok(Self { pool })
    }

    pub fn pool_for_test(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

async fn migrate_legacy_agent_birth_ticks(pool: &PgPool) -> Result<(), MobilitySnapshotStoreError> {
    let rows: Vec<(String, i64, Value)> = sqlx::query_as(
        r#"
        SELECT world_id, tick, payload
        FROM mobility_snapshots
        WHERE jsonb_typeof(payload->'agents') = 'object'
          AND EXISTS (
              SELECT 1
              FROM jsonb_each(payload->'agents') AS agent_entries(agent_id, agent)
              WHERE jsonb_typeof(agent) = 'object'
                AND NOT (agent ? 'birth_tick')
          )
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|error| {
        MobilitySnapshotStoreError::unavailable(format!(
            "{LEGACY_AGENT_BIRTH_TICK_MIGRATION_NAME}: {error}"
        ))
    })?;

    for (world_id, row_tick, mut payload) in rows {
        let snapshot_tick = payload
            .get("tick")
            .and_then(Value::as_u64)
            .or_else(|| u64::try_from(row_tick).ok())
            .ok_or_else(|| {
                MobilitySnapshotStoreError::unavailable(format!(
                    "{LEGACY_AGENT_BIRTH_TICK_MIGRATION_NAME}: negative snapshot tick for {world_id}"
                ))
            })?;

        if !migrate_legacy_agent_birth_ticks_in_payload(&mut payload, snapshot_tick) {
            continue;
        }

        sqlx::query(
            r#"
            UPDATE mobility_snapshots
            SET payload = $2,
                updated_at = now()
            WHERE world_id = $1
            "#,
        )
        .bind(&world_id)
        .bind(payload)
        .execute(pool)
        .await
        .map_err(|error| {
            MobilitySnapshotStoreError::unavailable(format!(
                "{LEGACY_AGENT_BIRTH_TICK_MIGRATION_NAME}: {error}"
            ))
        })?;
    }

    Ok(())
}

fn migrate_legacy_agent_birth_ticks_in_payload(payload: &mut Value, snapshot_tick: u64) -> bool {
    let Some(agents) = payload.get_mut("agents").and_then(Value::as_object_mut) else {
        return false;
    };
    let clock = SimClock::default();
    let mut changed = false;

    for (agent_id, agent) in agents.iter_mut() {
        let Some(agent) = agent.as_object_mut() else {
            continue;
        };
        if agent.contains_key("birth_tick") {
            continue;
        }

        let birth_tick =
            seeded_birth_tick_for_agent_id(&AgentId(agent_id.clone()), snapshot_tick, &clock);
        agent.insert("birth_tick".to_string(), Value::from(birth_tick));
        changed = true;
    }

    changed
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
    use sim_core::ids::AgentId;
    use sim_core::mobility::seed::seeded_birth_tick_for_agent_id;
    use sim_core::time::SimClock;

    #[test]
    fn mobility_snapshot_schema_migrations_do_not_touch_chunk_snapshots() {
        assert!(!SNAPSHOT_COMPATIBILITY_MIGRATION.contains("chunk_snapshots"));
    }

    #[test]
    fn legacy_agent_birth_tick_migration_fills_missing_birth_ticks() {
        let tick = 26_280_u64;
        let missing_id = AgentId("agent:legacy-birth:missing".to_string());
        let existing_id = AgentId("agent:legacy-birth:existing".to_string());
        let mut payload = serde_json::json!({
            "tick": tick,
            "last_processed_month": 2,
            "agents": {
                missing_id.0.clone(): {
                    "id": missing_id.0.clone(),
                    "state": {"AtActivity": {"activity_id": "activity:home"}},
                    "plan": [],
                    "plan_cursor": 0,
                    "walk_speed_per_tick": 1.0,
                    "home_market": 0,
                    "work_market": 0
                },
                existing_id.0.clone(): {
                    "id": existing_id.0.clone(),
                    "state": {"AtActivity": {"activity_id": "activity:home"}},
                    "plan": [],
                    "plan_cursor": 0,
                    "walk_speed_per_tick": 1.0,
                    "birth_tick": 7,
                    "home_market": 0,
                    "work_market": 0
                }
            },
            "vehicles": {},
            "stops": {},
            "routes": {},
            "link_polylines": {},
            "flow_cells": [],
            "chunk_activities": []
        });
        let expected = seeded_birth_tick_for_agent_id(&missing_id, tick, &SimClock::default());

        assert!(migrate_legacy_agent_birth_ticks_in_payload(
            &mut payload,
            tick
        ));

        assert_eq!(
            payload["agents"][&missing_id.0]["birth_tick"],
            serde_json::json!(expected)
        );
        assert_eq!(
            payload["agents"][&existing_id.0]["birth_tick"],
            serde_json::json!(7)
        );

        let restored: MobilityPersistSnapshot =
            serde_json::from_value(payload).expect("migrated payload deserializes strictly");
        assert_eq!(
            restored
                .agents
                .get(&missing_id)
                .expect("agent exists")
                .birth_tick,
            expected
        );
    }

    #[tokio::test]
    async fn postgres_mobility_store_round_trip_when_database_url_is_set() {
        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let pool = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool");
        let mut store = PostgresMobilitySnapshotStore::with_pool(pool)
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
            .execute(store.pool_for_test())
            .await;
    }

    #[tokio::test]
    async fn postgres_mobility_connect_migrates_legacy_missing_month_cursor() {
        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let pool = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool");
        let bootstrap = PostgresMobilitySnapshotStore::with_pool(pool)
            .await
            .expect("with_pool mobility store for bootstrap");
        let world_id = format!("test:mobility:legacy-month:{}", uuid::Uuid::now_v7());
        let compatibility = SnapshotCompatibility::new("abutopia", 1);
        let tick = 26_280_i64;
        let legacy_payload = serde_json::json!({
            "tick": tick,
            "agents": {},
            "vehicles": {},
            "stops": {},
            "routes": {},
            "link_polylines": {}
        });

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
            "#,
        )
        .bind(&world_id)
        .bind(tick)
        .bind(&compatibility.base_world_id)
        .bind(i32::try_from(compatibility.base_world_schema_version).unwrap())
        .bind(legacy_payload)
        .execute(bootstrap.pool_for_test())
        .await
        .expect("insert legacy mobility snapshot");

        let pool2 = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool for migration store");
        let store = PostgresMobilitySnapshotStore::with_pool(pool2)
            .await
            .expect("with_pool mobility store runs migrations");
        let (_tick, restored) = MobilitySnapshotStore::read(&store, &world_id, &compatibility)
            .await
            .expect("legacy snapshot is migrated before strict deserialization")
            .expect("legacy snapshot remains present");

        assert_eq!(restored.last_processed_month, 2);

        let _ = sqlx::query("DELETE FROM mobility_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .execute(store.pool_for_test())
            .await;
    }

    #[tokio::test]
    async fn postgres_mobility_connect_migrates_legacy_missing_agent_birth_tick() {
        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let pool = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool");
        let bootstrap = PostgresMobilitySnapshotStore::with_pool(pool)
            .await
            .expect("with_pool mobility store for bootstrap");
        let world_id = format!("test:mobility:legacy-birth:{}", uuid::Uuid::now_v7());
        let compatibility = SnapshotCompatibility::new("abutopia", 1);
        let tick = 26_280_i64;
        let agent_id = AgentId("agent:legacy-birth:db".to_string());
        let legacy_payload = serde_json::json!({
            "tick": tick,
            "last_processed_month": 2,
            "agents": {
                agent_id.0.clone(): {
                    "id": agent_id.0.clone(),
                    "state": {"AtActivity": {"activity_id": "activity:home"}},
                    "plan": [],
                    "plan_cursor": 0,
                    "walk_speed_per_tick": 1.0
                }
            },
            "vehicles": {},
            "stops": {},
            "routes": {},
            "link_polylines": {},
            "flow_cells": [],
            "chunk_activities": []
        });

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
            "#,
        )
        .bind(&world_id)
        .bind(tick)
        .bind(&compatibility.base_world_id)
        .bind(i32::try_from(compatibility.base_world_schema_version).unwrap())
        .bind(legacy_payload)
        .execute(bootstrap.pool_for_test())
        .await
        .expect("insert legacy mobility snapshot");

        let pool2 = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool for migration store");
        let store = PostgresMobilitySnapshotStore::with_pool(pool2)
            .await
            .expect("with_pool mobility store runs migrations");
        let (_tick, restored) = MobilitySnapshotStore::read(&store, &world_id, &compatibility)
            .await
            .expect("legacy snapshot is migrated before strict deserialization")
            .expect("legacy snapshot remains present");
        let expected = seeded_birth_tick_for_agent_id(
            &agent_id,
            u64::try_from(tick).unwrap(),
            &SimClock::default(),
        );

        assert_eq!(
            restored
                .agents
                .get(&agent_id)
                .expect("agent exists")
                .birth_tick,
            expected
        );

        let _ = sqlx::query("DELETE FROM mobility_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .execute(store.pool_for_test())
            .await;
    }
}
