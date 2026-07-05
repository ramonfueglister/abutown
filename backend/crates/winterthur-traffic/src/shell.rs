//! The headless `bevy_ecs` orchestration shell.
//!
//! A single [`World`] holds the traffic net, sim kernel, CH router, spawner,
//! edge-measurement accumulator and the sim clock as resources. One chained
//! [`Schedule`] runs the systems in a **fixed, deterministic order** every
//! tick:
//!
//! ```text
//! drain_commands → spawn_trips → core_tick → measure_edges → publish_snapshot
//! ```
//!
//! Signals advance *inside* `core_tick` (the kernel gates turns on its own
//! signal windows), so there is no separate signals system. Congestion
//! re-routing is folded into `core_tick`'s system on a 30-sim-second cadence
//! (see [`REROUTE_INTERVAL_TICKS`]).
//!
//! [`build_sim`] constructs the `(World, Schedule)` pair with no timing or I/O,
//! so tests drive `schedule.run(&mut world)` as fast as the CPU allows. The
//! binary ([`crate::main`]) wraps this with the tokio 10 Hz interval and the
//! `/healthz` endpoint.
//!
//! # Publish seam (Task 8)
//!
//! `publish_snapshot` calls the [`SnapshotHook`] resource's closure once per
//! tick with a [`Snapshot`] borrow. The default hook is a no-op; the WS gateway
//! (Task 8) installs a real one via [`World::insert_resource`] before the loop
//! starts. No WS code lives here — only the seam.

use crate::Router;
use crate::audit::Conservation;
use crate::clock::WallClock;
use crate::demand::TripSchedule;
use crate::measure::EdgeMeasure;
use crate::spawner::{MAX_CONCURRENT, SpawnRecord, SpawnerCfg, TripSpawner};
use bevy_ecs::prelude::*;
use traffic_core::{Core, u01};
use traffic_net::TrafficNet;
use world_core::{
    CitizenCarCounters, CoreAccess, TripRouter, TripRouterRes, WorldClock, WorldCorePlugin,
    advance_world_clock_system, arrivals_system, dispatch_trips_system, econ_systems,
    install_world_resources, rhythm_system,
};

/// Re-routing cadence: every 30 sim-seconds = 300 ticks at dt=0.1.
pub const REROUTE_INTERVAL_TICKS: u64 = (30.0 / traffic_core::DT) as u64;

/// A vehicle is a re-route candidate when the congestion factor on its current
/// edge (measured / free-flow travel time) exceeds this ratio.
pub const DELAY_RATIO_THRESHOLD: f32 = 1.5;

/// Probability a *candidate* vehicle actually re-routes on a given re-route
/// tick (keeps the swap sparse + avoids herd re-routing).
pub const REROUTE_PROBABILITY: f32 = 0.1;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// The baked traffic network (immutable during a run).
#[derive(Resource)]
pub struct TrafficNetRes(pub TrafficNet);

/// The microscopic simulation kernel.
#[derive(Resource)]
pub struct CoreRes(pub Core);

/// world-core's trip-bridge systems are generic over this seam so they never
/// have to know the shell's resource type (no world-core → shell dependency).
impl CoreAccess for CoreRes {
    fn core(&self) -> &Core {
        &self.0
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.0
    }
}

/// The CH router (weights updated on the measurement cadence).
#[derive(Resource)]
pub struct RouterRes(pub Router);

/// The wall-clock census trip spawner.
#[derive(Resource)]
pub struct SpawnerRes(pub TripSpawner);

/// The per-edge harmonic-mean-speed measurement accumulator.
#[derive(Resource)]
pub struct MeasureRes(pub EdgeMeasure);

/// Monotonic sim clock (tick count). One tick = `traffic_core::DT` seconds.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SimClock {
    pub tick: u64,
}

/// Per-vehicle destination edge, indexed by `VehId` (slot). Grown as slots are
/// allocated; a stale entry for a since-despawned slot is harmless because the
/// re-route pass only reads entries for currently-alive vehicles.
#[derive(Resource, Default)]
pub struct TripRegistry {
    pub dest_edge: Vec<u32>,
}

