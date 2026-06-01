use async_trait::async_trait;
use sim_core::economy::EconomyEvent;
use sim_core::persistence::{EconomyEventStore, EconomyEventStoreError};
use sqlx::{PgPool, postgres::PgPoolOptions};

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
    pub async fn connect(database_url: &str) -> Result<Self, EconomyEventStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| EconomyEventStoreError::unavailable(error.to_string()))?;

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

        let mut store = PostgresEconomyEventStore::connect(&database_url)
            .await
            .unwrap();
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

        // Best-effort cleanup of the test rows.
        let _ = sqlx::query("DELETE FROM economy_events WHERE world_id = $1")
            .bind(&world_id)
            .execute(store.pool_for_test())
            .await;
    }
}
