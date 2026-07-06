//! `sim-server` — THE one Abutown process (M1 Task 13): the winterthur
//! traffic kernel + the world-core citizens/economy sim in one deterministic
//! ECS loop, the `/traffic` + `/live` WS gateways, the card-hand HTTP routes
//! and `/health`, all on ONE port, with the world persisted to Postgres
//! every 5 s and resumed (frozen-time) on boot.
//!
//! # Env
//!  * `LISTEN_PORT` (default `8080`) / `LISTEN_HOST` (default `127.0.0.1`;
//!    `0.0.0.0` in the container).
//!  * `DATABASE_URL` — Postgres for card hands + world snapshots. OPTIONAL:
//!    without it the server runs IN-MEMORY (no persistence, local bearer
//!    auth) with a loud warning — the dev/test mode.
//!  * `SUPABASE_URL` — required only when `DATABASE_URL` is set.
//!  * `ABUTOWN_WORLD_ID` (default `winterthur`) — the snapshot row key.
//!  * `TRAFFICNET_JSON` / `TRIPS_BIN` / `SIMWORLD_JSON` / `ECONOMY_JSON` —
//!    data artefacts (defaults under `data/winterthur/`).
//!  * `TRAFFIC_SEED` (default `0`), `ABUTOWN_TRAFFIC_AT` (dev boot-time
//!    override, `HH:MM` or `YYYY-MM-DDTHH:MM`).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use sim_server::app::{self, WorldHealth};
use sim_server::config::ServerConfig;
use sim_server::db::connect_shared_pool;
use sim_server::world::{PersistTarget, WorldArgs, run_world};
use sim_server::world_store::WorldStore;
use winterthur_traffic::clock::{self, WallClock};
use winterthur_traffic::demand::TripSchedule;
use world_core::econ::EconomySeed;
use world_core::{SeedParams, SimWorld};

