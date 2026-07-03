//! `winterthur-traffic` server binary: the headless bevy_ecs traffic sim
//! ([`shell`](winterthur_traffic::shell)) driven by a 10 Hz tokio loop, with a
//! responsive `/healthz` endpoint on a separate task.
//!
//! # Timing (per the #91 outage lesson)
//!
//! The tick loop (in [`shell::run_loop`]) uses `tokio::time::interval(100 ms)`
//! with `MissedTickBehavior::Delay` so a slow tick can never cause a burst of
//! catch-up ticks that starves the HTTP accept loop, and `yield_now().await`
//! after every tick so the runtime stays responsive on a single vCPU. The sim
//! runs at real-time factor 1 (10 ticks/s × 0.1 s dt).
//!
//! # Env
//!  * `TRAFFICNET_JSON` — path to the baked net (default
//!    `data/winterthur/trafficnet.json`). Its sibling `buildings.json` feeds
//!    the spawner's clusters.
//!  * `TRAFFIC_SEED` — u64 sim seed (default `0`).
//!  * `TRAFFIC_PORT` — health endpoint port (default `8790`).
//!
//! The WS gateway (Task 8) will install a real publish hook via
//! [`SnapshotHook`](winterthur_traffic::shell::SnapshotHook); this binary
//! leaves the default no-op seam in place.

use winterthur_traffic::shell;

/// Default health endpoint port.
const DEFAULT_PORT: u16 = 8790;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let net_path = std::env::var("TRAFFICNET_JSON")
        .unwrap_or_else(|_| "data/winterthur/trafficnet.json".to_string());
    let seed: u64 = std::env::var("TRAFFIC_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let port: u16 = std::env::var("TRAFFIC_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let json =
        std::fs::read_to_string(&net_path).map_err(|e| anyhow::anyhow!("read {net_path}: {e}"))?;
    let net =
        traffic_net::load(&json).map_err(|e| anyhow::anyhow!("validate {net_path}: {e:?}"))?;

    tracing::info!(
        edges = net.edges.len(),
        lanes = net.lanes.len(),
        seed,
        port,
        "winterthur-traffic booting"
    );

    let (world, schedule) = shell::build_sim(net, seed);
    tracing::info!(%port, "healthz listening; entering tick loop");
    shell::run_loop(world, schedule, port).await?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
