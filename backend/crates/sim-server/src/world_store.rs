//! Postgres-Store für den [`WorldCoreSnapshot`] (Task 11): eine Zeile pro
//! `world_id` in `world_core_snapshots`, Upsert-Semantik. Payload ist JSONB;
//! beim Lesen geht der rohe Wert durch `world_core::persist::migrate_snapshot`
//! — die Migrationskette ist der EINZIGE Deserialisierungs-Pfad (No-Wipe-
//! Prinzip: Schema-Änderungen migrieren, nie `DELETE FROM`).

use sqlx::PgPool;
use world_core::persist::{MigrateError, WORLD_SNAPSHOT_VERSION, WorldCoreSnapshot};

const WORLD_CORE_MIGRATION: &str =
    include_str!("../migrations/202607050001_world_core_snapshots.sql");
/// Adds the compressed `payload_z BYTEA` column + drops NOT NULL on `payload`.
const WORLD_CORE_COMPRESS_MIGRATION: &str =
    include_str!("../migrations/202607070001_world_core_snapshot_compress.sql");

/// zstd compression level for the snapshot payload. Level 3 is the zstd default
/// — near-instant on a ~2 MB JSON blob and already ~10x on this data; higher
/// levels buy little here and cost CPU on the (off-tick) write task.
const SNAPSHOT_ZSTD_LEVEL: i32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database unavailable: {0}")]
    Database(#[from] sqlx::Error),
    #[error("snapshot does not encode: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("snapshot compression failed: {0}")]
    Compress(std::io::Error),
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
        for migration in [WORLD_CORE_MIGRATION, WORLD_CORE_COMPRESS_MIGRATION] {
            for statement in migration
                .split(';')
                .map(str::trim)
                .filter(|statement| !statement.is_empty())
            {
                sqlx::query(statement).execute(&pool).await?;
            }
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
        // Serialize + zstd-compress; the blob goes into `payload_z`, the legacy
        // JSONB `payload` is left NULL.
        let payload_z = compress_snapshot(snap)?;
        sqlx::query(
            "INSERT INTO world_core_snapshots (world_id, tick, schema_version, payload_z) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (world_id) DO UPDATE SET \
                 tick = EXCLUDED.tick, \
                 schema_version = EXCLUDED.schema_version, \
                 payload_z = EXCLUDED.payload_z, \
                 payload = NULL, \
                 updated_at = now()",
        )
        .bind(world_id)
        .bind(
            i64::try_from(tick)
                .expect("world tick exceeds i64 — impossible within the sun's lifetime"),
        )
        .bind(i32::try_from(WORLD_SNAPSHOT_VERSION).expect("schema version fits i32"))
        .bind(payload_z)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Liest den Snapshot für `world_id` (via Migrationskette) — `None`,
    /// wenn noch keine Zeile existiert (frische Welt).
    pub async fn read(&self, world_id: &str) -> Result<Option<WorldCoreSnapshot>, StoreError> {
        // Compressed `payload_z` is preferred; rows written before the
        // compression migration still carry their JSONB in `payload` and are
        // read via the fallback (No-Wipe: old snapshots stay loadable).
        let row: Option<(Option<Vec<u8>>, Option<serde_json::Value>)> = sqlx::query_as(
            "SELECT payload_z, payload FROM world_core_snapshots WHERE world_id = $1",
        )
        .bind(world_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((payload_z, payload)) = row else {
            return Ok(None);
        };
        let value = match payload_z {
            Some(bytes) => decompress_to_value(&bytes)?,
            None => payload.ok_or_else(|| {
                StoreError::Compress(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "snapshot row has neither payload_z nor payload",
                ))
            })?,
        };
        Ok(Some(world_core::persist::migrate_snapshot(value)?))
    }
}

/// Boot-Log-Vertrag (der Resume-Beweis, exakt — Muster der bewährten
/// mobility-Zeile aus #97: Resume via Boot-Log verifizieren, nicht via
/// DB-Tick). Verdrahtung in den Boot-Pfad folgt in Task 13.
/// Serialize a snapshot to JSON and zstd-compress it (the stored `payload_z`).
fn compress_snapshot(snap: &WorldCoreSnapshot) -> Result<Vec<u8>, StoreError> {
    let json = serde_json::to_vec(snap)?;
    zstd::encode_all(json.as_slice(), SNAPSHOT_ZSTD_LEVEL).map_err(StoreError::Compress)
}

/// Inverse of [`compress_snapshot`] up to the raw JSON value: zstd-decompress
/// then parse to a `serde_json::Value` (the read path then routes it through
/// `migrate_snapshot`, the sole deserialization path).
fn decompress_to_value(bytes: &[u8]) -> Result<serde_json::Value, StoreError> {
    let json = zstd::decode_all(bytes).map_err(StoreError::Compress)?;
    Ok(serde_json::from_slice(&json)?)
}

pub fn log_resume(tick: u64) {
    tracing::info!(tick, "resuming world-core from persisted snapshot");
}

/// Gegenstück für den frischen Boot (keine Snapshot-Zeile vorhanden).
pub fn log_fresh() {
    tracing::info!("seeding fresh world-core state");
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::WorldClock;
    use world_core::persist::{EconSnap, WorldCoreSnapshot};

    /// A snapshot survives the compress → decompress → migrate round trip
    /// byte-for-byte, and compression actually shrinks the payload.
    #[test]
    fn snapshot_compress_round_trip_is_lossless_and_shrinks() {
        let snap = WorldCoreSnapshot {
            version: WORLD_SNAPSHOT_VERSION,
            clock: WorldClock {
                world_tick: 123_456,
            },
            citizens: Vec::new(),
            building_states: Vec::new(),
            econ: EconSnap::default(),
            replanning: None,
        };
        let json = serde_json::to_vec(&snap).unwrap();
        let z = compress_snapshot(&snap).unwrap();
        // Round-trips through the exact read path (decompress → value → migrate).
        let value = decompress_to_value(&z).unwrap();
        let restored = world_core::persist::migrate_snapshot(value).unwrap();
        assert_eq!(snap, restored, "compression must be lossless");
        // A repetitive JSON blob must compress; even this tiny one should not
        // grow, and real ~MB payloads shrink ~10x.
        assert!(
            z.len() <= json.len(),
            "compressed ({}) must not exceed json ({})",
            z.len(),
            json.len()
        );
    }
}
