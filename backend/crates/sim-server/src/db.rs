//! Single shared Postgres pool for all stores. sqlx Pool is Arc-internal, so one
//! pool cloned into each store shares the same bounded connection set. Tuned for the
//! Supabase pooler: bounded, self-reclaiming (idle/lifetime), and prepared-statement
//! caching disabled so it is correct on the TRANSACTION pooler (:6543) as well as session.
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use std::str::FromStr;
use std::time::Duration;

/// Default pool ceiling; override with `ABUTOWN_DB_MAX_CONNECTIONS`. Sized well under
/// the Supabase pooler client limit so all stores together never exhaust it.
const DEFAULT_MAX_CONNECTIONS: u32 = 8;

pub async fn connect_shared_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let max_connections = std::env::var("ABUTOWN_DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_MAX_CONNECTIONS);
    // statement_cache_capacity(0): REQUIRED on the Supabase transaction pooler
    // (multiplexed backends cannot reuse prepared statements); harmless on session mode.
    let connect_options = PgConnectOptions::from_str(database_url)?.statement_cache_capacity(0);
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Some(Duration::from_secs(30)))
        .max_lifetime(Some(Duration::from_secs(900)))
        .test_before_acquire(true)
        .connect_with(connect_options)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_pool_connects_and_pings() {
        let Ok(url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
            return;
        };
        let pool = connect_shared_pool(&url).await.expect("connect");
        let one: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("ping");
        assert_eq!(one, 1);
        // cloning shares the SAME pool (size invariant): both handles see one pool.
        let clone = pool.clone();
        assert_eq!(pool.size(), clone.size());
    }
}
