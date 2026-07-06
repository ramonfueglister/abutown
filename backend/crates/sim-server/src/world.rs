//! The one-process world runtime (Task 13): traffic kernel + world-core
//! citizens/economy in ONE deterministic ECS chain, the `/traffic` and
//! `/live` WS gateways plus the card-hand routes on ONE port, and the
//! persist flush that snapshots the world every [`PERSIST_EVERY_N_TICKS`]
//! ticks without ever blocking the tick.
//!
//! # Persist flush shape
//!
//! `world_core::persist::extract` runs SYNCHRONOUSLY in the tick loop (it is
//! a read-only walk over the ECS resources — cheap, and the only way to get
//! a consistent snapshot), then the Postgres write is `tokio::spawn`ed with
//! the extracted value. A single in-flight guard skips a flush while the
//! previous write is still running, so slow writes coalesce instead of
//! stacking up (and out-of-order upserts can never regress the stored tick).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::net::TcpListener;
use winterthur_traffic::ChTripRouter;
use winterthur_traffic::cells::CellGrid;
use winterthur_traffic::clock::WallClock;
use winterthur_traffic::demand::TripSchedule;
use winterthur_traffic::gateway::{self, Registry, make_live_publisher, make_publisher};
use winterthur_traffic::shell::{self, WORLD_BG_DEMAND_SCALE, WorldCoreExt};
use winterthur_traffic::spawner::SpawnerCfg;
use world_core::econ::EconomySeed;
use world_core::persist::WorldCoreSnapshot;
use world_core::{AuditStatus, SeedParams, SimWorld, WorldClock, WorldCorePlugin};

use crate::app::WorldHealth;
use crate::world_store::WorldStore;

/// Snapshot the world every 50 ticks = every 5 s at the 10 Hz tick rate.
pub const PERSIST_EVERY_N_TICKS: u64 = 50;

/// Where snapshots go: the Postgres store plus this deployment's world id.
pub struct PersistTarget {
    pub store: Arc<WorldStore>,
    pub world_id: String,
}

/// Everything the world runtime needs, pre-loaded by the caller (the binary
/// loads from disk/env; tests inject fixtures + an ephemeral listener).
pub struct WorldArgs {
    pub net_json: String,
    pub trips: TripSchedule,
    pub sim_world: Arc<SimWorld>,
    pub economy: EconomySeed,
    pub seed_params: SeedParams,
    pub seed: u64,
    pub clock: WallClock,
    /// A persisted world to resume (`WorldStore::read`) — `None` seeds fresh.
    pub snapshot: Option<WorldCoreSnapshot>,
    /// `None` = in-memory run without persistence (tests / no DATABASE_URL).
    pub persist: Option<PersistTarget>,
    /// The card-hand + `/health` routes, merged onto the same port.
    pub extra_router: axum::Router,
    /// Pre-bound listener (the binary binds host:port; tests bind port 0).
    pub listener: TcpListener,
    pub health: Arc<WorldHealth>,
}

/// Build the sim, serve `/traffic` + `/live` + the extra routes on the one
/// listener, and run the 10 Hz tick loop forever (#91 timing lessons:
/// `MissedTickBehavior::Delay` + `yield_now` per tick). Returns only on a
/// build error — the loop itself never exits.
pub async fn run_world(args: WorldArgs) -> anyhow::Result<()> {
    let net = traffic_net::load(&args.net_json)
        .map_err(|e| anyhow::anyhow!("validate traffic net: {e:?}"))?;
    let grid = CellGrid::build(&net);
    let cell_count = grid.cell_count();
    let trip_router = ChTripRouter::new(&net);
    let traffic_registry = Registry::new();
    let live_registry = Registry::new();

    let resumed_from = args.snapshot.as_ref().map(|s| s.clock.world_tick);
    let (mut world, mut schedule) = shell::build_sim(
        net,
        args.seed,
        args.trips,
        args.clock,
        SpawnerCfg {
            demand_scale: WORLD_BG_DEMAND_SCALE,
        },
        Some(WorldCoreExt {
            plugin: WorldCorePlugin {
                seed: args.economy,
                sim_world: Arc::clone(&args.sim_world),
                seed_params: args.seed_params,
            },
            router: Box::new(trip_router),
            snapshot: args.snapshot,
        }),
    );
    world.insert_resource(make_publisher(grid.clone(), traffic_registry.clone()));
    world.insert_resource(make_live_publisher(
        grid,
        live_registry.clone(),
        Arc::clone(&args.sim_world),
    ));

    if let Some(tick) = resumed_from {
        args.health.resumed.store(true, Ordering::Relaxed);
        crate::world_store::log_resume(tick);
    } else {
        crate::world_store::log_fresh();
    }
    tracing::info!(
        population = world.resource::<world_core::CitizenRegistry>().count,
        world_tick = world.resource::<WorldClock>().world_tick,
        cells = cell_count,
        persistence = args.persist.is_some(),
        "world runtime built"
    );

    // ONE port: card-hand/health routes + both WS gateways.
    let app = args
        .extra_router
        .merge(gateway::router(traffic_registry, cell_count))
        .merge(gateway::live_router(live_registry, cell_count));
    tokio::spawn(async move {
        // A serve error here is fatal for clients but must not kill the sim.
        if let Err(err) = axum::serve(args.listener, app).await {
            tracing::error!(%err, "http/ws server exited");
        }
    });

    // The 10 Hz tick loop with the non-blocking persist flush.
    use tokio::time::{Duration, MissedTickBehavior, interval};
    let write_in_flight = Arc::new(AtomicBool::new(false));
    let mut ticker = interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        schedule.run(&mut world);

        let world_tick = world.resource::<WorldClock>().world_tick;
        args.health.world_tick.store(world_tick, Ordering::Relaxed);
        args.health
            .audit_ok
            .store(world.resource::<AuditStatus>().ok, Ordering::Relaxed);

        if let Some(persist) = &args.persist
            && world_tick.is_multiple_of(PERSIST_EVERY_N_TICKS)
        {
            // Skip if the previous write is still in flight (coalesce).
            if !write_in_flight.swap(true, Ordering::AcqRel) {
                let snap = world_core::persist::extract(&world); // sync, consistent
                let store = Arc::clone(&persist.store);
                let world_id = persist.world_id.clone();
                let flag = Arc::clone(&write_in_flight);
                tokio::spawn(async move {
                    if let Err(err) = store.write(&world_id, world_tick, &snap).await {
                        tracing::error!(%err, world_tick, "world snapshot write failed");
                    }
                    flag.store(false, Ordering::Release);
                });
            } else {
                tracing::warn!(
                    world_tick,
                    "world snapshot write still in flight — skipping this flush"
                );
            }
        }

        tokio::task::yield_now().await;
    }
}