impl TripRegistry {
    fn record(&mut self, veh: u32, dest_edge: u32) {
        let i = veh as usize;
        if i >= self.dest_edge.len() {
            self.dest_edge.resize(i + 1, u32::MAX);
        }
        self.dest_edge[i] = dest_edge;
    }
}

/// Base seed for all deterministic draws (spawner + re-route sampling).
#[derive(Resource, Clone, Copy)]
pub struct SimSeed(pub u64);

/// A stub inbound-command queue (player influence lands in a later task). The
/// `drain_commands` system empties it each tick; today it is always empty.
#[derive(Resource, Default)]
pub struct CommandQueue {
    pub pending: Vec<SimCommand>,
}

/// Placeholder command variant so the queue type is non-trivial and the drain
/// system has something concrete to consume once commands exist.
#[derive(Debug, Clone)]
pub enum SimCommand {}

/// Scratch buffer reused by `spawn_trips` for the [`SpawnRecord`]s a tick
/// produces, so the hot path allocates nothing steady-state.
#[derive(Resource, Default)]
struct SpawnScratch(Vec<SpawnRecord>);

// ---------------------------------------------------------------------------
// Publish seam (Task 8)
// ---------------------------------------------------------------------------

/// Read-only per-tick view handed to the publish hook. Deliberately minimal —
/// Task 8 decides the wire format; this just exposes what a publisher needs
/// without borrowing the whole `World`.
pub struct Snapshot<'a> {
    pub tick: u64,
    pub core: &'a Core,
    pub net: &'a TrafficNet,
}

