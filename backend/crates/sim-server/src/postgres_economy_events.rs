use async_trait::async_trait;
use sim_core::economy::EconomyEvent;
use sim_core::persistence::{EconomyEventStore, EconomyEventStoreError};
use sqlx::PgPool;

const ECONOMY_EVENTS_MIGRATION: &str =
    include_str!("../migrations/202605310001_economy_events.sql");

/// Durable, append-only audit log of economy events (observability — NOT a
/// recovery source; `PostgresEconomySnapshotStore` remains the recovery source of
/// truth). The table is indexed on `(world_id, id)` and `(world_id, tick)` so a
/// later query slice can page events and scope them to a tick window.
#[derive(Debug)]
pub struct PostgresEconomyEventStore {
    pool: PgPool,
}

impl PostgresEconomyEventStore {
    pub async fn with_pool(pool: PgPool) -> Result<Self, EconomyEventStoreError> {
        for statement in ECONOMY_EVENTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| EconomyEventStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }

    pub fn pool_for_test(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

#[async_trait]
impl EconomyEventStore for PostgresEconomyEventStore {
    async fn append(
        &mut self,
        world_id: &str,
        tick: u64,
        events: &[EconomyEvent],
    ) -> Result<(), EconomyEventStoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let tick_i64 = i64::try_from(tick)
            .map_err(|_| EconomyEventStoreError::unavailable("tick exceeds i64"))?;

        // One multi-row insert via UNNEST: the whole batch shares (world_id, tick),
        // so only the per-event columns are passed as arrays. Payloads ride as
        // `text[]` and are cast to `jsonb` server-side — `Vec<String>` -> `text[]`
        // is the most portable sqlx array bind.
        let mut event_types: Vec<String> = Vec::with_capacity(events.len());
        let mut payloads: Vec<String> = Vec::with_capacity(events.len());
        for event in events {
            event_types.push(event.event_type().to_string());
            payloads.push(
                serde_json::to_string(event)
                    .map_err(|error| EconomyEventStoreError::unavailable(error.to_string()))?,
            );
        }

        sqlx::query(
            r#"
            INSERT INTO economy_events (world_id, tick, event_type, payload)
            SELECT $1, $2, event_type, payload::jsonb
            FROM UNNEST($3::text[], $4::text[]) AS batch(event_type, payload)
            "#,
        )
        .bind(world_id)
        .bind(tick_i64)
        .bind(&event_types)
        .bind(&payloads)
        .execute(&self.pool)
        .await
        .map_err(|error| EconomyEventStoreError::unavailable(error.to_string()))?;

        Ok(())
    }

    async fn prune(
        &mut self,
        world_id: &str,
        keep_last: u64,
    ) -> Result<u64, EconomyEventStoreError> {
        let keep_last = i64::try_from(keep_last)
            .map_err(|_| EconomyEventStoreError::unavailable("keep_last exceeds i64"))?;
        // Index-friendly rolling window on the existing (world_id, id) index:
        // find the id of the (keep_last+1)-th newest row, delete it and older.
        // Fewer rows than keep_last → subquery yields NULL → COALESCE(-1) →
        // nothing matches (ids are BIGSERIAL, always positive).
        let result = sqlx::query(
            r#"
            DELETE FROM economy_events
            WHERE world_id = $1
              AND id <= COALESCE(
                (
                    SELECT id FROM economy_events
                    WHERE world_id = $1
                    ORDER BY id DESC
                    OFFSET $2 LIMIT 1
                ),
                -1
              )
            "#,
        )
        .bind(world_id)
        .bind(keep_last)
        .execute(&self.pool)
        .await
        .map_err(|error| EconomyEventStoreError::unavailable(error.to_string()))?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::economy::{EconomicActorId, Money};

    /// Opt-in round trip against a real Postgres. Mirrors the snapshot store's
    /// integration test: set `ABUTOWN_TEST_DATABASE_URL` to run it; otherwise it
    /// skips so the default `cargo test` stays hermetic.
    #[tokio::test]
    async fn postgres_economy_event_store_appends_when_database_url_is_set() {
        let Some(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping; ABUTOWN_TEST_DATABASE_URL not set");
            return;
        };

        let pool = crate::db::connect_shared_pool(&database_url)
            .await
            .expect("connect shared pool");
        let mut store = PostgresEconomyEventStore::with_pool(pool).await.unwrap();
        let world_id = format!("test:economy-events:{}", uuid::Uuid::now_v7());

        store
            .append(
                &world_id,
                7,
                &[
                    EconomyEvent::CashLocked {
                        actor: EconomicActorId(1),
                        amount: Money(10),
                    },
                    EconomyEvent::TransportPaid {
                        actor: EconomicActorId(2),
                        amount: Money(5),
                    },
                ],
            )
            .await
            .unwrap();
        // Empty batches are a no-op, never an error.
        store.append(&world_id, 8, &[]).await.unwrap();

        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT tick, event_type FROM economy_events WHERE world_id = $1 ORDER BY id",
        )
        .bind(&world_id)
        .fetch_all(store.pool_for_test())
        .await
        .unwrap();

        assert_eq!(
            rows,
            vec![
                (7, "cash_locked".to_string()),
                (7, "transport_paid".to_string())
            ]
        );

        // Rolling retention: keep only the newest row; the older one is deleted.
        // An under-cap prune afterwards is a no-op.
        let deleted = store.prune(&world_id, 1).await.unwrap();
        assert_eq!(deleted, 1, "prune deletes everything past keep_last");
        let remaining: Vec<(i64, String)> = sqlx::query_as(
            "SELECT tick, event_type FROM economy_events WHERE world_id = $1 ORDER BY id",
        )
        .bind(&world_id)
        .fetch_all(store.pool_for_test())
        .await
        .unwrap();
        assert_eq!(remaining, vec![(7, "transport_paid".to_string())]);
        assert_eq!(store.prune(&world_id, 10).await.unwrap(), 0);

        // Best-effort cleanup of the test rows.
        let _ = sqlx::query("DELETE FROM economy_events WHERE world_id = $1")
            .bind(&world_id)
            .execute(store.pool_for_test())
            .await;
    }
}