/// Der Start-Stadtteil (Plan „Offene Punkte"): Altstadt + Umfeld — Anker
/// (0,0) mit 2.5 km Radius, deterministischer Welt-Seed 42.
const WORLD_SEED_PARAMS: SeedParams = SeedParams {
    center: (0.0, 0.0),
    radius_m: 2_500.0,
    residents_per_40m2: 1.0,
    seed: 42,
};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Resolve the TCP listen address from host + port. Host must be a numeric IP
/// (`SocketAddr` does not resolve hostnames): `127.0.0.1` for dev (loopback only),
/// `0.0.0.0` in a container (all interfaces). Returns an error for a non-IP host
/// rather than silently failing.
fn resolve_listen_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    format!("{host}:{port}")
        .parse()
        .with_context(|| format!("parse listen address {host}:{port}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let _ = dotenvy::dotenv();

    // One-shot admin subcommands (no daemon boot). Unknown commands fail loudly.
    let mut cli_args = std::env::args().skip(1);
    if let Some(cmd) = cli_args.next() {
        return match cmd.as_str() {
            "load-building-attributes" => {
                let path = cli_args
                    .next()
                    .context("usage: sim-server load-building-attributes <path>")?;
                sim_server::building_attributes::load_from_file(&path).await
            }
            other => anyhow::bail!("unknown subcommand {other:?}"),
        };
    }

    let port: u16 = match std::env::var("LISTEN_PORT") {
        Err(_) => 8080,
        Ok(v) => v.parse().context("LISTEN_PORT must be a valid u16")?,
    };
    let host = env_or("LISTEN_HOST", "127.0.0.1");
    let world_id = env_or("ABUTOWN_WORLD_ID", "winterthur");
    let seed: u64 = std::env::var("TRAFFIC_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // ── Data artefacts ───────────────────────────────────────────────────
    let net_path = env_or("TRAFFICNET_JSON", "data/winterthur/trafficnet.json");
    let trips_path = env_or("TRIPS_BIN", "data/winterthur/trips.bin");
    let simworld_path = env_or("SIMWORLD_JSON", "data/winterthur/simworld.json");
    let economy_path = env_or("ECONOMY_JSON", "data/winterthur/economy.json");

    let net_json =
        std::fs::read_to_string(&net_path).map_err(|e| anyhow::anyhow!("read {net_path}: {e}"))?;
    let trips = TripSchedule::load(std::path::Path::new(&trips_path), net_json.as_bytes())
        .map_err(|e| anyhow::anyhow!("load {trips_path}: {e}"))?;
    let simworld_json = std::fs::read_to_string(&simworld_path)
        .map_err(|e| anyhow::anyhow!("read {simworld_path}: {e}"))?;
    let sim_world = Arc::new(
        SimWorld::load(&simworld_json).map_err(|e| anyhow::anyhow!("load {simworld_path}: {e}"))?,
    );
    let economy_json = std::fs::read_to_string(&economy_path)
        .map_err(|e| anyhow::anyhow!("read {economy_path}: {e}"))?;
    let economy = EconomySeed::from_json(&economy_json)
        .map_err(|e| anyhow::anyhow!("parse {economy_path}: {e}"))?;

    let clock = match std::env::var("ABUTOWN_TRAFFIC_AT") {
        Ok(s) => match clock::parse_at(&s).ok_or_else(|| {
            anyhow::anyhow!("ABUTOWN_TRAFFIC_AT {s:?} is not HH:MM or YYYY-MM-DDTHH:MM")
        })? {
            clock::AtOverride::DateTime(date, time) => WallClock::anchored(date, time),
            clock::AtOverride::Time(time) => WallClock::new(chrono::Utc::now(), Some(time)),
        },
        Err(_) => WallClock::new(chrono::Utc::now(), None),
    };

    // ── Persistence + card-hand wiring ───────────────────────────────────
    let health = Arc::new(WorldHealth::default());
    let (extra_router, persist, snapshot) = if std::env::var("DATABASE_URL").is_ok() {
        let config = ServerConfig::from_env().context("load server config")?;
        let pool = connect_shared_pool(&config.database_url)
            .await
            .context("connect postgres")?;
        let router = app::build_app_with_shared_pool(&config, pool.clone(), Arc::clone(&health))
            .await
            .context("build card-hand app")?;
        let store = Arc::new(
            WorldStore::with_pool(pool)
                .await
                .context("migrate world_core_snapshots")?,
        );
        let snapshot = store.read(&world_id).await.context("read world snapshot")?;
        (
            router,
            Some(PersistTarget {
                store,
                world_id: world_id.clone(),
            }),
            snapshot,
        )
    } else {
        tracing::warn!(
            "DATABASE_URL not set — running IN-MEMORY: no world persistence, \
             in-memory card hands, local bearer auth (dev/test mode only)"
        );
        (app::build_app(), None, None)
    };

    let addr = resolve_listen_addr(&host, port)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind sim-server")?;
    tracing::info!(%addr, %world_id, seed, "starting sim-server (one process: traffic + world + /live + card-hand)");

    run_world(WorldArgs {
        net_json,
        trips,
        sim_world,
        economy,
        seed_params: WORLD_SEED_PARAMS,
        seed,
        clock,
        snapshot,
        persist,
        extra_router,
        listener,
        health,
    })
    .await
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    // `Router::rebuild` re-freezes the CH every 5 sim-min; fast_paths WARNs
    // once per deliberate self-loop (~1750 lines per rebuild). Silence just
    // that target; an explicit `RUST_LOG` still wins.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("fast_paths=error".parse().expect("valid directive"));
    let _ = fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::resolve_listen_addr;

    #[test]
    fn defaults_to_loopback() {
        let addr = resolve_listen_addr("127.0.0.1", 8080).expect("valid loopback addr");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8080);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn binds_all_interfaces_when_overridden() {
        let addr = resolve_listen_addr("0.0.0.0", 8080).expect("valid wildcard addr");
        assert!(
            addr.ip().is_unspecified(),
            "0.0.0.0 must be the unspecified (all-interfaces) addr"
        );
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn rejects_non_ip_host() {
        // SocketAddr parsing is numeric-only — a hostname like "localhost" must
        // error, not silently bind nowhere.
        assert!(resolve_listen_addr("not-an-ip", 8080).is_err());
    }
}