/// Type of the publish callback: invoked once per tick after the kernel step.
type HookFn = Box<dyn Fn(&Snapshot<'_>) + Send + Sync>;

/// The publish seam. Default is a no-op; Task 8 replaces the closure.
#[derive(Resource)]
pub struct SnapshotHook(pub HookFn);

impl Default for SnapshotHook {
    fn default() -> Self {
        SnapshotHook(Box::new(|_snap| {}))
    }
}

impl SnapshotHook {
    /// Install a real publisher (Task 8).
    pub fn new(f: impl Fn(&Snapshot<'_>) + Send + Sync + 'static) -> Self {
        SnapshotHook(Box::new(f))
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// The optional world-sim extension for [`build_sim`] (Task 9): citizens +
/// economy from `world-core`, plus the [`TripRouter`] the trip bridge routes
/// citizen cars with (in production [`crate::ChTripRouter`] over the same
/// net; tests may inject a fixture router).
pub struct WorldCoreExt {
    pub plugin: WorldCorePlugin,
    pub router: Box<dyn TripRouter>,
}

/// Background census demand scale when the world sim is active: citizens
/// drive their own trips, so the anonymous census keeps only half its volume
/// (plan Task 9). Applied by the caller that wires the world in (Task 13's
/// sim-server), not silently here.
pub const WORLD_BG_DEMAND_SCALE: f32 = 0.5;

/// The [`WorldClock`] anchor at boot: world time starts aligned with the
/// wall clock's local second-of-day (so demand curves stay meaningful at
/// boot) and then runs at `WORLD_TIME_SCALE`× real time. A persisted world
/// (Task 11) overwrites this resource on resume — frozen time wins.
fn world_clock_anchored(clock: &WallClock) -> WorldClock {
    WorldClock {
        world_tick: u64::from(clock.s_of_day(0)) * world_core::TICKS_PER_SECOND
            / world_core::WORLD_TIME_SCALE,
    }
}

/// Build the `(World, Schedule)` for a headless run over `net`, seeded with
/// `seed`, spawning the census `trips` against the world clock (anchored at
/// the boot wall `clock`, running 6× real time), thinned per `cfg`. With
/// `ext = Some(..)` the world sim (citizens + economy, Task 9) is woven into
/// the SAME deterministic chain; `None` is the unchanged traffic-only run.
/// No timing / I/O — callers drive `schedule.run`.
pub fn build_sim(
    net: TrafficNet,
    seed: u64,
    trips: TripSchedule,
    clock: WallClock,
    cfg: SpawnerCfg,
    ext: Option<WorldCoreExt>,
) -> (World, Schedule) {
    let router = Router::new(&net);
    let core = Core::new(&net, MAX_CONCURRENT + 64, seed);
    let world_clock = world_clock_anchored(&clock);
    let spawner = TripSpawner::new(trips, clock, cfg, seed, &world_clock);
    let measure = EdgeMeasure::new(&net);

    let mut world = World::new();
    world.insert_resource(TrafficNetRes(net));
    world.insert_resource(CoreRes(core));
    world.insert_resource(RouterRes(router));
    world.insert_resource(SpawnerRes(spawner));
    world.insert_resource(MeasureRes(measure));
    world.insert_resource(SimClock::default());
    world.insert_resource(world_clock);
    world.insert_resource(TripRegistry::default());
    world.insert_resource(SimSeed(seed));
    world.insert_resource(CommandQueue::default());
    world.insert_resource(SpawnScratch::default());
    world.insert_resource(SnapshotHook::default());
    world.insert_resource(Conservation::default());
    // The trip-bridge counters exist in both modes so `book_citizen_cars`-
    // free traffic-only code never has to branch (they just stay 0).
    world.insert_resource(CitizenCarCounters::default());

    let mut schedule = Schedule::default();
    match ext {
        None => {
            // Traffic-only: unchanged order, plus the world clock advancing
            // first so census demand keeps flowing on world time.
            schedule.add_systems(
                (
                    advance_world_clock_system,
                    drain_commands,
                    spawn_trips,
                    core_tick,
                    measure_edges,
                    publish_snapshot,
                )
                    .chain(),
            );
        }
        Some(ext) => {
            // World sim: resources + ONE fixed chain interleaving world and
            // traffic systems (two separate chained tuples in one schedule
            // would be unordered relative to each other):
            //   clock → commands → census spawns → rhythm → trip dispatch →
            //   conservation booking → kernel tick → arrivals → econ chain →
            //   measure → publish.
            install_world_resources(&mut world, &ext.plugin);
            world.insert_resource(TripRouterRes(ext.router));
            schedule.add_systems(
                (
                    (
                        advance_world_clock_system,
                        drain_commands,
                        spawn_trips,
                        rhythm_system,
                        dispatch_trips_system::<CoreRes>,
                        book_citizen_cars,
                        core_tick,
                        arrivals_system::<CoreRes>,
                    )
                        .chain(),
                    econ_systems(),
                    (measure_edges, publish_snapshot).chain(),
                )
                    .chain(),
            );
        }
    }

    (world, schedule)
}

// ---------------------------------------------------------------------------
// Server loop (tokio timing + healthz) — shared by the binary and tests
// ---------------------------------------------------------------------------

/// Run the 10 Hz tick loop with a responsive `/healthz` endpoint on `port`.
///
/// Timing follows the #91 outage lesson: `interval(100 ms)` with
/// [`tokio::time::MissedTickBehavior::Delay`] (a slow tick never triggers a
/// burst of catch-up ticks) and `yield_now().await` after each tick so the HTTP
/// accept loop keeps making progress on a single vCPU. The health server runs
/// on its own task, so a busy tick can never block it.
///
/// Loops forever (until the process exits); the health task is spawned and left
/// running. Returns only on a bind error.
pub async fn run_loop(world: World, schedule: Schedule, port: u16) -> std::io::Result<()> {
    run_loop_with_router(world, schedule, port, None).await
}

/// Like [`run_loop`] but merges an extra axum router (e.g. the WS gateway's
/// `/traffic`) onto the same port alongside `/healthz`. The publish hook must
/// already be installed on `world` (see [`crate::gateway::make_publisher`]).
pub async fn run_loop_with_router(
    mut world: World,
    mut schedule: Schedule,
    port: u16,
    extra: Option<axum::Router>,
) -> std::io::Result<()> {
    use axum::{Router as AxumRouter, routing::get};
    use tokio::time::{Duration, MissedTickBehavior, interval};

    let mut app = AxumRouter::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/", get(|| async { "winterthur-traffic" }));
    if let Some(extra) = extra {
        app = app.merge(extra);
    }
    let health = app;
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, health).await;
    });

    let mut ticker = interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        schedule.run(&mut world);
        tokio::task::yield_now().await;
    }
}

