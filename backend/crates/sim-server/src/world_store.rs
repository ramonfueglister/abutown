//! Postgres-Store für den [`WorldCoreSnapshot`] (Task 11): eine Zeile pro
//! `world_id` in `world_core_snapshots`, Upsert-Semantik. Payload ist JSONB;
//! beim Lesen geht der rohe Wert durch `world_core::persist::migrate_snapshot`
//! — die Migrationskette ist der EINZIGE Deserialisierungs-Pfad (No-Wipe-
//! Prinzip: Schema-Änderungen migrieren, nie `DELETE FROM`).

use sqlx::PgPool;
use world_core::persist::{MigrateError, WORLD_SNAPSHOT_VERSION, WorldCoreSnapshot};

const WORLD_CORE_MIGRATION: &str =
    include_str!("../migrations/202607050001_world_core_snapshots.sql");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database unavailable: {0}")]
    Database(#[from] sqlx::Error),
    #[error("snapshot does not encode: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("snapshot migration failed: {0}")]
    Migrate(#[from] MigrateError),
}

pub struct WorldStore {
    pool: PgPool,
}

impl WorldStore {
    /// Führt die Tabellen-Migration aus (idempotentes `CREATE TABLE IF NOT
    /// EXISTS`, include_str!-Muster wie `card_hand.rs`) und liefert den Store.
    pub async fn with_pool(pool: PgPool) -> Result<Self, StoreError> {
        for statement in WORLD_CORE_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self { pool })
    }

    /// Upsert des Snapshots für `world_id` (Single-Writer-Deployment: der
    /// letzte Schreiber gewinnt, `updated_at` dokumentiert wann).
    pub async fn write(
        &self,
        world_id: &str,
        tick: u64,
        snap: &WorldCoreSnapshot,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_value(snap)?;
        sqlx::query(
            "INSERT INTO world_core_snapshots (world_id, tick, schema_version, payload) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (world_id) DO UPDATE SET \
                 tick = EXCLUDED.tick, \
                 schema_version = EXCLUDED.schema_version, \
                 payload = EXCLUDED.payload, \
                 updated_at = now()",
        )
        .bind(world_id)
        .bind(
            i64::try_from(tick)
                .expect("world tick exceeds i64 — impossible within the sun's lifetime"),
        )
        .bind(i32::try_from(WORLD_SNAPSHOT_VERSION).expect("schema version fits i32"))
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Liest den Snapshot für `world_id` (via Migrationskette) — `None`,
    /// wenn noch keine Zeile existiert (frische Welt).
    pub async fn read(&self, world_id: &str) -> Result<Option<WorldCoreSnapshot>, StoreError> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT payload FROM world_core_snapshots WHERE world_id = $1")
                .bind(world_id)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(payload,)| Ok(world_core::persist::migrate_snapshot(payload)?))
            .transpose()
    }
}

/// Boot-Log-Vertrag (der Resume-Beweis, exakt — Muster der bewährten
/// mobility-Zeile aus #97: Resume via Boot-Log verifizieren, nicht via
/// DB-Tick). Verdrahtung in den Boot-Pfad folgt in Task 13.
pub fn log_resume(tick: u64) {
    tracing::info!(tick, "resuming world-core from persisted snapshot");
}

/// Gegenstück für den frischen Boot (keine Snapshot-Zeile vorhanden).
pub fn log_fresh() {
    tracing::info!("seeding fresh world-core state");
}
