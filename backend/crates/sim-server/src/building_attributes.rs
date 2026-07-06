//! Per-building enrichment store: ÖREB Bauzone (allowed) + GWR category (is).
//! Supabase is the source of truth; `data/winterthur/building-attributes.json`
//! is the deterministic bake artifact that seeds it. Mirrors the CardHandStore
//! shape: Postgres in production, in-memory in the no-DATABASE_URL dev mode.
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use sqlx::PgPool;

const BUILDING_ATTRIBUTES_MIGRATION: &str =
    include_str!("../migrations/202607060001_building_attributes.sql");

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildingAttributes {
    #[serde(rename = "id")]
    pub building_id: String,
    pub egid: Option<i64>,
    pub gwr_category: Option<String>,
    pub gwr_class: Option<String>,
    pub bauzone: Option<String>,
    pub bauzone_code: Option<String>,
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// The bake artifact (data/winterthur/building-attributes.json).
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildingAttributesFile {
    pub world_id: String,
    pub buildings: Vec<BuildingAttributes>,
}

#[derive(Clone)]
pub struct BuildingAttributesStore(Inner);

#[derive(Clone)]
enum Inner {
    Postgres(PgPool),
    Memory(Arc<RwLock<HashMap<String, Vec<BuildingAttributes>>>>),
}

impl BuildingAttributesStore {
    /// Production: runs the migration, then serves reads/writes from Postgres.
    pub async fn with_pool(pool: PgPool) -> Result<Self, sqlx::Error> {
        for statement in BUILDING_ATTRIBUTES_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self(Inner::Postgres(pool)))
    }

    pub fn memory() -> Self {
        Self(Inner::Memory(Arc::new(RwLock::new(HashMap::new()))))
    }

    pub async fn upsert_all(
        &self,
        world_id: &str,
        rows: &[BuildingAttributes],
    ) -> Result<u64, sqlx::Error> {
        match &self.0 {
            Inner::Postgres(pool) => {
                // One round-trip per batch via UNNEST (SOTA bulk upsert, no per-row loop).
                let mut total = 0u64;
                for chunk in rows.chunks(1000) {
                    let (mut ids, mut egids, mut cats, mut classes, mut zones, mut codes, mut raws) =
                        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
                    for r in chunk {
                        ids.push(r.building_id.clone());
                        egids.push(r.egid);
                        cats.push(r.gwr_category.clone());
                        classes.push(r.gwr_class.clone());
                        zones.push(r.bauzone.clone());
                        codes.push(r.bauzone_code.clone());
                        raws.push(r.raw.clone());
                    }
                    let done = sqlx::query(
                        r#"INSERT INTO building_attributes
                             (world_id, building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw, updated_at)
                           SELECT $1, u.building_id, u.egid, u.gwr_category, u.gwr_class, u.bauzone, u.bauzone_code, u.raw, now()
                           FROM UNNEST($2::text[], $3::int8[], $4::text[], $5::text[], $6::text[], $7::text[], $8::jsonb[])
                             AS u(building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw)
                           ON CONFLICT (world_id, building_id) DO UPDATE SET
                             egid = EXCLUDED.egid, gwr_category = EXCLUDED.gwr_category,
                             gwr_class = EXCLUDED.gwr_class, bauzone = EXCLUDED.bauzone,
                             bauzone_code = EXCLUDED.bauzone_code, raw = EXCLUDED.raw,
                             updated_at = now()"#,
                    )
                    .bind(world_id)
                    .bind(&ids)
                    .bind(&egids)
                    .bind(&cats)
                    .bind(&classes)
                    .bind(&zones)
                    .bind(&codes)
                    .bind(&raws)
                    .execute(pool)
                    .await?;
                    total += done.rows_affected();
                }
                Ok(total)
            }
            Inner::Memory(map) => {
                let mut guard = map.write().expect("building_attributes lock poisoned");
                let entry = guard.entry(world_id.to_string()).or_default();
                for r in rows {
                    entry.retain(|e| e.building_id != r.building_id);
                    entry.push(r.clone());
                }
                Ok(rows.len() as u64)
            }
        }
    }

    pub async fn list(&self, world_id: &str) -> Result<Vec<BuildingAttributes>, sqlx::Error> {
        match &self.0 {
            Inner::Postgres(pool) => sqlx::query_as::<
                _,
                (
                    String,
                    Option<i64>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    serde_json::Value,
                ),
            >(
                "SELECT building_id, egid, gwr_category, gwr_class, bauzone, bauzone_code, raw
                     FROM building_attributes WHERE world_id = $1 ORDER BY building_id",
            )
            .bind(world_id)
            .fetch_all(pool)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(
                        |(
                            building_id,
                            egid,
                            gwr_category,
                            gwr_class,
                            bauzone,
                            bauzone_code,
                            raw,
                        )| BuildingAttributes {
                            building_id,
                            egid,
                            gwr_category,
                            gwr_class,
                            bauzone,
                            bauzone_code,
                            raw,
                        },
                    )
                    .collect()
            }),
            Inner::Memory(map) => {
                let guard = map.read().expect("building_attributes lock poisoned");
                let mut rows = guard.get(world_id).cloned().unwrap_or_default();
                rows.sort_by(|a, b| a.building_id.cmp(&b.building_id));
                Ok(rows)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> BuildingAttributes {
        BuildingAttributes {
            building_id: id.to_string(),
            egid: Some(42),
            gwr_category: Some("Gebäude mit ausschliesslicher Wohnnutzung".into()),
            gwr_class: Some("1110".into()),
            bauzone: Some("Wohnzone W3".into()),
            bauzone_code: Some("W3".into()),
            raw: serde_json::json!({"egids": [42]}),
        }
    }

    #[tokio::test]
    async fn memory_upsert_and_list_roundtrip() {
        let store = BuildingAttributesStore::memory();
        store
            .upsert_all("winterthur", &[sample("{B}"), sample("{A}")])
            .await
            .unwrap();
        // idempotent overwrite
        store
            .upsert_all("winterthur", &[sample("{A}")])
            .await
            .unwrap();
        let rows = store.list("winterthur").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].building_id, "{A}"); // sorted
        assert!(store.list("other").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn postgres_upsert_and_list_roundtrip() {
        // Opt-in (same pattern as db.rs): set ABUTOWN_TEST_DATABASE_URL to run.
        let Ok(url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
            return;
        };
        let pool = crate::db::connect_shared_pool(&url).await.expect("connect");
        let store = BuildingAttributesStore::with_pool(pool)
            .await
            .expect("migrate");
        let world = format!("test-{}", std::process::id());
        store
            .upsert_all(&world, &[sample("{A}"), sample("{B}")])
            .await
            .unwrap();
        store.upsert_all(&world, &[sample("{B}")]).await.unwrap(); // upsert path
        let rows = store.list(&world).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].bauzone_code.as_deref(), Some("W3"));
    }
}