// ---------------------------------------------------------------------------
// Systems (run in the chained order declared in build_sim)
// ---------------------------------------------------------------------------

/// Drain the inbound command queue. Stub for now: the queue is always empty, so
/// this is a no-op placeholder that reserves the first slot in the fixed order.
fn drain_commands(mut queue: ResMut<CommandQueue>) {
    queue.pending.clear();
}

/// Release this tick's scheduled trips, record their destinations and book
/// them into the conservation ledger.
// A bevy system's "arguments" are its resource accesses — the count is the
// ECS wiring surface, not a call-site burden (systems are never called
// directly).
#[allow(clippy::too_many_arguments)]
fn spawn_trips(
    mut core: ResMut<CoreRes>,
    net: Res<TrafficNetRes>,
    router: Res<RouterRes>,
    mut spawner: ResMut<SpawnerRes>,
    clock: Res<SimClock>,
    world_clock: Res<WorldClock>,
    mut registry: ResMut<TripRegistry>,
    mut scratch: ResMut<SpawnScratch>,
    mut conservation: ResMut<Conservation>,
) {
    scratch.0.clear();
    spawner.0.step(
        &mut core.0,
        &net.0,
        &router.0,
        clock.tick,
        &world_clock,
        &mut scratch.0,
    );
    for rec in &scratch.0 {
        registry.record(rec.veh, rec.dest_edge);
    }
    conservation.spawned += scratch.0.len() as u64;
    conservation.skipped_no_route = spawner.0.counters().skipped_no_route;
}

/// Fold the citizen-trip bridge's kernel spawns/manual despawns into the
/// vehicle-conservation ledger. Runs after `dispatch_trips_system` and
/// before `core_tick`, so both this tick's citizen spawns and last tick's
/// destination despawns (arrivals runs after `core_tick`) are booked before
/// the invariant is asserted. Kernel end-of-route despawns of citizen cars
/// are already counted by `core_tick` via `despawned_last_tick`.
fn book_citizen_cars(
    counters: Res<CitizenCarCounters>,
    mut conservation: ResMut<Conservation>,
    mut seen: Local<CitizenCarCounters>,
) {
    conservation.spawned += counters.spawned - seen.spawned;
    conservation.arrived += counters.despawned_at_destination - seen.despawned_at_destination;
    *seen = *counters;
}

/// Advance the kernel one tick (signals gate internally), book this tick's
/// end-of-route despawns as arrivals (incl. gateway sinks) and check the
/// conservation invariant, then run the periodic congestion re-route, then
/// bump the clock.
fn core_tick(
    mut core: ResMut<CoreRes>,
    net: Res<TrafficNetRes>,
    router: Res<RouterRes>,
    registry: Res<TripRegistry>,
    seed: Res<SimSeed>,
    mut clock: ResMut<SimClock>,
    mut conservation: ResMut<Conservation>,
) {
    let t = clock.tick;
    core.0.tick(t);

    conservation.arrived += core.0.despawned_last_tick().len() as u64;
    debug_assert!(
        conservation.holds(core.0.fleet.alive_count()),
        "vehicle conservation violated at tick {t}: {:?} alive={}",
        *conservation,
        core.0.fleet.alive_count()
    );

    if t > 0 && t.is_multiple_of(REROUTE_INTERVAL_TICKS) {
        reroute_congested(&mut core.0, &net.0, &router.0, &registry, seed.0, t);
    }

    clock.tick = t + 1;
}

