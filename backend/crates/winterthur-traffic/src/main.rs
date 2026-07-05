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
//!    `data/winterthur/trafficnet.json`).
//!  * `TRIPS_BIN` — path to the census trip table (default
//!    `data/winterthur/trips.bin`); must have been baked against the exact
//!    `TRAFFICNET_JSON` bytes (net-hash checked, hard error on mismatch).
//!  * `ABUTOWN_TRAFFIC_AT` — dev override `HH:MM` (fixes the boot
//!    time-of-day, Europe/Zurich; real date) or `YYYY-MM-DDTHH:MM` (also
//!    pins the boot date, and with it `day_kind` — required for
//!    reproducible harnesses); malformed = hard error.
//!  * `DEMAND_SCALE` — f32 trip-thinning factor (default `1.0`).
//!  * `TRAFFIC_SEED` — u64 sim seed (default `0`).
//!  * `TRAFFIC_PORT` — health endpoint port (default `8790`).
//!
//! The WS gateway installs a real publish hook via
//! [`SnapshotHook`](winterthur_traffic::shell::SnapshotHook) and serves the
//! `/traffic` WebSocket endpoint on the same [`TRAFFIC_PORT`] as `/healthz`.

use winterthur_traffic::cells::CellGrid;
use winterthur_traffic::clock::{self, WallClock};
use winterthur_traffic::demand::{DayKind, TripSchedule};
use winterthur_traffic::gateway::{self, Registry, make_publisher};
use winterthur_traffic::shell;
use winterthur_traffic::spawner::SpawnerCfg;

/// Default health endpoint port.
const DEFAULT_PORT: u16 = 8790;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let net_path = std::env::var("TRAFFICNET_JSON")
        .unwrap_or_else(|_| "data/winterthur/trafficnet.json".to_string());
    let trips_path =
        std::env::var("TRIPS_BIN").unwrap_or_else(|_| "data/winterthur/trips.bin".to_string());
    let seed: u64 = std::env::var("TRAFFIC_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let port: u16 = std::env::var("TRAFFIC_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let demand_scale: f32 = match std::env::var("DEMAND_SCALE") {
        Ok(s) => s
            .parse()
            .map_err(|e| anyhow::anyhow!("DEMAND_SCALE {s:?} is not an f32: {e}"))?,
        Err(_) => 1.0,
    };
    let override_at = match std::env::var("ABUTOWN_TRAFFIC_AT") {
        Ok(s) => Some(clock::parse_at(&s).ok_or_else(|| {
            anyhow::anyhow!("ABUTOWN_TRAFFIC_AT {s:?} is not HH:MM or YYYY-MM-DDTHH:MM")
        })?),
        Err(_) => None,
    };

    let json =
        std::fs::read_to_string(&net_path).map_err(|e| anyhow::anyhow!("read {net_path}: {e}"))?;
    let net =
        traffic_net::load(&json).map_err(|e| anyhow::anyhow!("validate {net_path}: {e:?}"))?;

    // The census trip table must match the exact net bytes we just loaded.
    let trips = TripSchedule::load(std::path::Path::new(&trips_path), json.as_bytes())
        .map_err(|e| anyhow::anyhow!("load {trips_path}: {e}"))?;
    let clock = match override_at {
        Some(clock::AtOverride::DateTime(date, time)) => WallClock::anchored(date, time),
        Some(clock::AtOverride::Time(time)) => WallClock::new(chrono::Utc::now(), Some(time)),
        None => WallClock::new(chrono::Utc::now(), None),
    };

    tracing::info!(
        edges = net.edges.len(),
        lanes = net.lanes.len(),
        gateways = net.gateways().len(),
        seed,
        port,
        "winterthur-traffic booting"
    );
    // The #97 lesson: log the boot anchor so resume/demo behaviour is
    // verifiable from the boot log alone.
    tracing::info!(
        boot_s_of_day = clock.s_of_day(0),
        day_kind = ?clock.day_kind(0),
        trips_weekday = trips.count(DayKind::Workday),
        trips_weekend = trips.count(DayKind::Weekend),
        demand_scale,
        "traffic demand bound to wall clock"
    );

    // Build the AOI cell grid from the net geometry *before* `build_sim`
    // consumes the net, then install the real publish hook + serve `/traffic`.
    let grid = CellGrid::build(&net);
    let (cols, rows) = grid.dims();
    tracing::info!(cols, rows, cells = cols * rows, "AOI grid built");

    let registry = Registry::new();
    let cell_count = grid.cell_count();
    let (mut world, schedule) =
        shell::build_sim(net, seed, trips, clock, SpawnerCfg { demand_scale });
    world.insert_resource(make_publisher(grid, registry.clone()));

    let extra = gateway::router(registry, cell_count);
    tracing::info!(%port, "healthz + /traffic listening; entering tick loop");
    shell::run_loop_with_router(world, schedule, port, Some(extra)).await?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    // `Router::rebuild` re-freezes the CH every 5 sim-min and re-adds one
    // self-loop per edge to register isolated node ids (see
    // `Router::input_graph_from_arcs`). fast_paths skips each self-loop with a
    // WARN, ~1750 lines per rebuild — a benign flood from a deliberate trick.
    // Silence just that target at ERROR while keeping everything else at the
    // requested level (default `info`). An explicit `RUST_LOG` still wins.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("fast_paths=error".parse().expect("valid directive"));
    let _ = fmt().with_env_filter(filter).try_init();
}