/// Accumulate per-edge speed samples every tick; on a window boundary flush to
/// the router (update weights + rebuild CH).
fn measure_edges(
    core: Res<CoreRes>,
    net: Res<TrafficNetRes>,
    mut measure: ResMut<MeasureRes>,
    mut router: ResMut<RouterRes>,
    clock: Res<SimClock>,
) {
    measure.0.sample(&core.0, &net.0);
    // `clock.tick` was already advanced in `core_tick`, so the window closes on
    // the tick *after* the window's sim steps — one flush per window length
    // (5 sim-min by default).
    if measure.0.window_closes(clock.tick) {
        measure.0.flush(&mut router.0);
    }
}

/// Invoke the publish seam with a read-only snapshot. No-op by default.
fn publish_snapshot(
    hook: Res<SnapshotHook>,
    core: Res<CoreRes>,
    net: Res<TrafficNetRes>,
    clock: Res<SimClock>,
) {
    let snap = Snapshot {
        tick: clock.tick,
        core: &core.0,
        net: &net.0,
    };
    (hook.0)(&snap);
}

// ---------------------------------------------------------------------------
// Congestion re-routing
// ---------------------------------------------------------------------------

/// Sample alive vehicles on congested edges and, with [`REROUTE_PROBABILITY`],
/// re-query the router from the vehicle's current edge to its destination and
/// swap its route (keeping the current lane). Deterministic: candidacy and the
/// probability draw are pure functions of `u01(seed, t, veh | (1<<63))` (the
/// re-route draw stream is namespaced away from the spawner's).
///
/// # Limitations (documented per the plan)
///  * The delay proxy is the vehicle's **current-edge** congestion factor —
///    the edge's free speed over the vehicle's live speed — not a full
///    remaining-route expected-vs-free-flow ratio. The kernel does not track
///    per-vehicle expected time, and this is the cheapest deterministic proxy
///    that reacts to real congestion. A vehicle stuck behind a jam it has not
///    yet reached is not re-routed until it enters the congested edge. (A
///    future remaining-route proxy would consult [`EdgeMeasure::free_flow_s`];
///    it is not wired here yet, so the accumulator is not read in this pass.)
///  * The swap only takes effect when the router returns a route whose head is
///    the vehicle's current lane's edge and whose first lane is the current
///    lane (via `Core::reroute`'s guard); otherwise the vehicle keeps its old
///    route. No mid-lane teleporting is possible.
fn reroute_congested(
    core: &mut Core,
    net: &TrafficNet,
    router: &Router,
    registry: &TripRegistry,
    seed: u64,
    t: u64,
) {
    let slots = core.fleet.slots();
    for veh in 0..slots as u32 {
        let Some(view) = core.vehicle_view(veh) else {
            continue;
        };
        // Destination known?
        let Some(&dest_edge) = registry.dest_edge.get(veh as usize) else {
            continue;
        };
        if dest_edge == u32::MAX || dest_edge == view.edge {
            continue;
        }

        // Congestion factor on the current edge: how slow the vehicle is going
        // vs the edge free speed (a per-vehicle congestion proxy).
        let edge_speed = net.edges[view.edge as usize].speed_ms.max(0.1);
        let ratio = if view.v <= 0.05 {
            f32::INFINITY
        } else {
            edge_speed / view.v
        };
        if ratio <= DELAY_RATIO_THRESHOLD {
            continue;
        }

        // Probability gate (deterministic). Namespace the re-route draw stream
        // away from the spawner's `u01(seed, t, draw_counter)` (finding 2): the
        // spawner and this pass would otherwise share the `id` space at the same
        // `(seed, t)` and their draws would correlate. Setting the high bit puts
        // re-route draws in a disjoint half of the id space.
        let reroute_id = (veh as u64) | (1 << 63);
        if u01(seed, t, reroute_id) >= REROUTE_PROBABILITY {
            continue;
        }

        // Re-query from the current edge; the new route must start at the
        // current lane so the swap is a continuation.
        let Some(new_route) = router.route(net, view.edge, dest_edge) else {
            continue;
        };
        if new_route.first() != Some(&view.lane) {
            // Router picked a different lane on the current edge as the head;
            // Core::reroute would reject it. Skip — keep the old route.
            continue;
        }
        core.reroute(veh, &new_route);
    }
}
