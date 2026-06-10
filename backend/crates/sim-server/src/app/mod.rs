use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use abutown_protocol::ChunkSnapshotDto;
use abutown_protocol::v1 as w;
use axum::{
    Json, Router,
    extract::{
        FromRequest, Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{self, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use dashmap::DashMap;
use prost::Message as _;
use sim_core::{
    base_world::BaseWorldBundle,
    ids::ChunkCoord,
    persistence::{
        ChunkSnapshotStore, ChunkSnapshotStoreError, EconomyEventStore, EconomySnapshotStore,
        InMemoryChunkSnapshotStore, InMemoryEconomyEventStore, InMemoryEconomySnapshotStore,
        InMemoryMobilitySnapshotStore, MobilitySnapshotStore,
    },
};
use tokio::sync::{Mutex, broadcast};
use tokio_stream::StreamMap;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{
    card_hand::{
        AuthVerifier, CardHandError, CardHandResponse, CardHandStore, SaveCardHandRequest,
        card_definitions,
    },
    config::ServerConfig,
    persistence_liveness::{MobilityPersistenceHealthStatus, MobilityPersistenceLiveness},
    postgres_economy::PostgresEconomySnapshotStore,
    postgres_economy_events::PostgresEconomyEventStore,
    postgres_events::PostgresWorldEventStore,
    postgres_mobility::PostgresMobilitySnapshotStore,
    postgres_snapshots::PostgresChunkSnapshotStore,
    runtime::SimulationRuntime,
};

mod base_world_response;
pub use base_world_response::*;

mod proto_convert;
use proto_convert::*;

use crate::db::connect_shared_pool;

const DELTA_BROADCAST_CAPACITY: usize = 64;
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_millis(100);
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);
const ECONOMY_EVENTS_PRUNE_INTERVAL: Duration = Duration::from_secs(300);
/// Rolling row cap for the durable economy audit log (per world). With the
/// durable-event filter (~33 k rows/day live) this retains roughly a week of
/// financial history. Env-tunable like `ABUTOWN_BASE_WORLD_PATH`.
const ECONOMY_EVENTS_RETENTION_CAP_DEFAULT: u64 = 200_000;

fn economy_events_retention_cap() -> u64 {
    std::env::var("ABUTOWN_ECONOMY_EVENTS_RETENTION_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ECONOMY_EVENTS_RETENTION_CAP_DEFAULT)
}
const MOBILITY_PERSISTENCE_FRESHNESS_WINDOW: Duration = Duration::from_secs(15);
const BASE_WORLD_DEFAULT_PATH: &str = "data/worlds/abutopia";

fn resolve_base_world_path() -> PathBuf {
    std::env::var("ABUTOWN_BASE_WORLD_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                // infallible: fixed crate layout — sim-server always sits 3 levels under the repo root
                .nth(3)
                .expect("sim-server crate lives under backend/crates/sim-server")
                .join(BASE_WORLD_DEFAULT_PATH)
        })
}

#[derive(Clone)]
pub struct AppState {
    deltas: broadcast::Sender<w::ServerMessage>,
    card_hands: CardHandStore,
    auth: AuthVerifier,
    snapshot_store: Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>>,
    economy_snapshot_store: Arc<Mutex<Box<dyn EconomySnapshotStore + Send + Sync>>>,
    economy_event_store: Arc<Mutex<Box<dyn EconomyEventStore + Send + Sync>>>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>>,
    view: Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
    mutations: tokio::sync::mpsc::UnboundedSender<crate::runtime_view::Mutation>,
    base_world: Arc<BaseWorldResponse>,
    mobility_liveness: Arc<MobilityPersistenceLiveness>,
}

impl AppState {
    pub fn new(runtime: SimulationRuntime) -> Self {
        let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
            .expect("base world bundle present (test/dev convenience; production uses build_app_from_config which propagates errors)");
        Self::new_with_stores(
            runtime,
            &base_world,
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryEconomySnapshotStore::default()),
            Box::new(InMemoryEconomyEventStore::default()),
            CardHandStore::memory(),
            AuthVerifier::local_bearer_uuid(),
        )
    }

    pub fn new_with_card_hands(
        runtime: SimulationRuntime,
        card_hands: CardHandStore,
        auth: AuthVerifier,
    ) -> Self {
        let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
            .expect("base world bundle present (test/dev convenience; production uses build_app_from_config which propagates errors)");
        Self::new_with_stores(
            runtime,
            &base_world,
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryEconomySnapshotStore::default()),
            Box::new(InMemoryEconomyEventStore::default()),
            card_hands,
            auth,
        )
    }

    // Dependency-injection constructor wiring all persistence stores in one place;
    // the arg count is inherent to that (repo convention applies this allow to such
    // cohesive-parameter functions rather than introducing a pass-through struct).
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_stores(
        runtime: SimulationRuntime,
        base_world: &BaseWorldBundle,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send + Sync>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send + Sync>,
        economy_snapshot_store: Box<dyn EconomySnapshotStore + Send + Sync>,
        economy_event_store: Box<dyn EconomyEventStore + Send + Sync>,
        card_hands: CardHandStore,
        auth: AuthVerifier,
    ) -> Self {
        let (deltas, _) = broadcast::channel(DELTA_BROADCAST_CAPACITY);
        let initial_view =
            build_read_view_from_runtime(&runtime, &std::collections::HashMap::new(), None);
        let (mutation_tx, mutation_rx) = tokio::sync::mpsc::unbounded_channel();
        let view = Arc::new(arc_swap::ArcSwap::from_pointee(initial_view));
        let chunk_channels: Arc<DashMap<_, _>> = Arc::new(DashMap::new());
        let mobility_liveness = Arc::new(MobilityPersistenceLiveness::new(
            MOBILITY_PERSISTENCE_FRESHNESS_WINDOW,
        ));

        let state = Self {
            deltas: deltas.clone(),
            card_hands,
            auth,
            snapshot_store: Arc::new(Mutex::new(snapshot_store)),
            mobility_snapshot_store: Arc::new(Mutex::new(mobility_snapshot_store)),
            economy_snapshot_store: Arc::new(Mutex::new(economy_snapshot_store)),
            economy_event_store: Arc::new(Mutex::new(economy_event_store)),
            chunk_channels: Arc::clone(&chunk_channels),
            view: Arc::clone(&view),
            mutations: mutation_tx,
            base_world: Arc::new(BaseWorldResponse::from(base_world)),
            mobility_liveness,
        };

        // Panic supervisor: if tick_loop panics, every reader is stuck on
        // the last-published view forever and every Mutation::send fails.
        // Log loudly and abort so an external supervisor (systemd, k8s,
        // container restart policy) can recover us instead of running a
        // zombie server that serves stale data with no recovery path.
        let tick_fut = tick_loop(
            runtime,
            mutation_rx,
            view,
            deltas,
            chunk_channels,
            SIMULATION_TICK_INTERVAL,
        );
        let supervised = tokio::spawn(tick_fut);
        tokio::spawn(async move {
            match supervised.await {
                Ok(()) => {
                    tracing::error!("tick_loop exited normally — should run forever");
                    std::process::abort();
                }
                Err(join_err) => {
                    if join_err.is_panic() {
                        let panic = join_err.into_panic();
                        let msg = panic
                            .downcast_ref::<&'static str>()
                            .copied()
                            .or_else(|| panic.downcast_ref::<String>().map(|s| s.as_str()))
                            .unwrap_or("<non-string panic>");
                        tracing::error!(panic = %msg, "tick_loop panicked");
                    } else {
                        tracing::error!(?join_err, "tick_loop task cancelled");
                    }
                    std::process::abort();
                }
            }
        });

        state
    }

    pub(crate) fn view(&self) -> Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>> {
        Arc::clone(&self.view)
    }

    fn snapshot_store(&self) -> Arc<Mutex<Box<dyn ChunkSnapshotStore + Send + Sync>>> {
        Arc::clone(&self.snapshot_store)
    }

    fn mobility_snapshot_store(&self) -> Arc<Mutex<Box<dyn MobilitySnapshotStore + Send + Sync>>> {
        Arc::clone(&self.mobility_snapshot_store)
    }

    fn economy_snapshot_store(&self) -> Arc<Mutex<Box<dyn EconomySnapshotStore + Send + Sync>>> {
        Arc::clone(&self.economy_snapshot_store)
    }

    fn economy_event_store(&self) -> Arc<Mutex<Box<dyn EconomyEventStore + Send + Sync>>> {
        Arc::clone(&self.economy_event_store)
    }

    pub(crate) fn mobility_liveness(&self) -> Arc<MobilityPersistenceLiveness> {
        Arc::clone(&self.mobility_liveness)
    }

    fn base_world(&self) -> Arc<BaseWorldResponse> {
        Arc::clone(&self.base_world)
    }

    pub(crate) fn chunk_channels(
        &self,
    ) -> Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>> {
        Arc::clone(&self.chunk_channels)
    }

    /// Read a chunk snapshot directly from the snapshot store.
    /// Used by tests and diagnostic tooling; not on the hot path.
    pub async fn stored_chunk_snapshot(
        &self,
        coord: ChunkCoord,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        let store = self.snapshot_store();
        let store = store.lock().await;
        let compatibility = sim_core::persistence::SnapshotCompatibility::new(
            self.base_world.world_id.clone(),
            self.base_world.schema_version,
        );
        store.read_snapshot(coord, &compatibility).await
    }

    fn subscribe_deltas(&self) -> broadcast::Receiver<w::ServerMessage> {
        self.deltas.subscribe()
    }

    fn spawn_snapshot_loop(&self, snapshot_interval: Duration) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(snapshot_interval);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(error) = persist_snapshots_once(&state).await {
                    tracing::warn!(%error, "failed to persist chunk snapshots");
                }
            }
        });
    }

    /// Low-frequency rolling retention for the economy audit log: keep the most
    /// recent `keep_last` rows per world (2026-06-10 retention design — the
    /// table grew unbounded to 1.9 M rows / 449 MB in two days live). Best-effort
    /// like the audit append itself.
    fn spawn_economy_events_retention_loop(&self, prune_interval: Duration, keep_last: u64) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(prune_interval);
            interval.tick().await;
            loop {
                interval.tick().await;
                let world_id = state.view().load().world_id.clone();
                let store = state.economy_event_store();
                let mut store = store.lock().await;
                match store.prune(&world_id.0, keep_last).await {
                    Ok(0) => {}
                    Ok(deleted) => {
                        tracing::info!(deleted, keep_last, "pruned economy audit events")
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to prune economy audit events")
                    }
                }
            }
        });
    }
}

fn build_economy_snapshot(
    world: &sim_core::bevy_ecs::world::World,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> w::EconomySnapshot {
    use sim_core::economy::{
        AccountBook, FlowRateEwma, InputPools, MarketGoods, Markets, ProducerPolicies,
        WageTelemetry, capita::CapitaFactor, wc_target,
    };
    use sim_core::mobility::resources::{
        AgentIdIndex, CitizenEconomicTargets, RouteAssignmentStats,
    };
    use sim_core::routing::Graph;
    let markets_res = world.resource::<Markets>();
    let goods_res = world.resource::<MarketGoods>();
    let flows_res = world.resource::<FlowRateEwma>();
    let wages = world.resource::<WageTelemetry>();
    let graph = world.resource::<Graph>();
    let markets = markets_res
        .0
        .iter()
        .map(|(id, site)| {
            let pos = graph.node(site.node_id).position; // (f32, f32) tile coords
            w::EconomyMarket {
                market_id: id.0,
                name: site.name.clone(),
                tile_x: pos.0.floor() as i32,
                tile_y: pos.1.floor() as i32,
                wage_paid_last_tick: wages
                    .0
                    .get(id)
                    .copied()
                    .unwrap_or(sim_core::economy::Money::ZERO)
                    .0,
            }
        })
        .collect();
    let goods = goods_res
        .0
        .iter()
        .map(|(key, st)| w::EconomyMarketGood {
            market_id: key.market.0,
            good_id: u32::from(key.good.0),
            last_settlement_price: st.last_settlement_price.0,
            ewma_reference_price: st.ewma_reference_price.0,
            traded_qty_last_tick: st.traded_qty_last_tick.0,
            unmet_demand_last_tick: st.unmet_demand_last_tick.0,
            unsold_supply_last_tick: st.unsold_supply_last_tick.0,
        })
        .collect();
    let flows = flows_res
        .0
        .iter()
        .map(|(&(src, dst, good), &rate)| w::EconomyFlow {
            src_market_id: src.0,
            dst_market_id: dst.0,
            good_id: u32::from(good.0),
            rate: rate.0,
        })
        .collect();
    let producers = {
        let pools = world.resource::<InputPools>();
        let policies = world.resource::<ProducerPolicies>();
        let accounts = world.resource::<AccountBook>();
        let capita = world.resource::<CapitaFactor>().0;
        pools
            .0
            .iter()
            .map(|(&actor, pool)| {
                // Keyset invariant (seed-asserted by assert_producer_keysets_match):
                // every InputPool entry has a matching ProducerPolicy.
                let policy = policies
                    .0
                    .get(&actor)
                    .copied()
                    .expect("InputPools entry without ProducerPolicy — seed keyset invariant");
                // wc_target is caller-must-guard max_price > 0; an unpriced pool
                // (bound not yet discovered) reports target 0, mirroring the
                // dividend path's conservative retention (wages.rs).
                let wc = if pool.max_price.0 > 0 {
                    wc_target(policy, pool, capita)
                        .expect("wc_target on a priced, seed-validated pool cannot overflow")
                        .0
                } else {
                    0
                };
                w::EconomyProducer {
                    actor_id: actor.0,
                    market_id: pool.market.0,
                    in_good: u32::from(pool.good.0),
                    out_good: u32::from(pool.out_good.0),
                    retained_earnings: accounts.account(actor).available.0,
                    wc_target: wc,
                    max_bid: pool.max_price.0,
                    in_qty: pool.in_qty.0,
                    out_qty: pool.out_qty.0,
                }
            })
            .collect()
    };
    // Every caller's world installs EconomyPlugin + mobility (constructors at
    // runtime/mod.rs:218/368), so absence of any of these resources is a
    // programming error and must panic.
    let vitals = {
        let total_money = world
            .resource::<AccountBook>()
            .total_money()
            .expect("tick audit guarantees a summable ledger")
            .0;
        let routed = world.resource::<CitizenEconomicTargets>().0.len() as u64;
        let stats = *world.resource::<RouteAssignmentStats>();
        let population = world.resource::<AgentIdIndex>().0.len() as u64;
        Some(w::EconomyVitals {
            population,
            routed_citizens: routed,
            total_money,
            routes_assigned: stats.assigned,
            routes_failed: stats.failed,
        })
    };
    w::EconomySnapshot {
        protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
        world_id: world_id.0.clone(),
        tick,
        markets,
        goods,
        vitals,
        flows,
        producers,
    }
}

fn build_read_view_from_runtime(
    runtime: &SimulationRuntime,
    per_chunk: &std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        sim_core::mobility::MobilityChunkDelta,
    >,
    prev: Option<&crate::runtime_view::RuntimeReadView>,
) -> crate::runtime_view::RuntimeReadView {
    let world_id = runtime.world_id_for_persist().clone();
    let mobility_tick = runtime.mobility_tick();
    let mobility = runtime.mobility();

    let per_chunk_deltas: Vec<w::MobilityChunkDelta> = per_chunk
        .values()
        .map(|delta| chunk_delta_to_dto(delta, mobility, &world_id, mobility_tick))
        .collect();
    // `mobility` is `&sim_core::bevy_ecs::world::World` after Task 9.

    // Pre-materialize per-chunk tile + mobility snapshots for every loaded
    // chunk so HTTP /chunks/{x}/{y} and WS lagged-recovery can read without
    // a lock.
    //
    // Tile snapshots are version-gated against the previous view: a chunk
    // whose `ChunkVersion` is unchanged reuses the cached `Arc` instead of
    // re-reading ~1024 tiles + re-encoding a proto. Tiles change only on
    // `SetTileKind` commands or chunk (un)load, so on a quiet tick this loop
    // does no tile work at all (2026-06-10 tick-cost design — the full
    // per-tick rebuild dominated the saturated tick loop behind the
    // 2026-06-09 outage).
    let world_summary_legacy = runtime.world_summary();
    let world_summary = world_summary_dto_to_proto(&world_summary_legacy);
    let loaded_coords: Vec<sim_core::ids::ChunkCoord> = world_summary_legacy
        .loaded_chunks
        .iter()
        .map(|c| sim_core::ids::ChunkCoord { x: c.x, y: c.y })
        .collect();
    let mut chunk_snapshots: std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        Arc<w::ChunkSnapshot>,
    > = std::collections::HashMap::new();
    for &coord in &loaded_coords {
        let cached = prev
            .and_then(|p| p.chunk_snapshots.get(&coord))
            .filter(|cached| runtime.chunk_version(coord) == Some(cached.chunk_version));
        let snap = match cached {
            Some(arc) => Some(Arc::clone(arc)),
            None => runtime
                .chunk_snapshot(coord)
                .map(|snap| Arc::new(chunk_snapshot_dto_to_proto(&snap))),
        };
        if let Some(snap) = snap {
            chunk_snapshots.insert(coord, snap);
        }
    }

    // Mobility snapshots change every tick (agents move), so they are always
    // rebuilt — but in ONE O(agents) bucketing pass over all loaded chunks,
    // not a full agent scan per chunk (was O(chunks × agents)).
    let mobility_chunk_snapshots: std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        w::MobilityChunkSnapshot,
    > = sim_core::mobility::api::build_mobility_chunk_snapshots(mobility, &loaded_coords)
        .into_iter()
        .map(|(coord, snap)| {
            (
                coord,
                chunk_snapshot_to_dto(&snap, mobility, &world_id, mobility_tick),
            )
        })
        .collect();

    let chunk_subscriber_counts =
        sim_core::mobility::api::chunk_subscriber_counts_snapshot(mobility);
    let mobility_full_legacy = runtime.mobility_snapshot();
    let mobility_full_dto = mobility_snapshot_dto_to_proto(&mobility_full_legacy);
    let health_legacy = runtime.health();
    let health = health_dto_to_proto(&health_legacy);
    let economy = build_economy_snapshot(mobility, &world_id, mobility_tick);

    crate::runtime_view::RuntimeReadView {
        tick: mobility_tick,
        world_id: world_id.clone(),
        mobility_tick,
        health,
        world_summary,
        chunk_snapshots,
        mobility_chunk_snapshots,
        mobility_full_dto,
        per_chunk_deltas,
        chunk_subscriber_counts,
        economy,
    }
}

pub fn build_app() -> Router {
    // dev/test entry; production boots via build_app_from_config which propagates errors
    let runtime = SimulationRuntime::new_from_base_world_dir(resolve_base_world_path())
        .expect("base world bundle is required for app startup");
    build_app_with_runtime(runtime)
}

pub fn build_app_with_allowed_origins(allowed_origins: &[String]) -> anyhow::Result<Router> {
    let runtime = SimulationRuntime::new_from_base_world_dir(resolve_base_world_path())?;
    let state = AppState::new(runtime);
    let cors = cors_layer(allowed_origins)?;
    Ok(build_router_from_state(state, cors))
}

pub async fn build_app_from_env() -> anyhow::Result<Router> {
    let _ = dotenvy::dotenv();
    let config = ServerConfig::from_env()?;
    build_app_from_config(&config).await
}

pub async fn build_app_from_config(config: &ServerConfig) -> anyhow::Result<Router> {
    let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())?;
    let pool = connect_shared_pool(&config.database_url).await?;
    let event_store = PostgresWorldEventStore::with_pool(pool.clone()).await?;
    let snapshot_store = PostgresChunkSnapshotStore::with_pool(
        pool.clone(),
        abutown_protocol::WorldId(base_world.world_id().to_owned()),
        base_world.snapshot_compatibility(),
    )
    .await?;
    let mobility_snapshot_store = PostgresMobilitySnapshotStore::with_pool(pool.clone()).await?;
    let economy_snapshot_store = PostgresEconomySnapshotStore::with_pool(pool.clone()).await?;
    let economy_event_store = PostgresEconomyEventStore::with_pool(pool.clone()).await?;
    let card_hands = CardHandStore::with_pool(pool.clone()).await?;
    let auth = AuthVerifier::supabase(&config.supabase_url).await;

    let (runtime, snapshot_store, mobility_snapshot_store, economy_snapshot_store) =
        SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(mobility_snapshot_store),
            Box::new(economy_snapshot_store),
            &base_world,
        )
        .await?;

    let state = AppState::new_with_stores(
        runtime,
        &base_world,
        snapshot_store,
        mobility_snapshot_store,
        economy_snapshot_store,
        Box::new(economy_event_store),
        card_hands,
        auth,
    );
    let cors = cors_layer(&config.cors_allowed_origins)?;
    Ok(build_router_from_state(state, cors))
}

pub fn build_app_with_runtime(runtime: SimulationRuntime) -> Router {
    build_app_with_runtime_and_card_hands(
        runtime,
        CardHandStore::memory(),
        AuthVerifier::local_bearer_uuid(),
    )
}

pub fn build_app_with_runtime_and_card_hands(
    runtime: SimulationRuntime,
    card_hands: CardHandStore,
    auth: AuthVerifier,
) -> Router {
    let state = AppState::new_with_card_hands(runtime, card_hands, auth);
    // infallible: hardcoded empty origin slice can never contain a malformed origin
    let cors = cors_layer(&[]).expect("empty origin list is always valid");
    build_router_from_state(state, cors)
}

/// Build a fail-closed CORS layer from an explicit allow-list. An empty list
/// allows no cross-origin requests. Malformed origins are a startup error.
fn cors_layer(allowed_origins: &[String]) -> anyhow::Result<CorsLayer> {
    use axum::http::{HeaderValue, Method, header};

    let origins = allowed_origins
        .iter()
        .map(|origin| {
            origin
                .parse::<HeaderValue>()
                .map_err(|err| anyhow::anyhow!("invalid CORS origin {origin:?}: {err}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Methods are an explicit allow-list matching the routes this server
    // exposes (GET reads, POST /commands, PUT /card-hand). DELETE/PATCH are
    // deliberately omitted — add them here if a future endpoint needs them,
    // otherwise the browser preflight will block that verb.
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
}

fn build_router_from_state(state: AppState, cors: CorsLayer) -> Router {
    // tick_loop is already running (spawned in new_with_stores). Only the
    // periodic persist loop needs to be spawned here, since it depends on the
    // AppState clone (view + mutations).
    state.spawn_snapshot_loop(SNAPSHOT_INTERVAL);
    state.spawn_economy_events_retention_loop(
        ECONOMY_EVENTS_PRUNE_INTERVAL,
        economy_events_retention_cap(),
    );

    Router::new()
        .route("/health", get(health))
        .route("/cards", get(cards))
        .route("/card-hand", get(card_hand).put(save_card_hand))
        .route("/world", get(world))
        .route("/base-world", get(base_world))
        .route("/chunks/{x}/{y}", get(chunk))
        .route("/commands", post(command))
        .route("/mobility", get(mobility))
        .route("/economy", get(economy))
        .route("/ws", get(websocket))
        .with_state(state)
        .layer(cors)
}

async fn health(State(state): State<AppState>) -> Response {
    proto_response(health_response_for_state(&state))
}

fn health_response_for_state(state: &AppState) -> w::HealthResponse {
    let view = state.view().load();
    let mut health = view.health.clone();
    let persistence = state.mobility_liveness().snapshot();
    let runtime_agents_ok = !view.mobility_full_dto.agents.is_empty();
    health.ok = health.ok
        && runtime_agents_ok
        && persistence.status != MobilityPersistenceHealthStatus::Stale;
    health.persistence = Some(persistence_health_to_proto(persistence));
    health
}

fn persistence_health_to_proto(
    health: crate::persistence_liveness::MobilityPersistenceHealth,
) -> w::PersistenceHealth {
    w::PersistenceHealth {
        status: match health.status {
            MobilityPersistenceHealthStatus::Starting => {
                w::PersistenceHealthStatus::Starting as i32
            }
            MobilityPersistenceHealthStatus::Healthy => w::PersistenceHealthStatus::Healthy as i32,
            MobilityPersistenceHealthStatus::Degraded => {
                w::PersistenceHealthStatus::Degraded as i32
            }
            MobilityPersistenceHealthStatus::Stale => w::PersistenceHealthStatus::Stale as i32,
        },
        world_id: health.world_id.unwrap_or_default(),
        mobility_tick: health.mobility_tick.unwrap_or_default(),
        last_attempt_unix_ms: system_time_to_unix_ms(health.last_attempt),
        last_success_unix_ms: system_time_to_unix_ms(health.last_success),
        consecutive_failures: health.consecutive_failures,
        last_error: health.last_error.unwrap_or_default(),
        freshness_ms: duration_to_ms(health.freshness),
    }
}

fn system_time_to_unix_ms(time: Option<SystemTime>) -> u64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration_to_ms(Some(duration)))
        .unwrap_or_default()
}

fn duration_to_ms(duration: Option<Duration>) -> u64 {
    duration
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

async fn world(State(state): State<AppState>) -> Response {
    proto_response(state.view().load().world_summary.clone())
}

async fn mobility(State(state): State<AppState>) -> Response {
    proto_response(state.view().load().mobility_full_dto.clone())
}

/// Backend-only debug view: returns the live economy snapshot as JSON.
async fn economy(State(state): State<AppState>) -> Response {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if state
        .mutations
        .send(crate::runtime_view::Mutation::CollectEconomySnapshot { reply: tx })
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match rx.await {
        Ok(snap) => match serde_json::to_vec(&snap) {
            Ok(bytes) => {
                ([(http::header::CONTENT_TYPE, "application/json")], bytes).into_response()
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn base_world(State(state): State<AppState>) -> Json<BaseWorldResponse> {
    Json((*state.base_world()).clone())
}

/// Encode any prost message as an `application/x-protobuf` HTTP response.
fn proto_response<M: prost::Message>(message: M) -> Response {
    let bytes = message.encode_to_vec();
    (
        [(http::header::CONTENT_TYPE, "application/x-protobuf")],
        bytes,
    )
        .into_response()
}

/// Extractor that decodes the request body as a prost message. Mirrors the
/// shape of `axum::Json` but for `application/x-protobuf` bodies. Used by
/// `POST /commands` after Task 6.
pub struct ProtoBody<M>(pub M);

impl<S, M> FromRequest<S> for ProtoBody<M>
where
    S: Send + Sync,
    M: prost::Message + Default,
{
    type Rejection = (StatusCode, String);

    async fn from_request(
        req: http::Request<axum::body::Body>,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let msg =
            M::decode(bytes.as_ref()).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        Ok(ProtoBody(msg))
    }
}

async fn cards() -> Json<Vec<crate::card_hand::CardDefinition>> {
    Json(card_definitions())
}

async fn card_hand(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match state.auth.authenticate(&headers).await {
        Ok(user_id) => user_id,
        Err(error) => return card_hand_error(error),
    };
    match state.card_hands.get_or_create(user_id).await {
        Ok(cards) => Json(CardHandResponse {
            user_id: user_id.to_string(),
            cards,
        })
        .into_response(),
        Err(error) => card_hand_error(error),
    }
}

async fn save_card_hand(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveCardHandRequest>,
) -> Response {
    let user_id = match state.auth.authenticate(&headers).await {
        Ok(user_id) => user_id,
        Err(error) => return card_hand_error(error),
    };
    match state.card_hands.save(user_id, request.cards.clone()).await {
        Ok(()) => Json(CardHandResponse {
            user_id: user_id.to_string(),
            cards: request.cards,
        })
        .into_response(),
        Err(error) => card_hand_error(error),
    }
}

fn card_hand_error(error: CardHandError) -> Response {
    let status = match error {
        CardHandError::MissingAuth | CardHandError::InvalidAuth => StatusCode::UNAUTHORIZED,
        CardHandError::UnknownCard(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CardHandError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": error.to_string() })),
    )
        .into_response()
}

async fn chunk(State(state): State<AppState>, Path((x, y)): Path<(i32, i32)>) -> Response {
    let coord = sim_core::ids::ChunkCoord { x, y };
    match state.view().load().chunk_snapshots.get(&coord).cloned() {
        Some(snap) => proto_response(w::ChunkSnapshot::clone(&snap)),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn command(
    State(state): State<AppState>,
    ProtoBody(command): ProtoBody<w::ClientCommand>,
) -> Response {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .mutations
        .send(crate::runtime_view::Mutation::ApplyCommand {
            command,
            reply: reply_tx,
        })
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    let result = match reply_rx.await {
        Ok(r) => r,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    match result {
        Ok(applied) => {
            let event_proto = world_event_dto_to_proto(&applied.event);
            let msg = w::ServerMessage {
                body: Some(w::server_message::Body::WorldEvent(event_proto.clone())),
            };
            let _ = state.deltas.send(msg);
            (
                StatusCode::OK,
                proto_response(w::CommandResponse {
                    outcome: Some(w::command_response::Outcome::Accepted(w::CommandAccepted {
                        protocol_version: u32::from(applied.response.protocol_version),
                        world_id: applied.response.world_id.0.clone(),
                        command_id: applied.response.command_id.clone(),
                        event: Some(event_proto),
                    })),
                }),
            )
                .into_response()
        }
        Err(rejection) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            proto_response(w::CommandResponse {
                outcome: Some(w::command_response::Outcome::Rejected(w::CommandRejected {
                    protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
                    world_id: rejection
                        .world_id
                        .as_ref()
                        .map(|w| w.0.clone())
                        .unwrap_or_default(),
                    command_id: rejection.command_id.clone().unwrap_or_default(),
                    code: rejection.code.to_string(),
                    message: rejection.message.clone(),
                })),
            }),
        )
            .into_response(),
    }
}

async fn websocket(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_world_deltas(socket, state))
}

struct ConnectionState {
    subscription: std::collections::HashSet<sim_core::ids::ChunkCoord>,
    chunk_streams: StreamMap<sim_core::ids::ChunkCoord, BroadcastStream<w::MobilityChunkDelta>>,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            subscription: std::collections::HashSet::new(),
            chunk_streams: StreamMap::new(),
        }
    }
}

async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let mut deltas = state.subscribe_deltas();
    let hello = {
        let view = state.view().load();
        w::ServerMessage {
            body: Some(w::server_message::Body::Hello(w::Hello {
                protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
                world_id: view.world_id.0.clone(),
                chunk_size: view.world_summary.chunk_size,
            })),
        }
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }
    let economy_msg = w::ServerMessage {
        body: Some(w::server_message::Body::EconomySnapshot(
            state.view().load().economy.clone(),
        )),
    };
    if send_server_message(&mut socket, economy_msg).await.is_err() {
        return;
    }

    let mut connection = ConnectionState::new();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                let Some(Ok(message)) = inbound else { break; };
                let Message::Binary(bytes) = message else { continue; };
                let client_message = match w::ClientMessage::decode(bytes.as_ref()) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(?e, "invalid client message");
                        continue;
                    }
                };
                let outgoing = handle_client_message(&state, &client_message, &mut connection).await;
                let mut errored = false;
                for msg in outgoing {
                    if send_server_message(&mut socket, msg).await.is_err() {
                        errored = true;
                        break;
                    }
                }
                if errored {
                    break;
                }
            }
            Some((chunk, item)) = tokio_stream::StreamExt::next(&mut connection.chunk_streams), if !connection.chunk_streams.is_empty() => {
                use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
                match item {
                    Ok(delta) => {
                        let msg = w::ServerMessage {
                            body: Some(w::server_message::Body::MobilityChunkDelta(delta)),
                        };
                        if send_server_message(&mut socket, msg).await.is_err() {
                            break;
                        }
                    }
                    Err(BroadcastStreamRecvError::Lagged(_)) => {
                        // Recovery: re-send a fresh snapshot for this chunk
                        // from the lock-free RuntimeReadView. If the chunk
                        // isn't in the view (e.g., it just unloaded), skip.
                        let snap = state.view().load().mobility_chunk_snapshots.get(&chunk).cloned();
                        if let Some(snap) = snap {
                            let msg = w::ServerMessage {
                                body: Some(w::server_message::Body::MobilityChunkSnapshot(snap)),
                            };
                            if send_server_message(&mut socket, msg).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            broadcast = deltas.recv() => {
                let message = match broadcast {
                    Ok(message) => message,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if send_server_message(&mut socket, message).await.is_err() {
                    break;
                }
            }
        }
    }

    // Cleanup: drop this connection's chunk subscriptions so the shared
    // ChunkSubscribers count stays consistent after disconnect.
    if !connection.subscription.is_empty() {
        let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
        let _ = state
            .mutations
            .send(crate::runtime_view::Mutation::SubscriptionDiff {
                added: Vec::new(),
                removed: connection.subscription.iter().copied().collect(),
                reply: reply_tx,
            });
    }
}

// ===== per-chunk delta + snapshot builders (proto outputs) =====

fn chunk_delta_to_dto(
    delta: &sim_core::mobility::MobilityChunkDelta,
    world: &sim_core::bevy_ecs::world::World,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> w::MobilityChunkDelta {
    w::MobilityChunkDelta {
        protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
        world_id: world_id.0.clone(),
        tick,
        chunk: Some(w::ChunkCoord {
            x: delta.chunk.x,
            y: delta.chunk.y,
        }),
        changed_agents: delta
            .changed_agents
            .iter()
            .filter_map(|r| sim_core::mobility::api::agent_dto_for(world, &r.id))
            .map(agent_dto_to_proto)
            .collect(),
        changed_vehicles: delta
            .changed_vehicles
            .iter()
            .filter_map(|r| sim_core::mobility::api::vehicle_dto_for(world, &r.id))
            .map(vehicle_dto_to_proto)
            .collect(),
        left_agents: delta.left_agents.iter().map(|id| id.0.clone()).collect(),
        left_vehicles: delta.left_vehicles.iter().map(|id| id.0.clone()).collect(),
    }
}

async fn apply_mutation_owned(
    runtime: &mut SimulationRuntime,
    mutation: crate::runtime_view::Mutation,
) {
    use crate::runtime_view::Mutation;
    match mutation {
        Mutation::ApplyCommand { command, reply } => {
            let result = match abutown_protocol::ClientCommandDto::try_from(command) {
                Ok(dto) => runtime.apply_client_command(dto).await,
                Err((code, message)) => Err(crate::commands::CommandRejection {
                    world_id: None,
                    command_id: None,
                    code,
                    message: message.to_string(),
                }),
            };
            let _ = reply.send(result);
        }
        Mutation::SubscriptionDiff {
            added,
            removed,
            reply,
        } => {
            runtime.apply_subscription_diff(added.iter(), removed.iter());
            if reply.is_closed() {
                return;
            }
            let world_id = runtime.world_id_for_persist().clone();
            let tick = runtime.mobility_tick();
            let snapshots: Vec<w::MobilityChunkSnapshot> = added
                .iter()
                .map(|coord| {
                    let snapshot = sim_core::mobility::api::build_mobility_chunk_snapshot(
                        runtime.mobility(),
                        *coord,
                    );
                    chunk_snapshot_to_dto(&snapshot, runtime.mobility(), &world_id, tick)
                })
                .collect();
            let _ = reply.send(snapshots);
        }
        Mutation::MarkChunkSnapshotsPersisted { coords } => {
            runtime.mark_chunk_snapshots_persisted(&coords);
        }
        Mutation::CommitLedgerAudit { count } => {
            runtime.commit_ledger_audit(count);
        }
        Mutation::CollectPersistData { reply } => {
            // Iterate registered SnapshotProviders and dispatch by
            // `key.kind`. The provider path is the source of truth for
            // persistence post-Phase-8a — `collect_chunk_snapshots()` /
            // `mobility_persist_snapshot()` remain on the runtime as
            // lower-level helpers (tests still use them) but are no longer
            // the persist call site.
            let items = runtime.collect_provider_items();
            let mut chunk_snapshots: Vec<abutown_protocol::ChunkSnapshotDto> = Vec::new();
            let mut mobility_world: Option<sim_core::mobility::MobilityPersistSnapshot> = None;
            let mut economy_world: Option<sim_core::economy::EconomyPersistSnapshot> = None;
            for item in items {
                match item.key.kind {
                    "chunk" => match serde_json::from_slice::<abutown_protocol::ChunkSnapshotDto>(
                        &item.payload,
                    ) {
                        Ok(dto) => chunk_snapshots.push(dto),
                        Err(error) => tracing::warn!(
                            %error,
                            kind = item.key.kind,
                            identifier = %item.key.identifier,
                            "provider emitted chunk payload that failed to deserialize",
                        ),
                    },
                    "mobility" => match serde_json::from_slice::<
                        sim_core::mobility::MobilityPersistSnapshot,
                    >(&item.payload)
                    {
                        Ok(snap) => mobility_world = Some(snap),
                        Err(error) => tracing::warn!(
                            %error,
                            kind = item.key.kind,
                            identifier = %item.key.identifier,
                            "provider emitted mobility payload that failed to deserialize",
                        ),
                    },
                    "economy" => match serde_json::from_slice::<
                        sim_core::economy::EconomyPersistSnapshot,
                    >(&item.payload)
                    {
                        Ok(snap) => economy_world = Some(snap),
                        Err(error) => tracing::warn!(
                            %error,
                            kind = item.key.kind,
                            identifier = %item.key.identifier,
                            "provider emitted economy payload that failed to deserialize",
                        ),
                    },
                    other => {
                        tracing::warn!(kind = other, "ignoring SnapshotItem with unknown kind",)
                    }
                }
            }
            // Stable ordering for chunk snapshots (matches the legacy
            // `collect_chunk_snapshots()` (y, x) sort).
            chunk_snapshots.sort_by_key(|s| (s.coord.y, s.coord.x));
            let (economy_audit_tick, economy_audit_pending) = runtime.pending_ledger_audit();
            let payload = crate::runtime_view::PersistPayload {
                chunk_snapshots,
                world_id: runtime.world_id_for_persist().clone(),
                mobility_tick: runtime.mobility_tick(),
                mobility_world: mobility_world
                    .unwrap_or_else(|| runtime.mobility_persist_snapshot()),
                economy_tick: runtime.mobility_tick(),
                economy_world: economy_world.unwrap_or_default(),
                economy_audit_tick,
                economy_audit_pending,
            };
            let _ = reply.send(payload);
        }
        Mutation::CollectEconomySnapshot { reply } => {
            let _ = reply.send(runtime.economy_snapshot());
        }
    }
}

/// Ticker for the simulation loop. `Delay` (instead of the tokio default
/// `Burst`) means missed ticks accrue no catch-up debt: under sustained
/// overload sim-time slows gracefully instead of queueing a backlog that
/// would later replay at max CPU speed.
fn simulation_ticker(interval: Duration) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

/// Pace one tick-loop iteration. Once per-tick work exceeds the interval the
/// interval is permanently overdue, so `tick()` resolves inline and yields to
/// the scheduler only sporadically (measured: every ~25–60 iterations). On a
/// 1-vCPU machine that starves the HTTP accept loop: requests need several
/// polls, each gated on one of those rare yields, so /health never answered
/// within Fly's 5 s check timeout while the sim kept ticking (2026-06-09
/// production outage). The explicit yield guarantees a scheduler pass per
/// iteration regardless of load.
async fn pace_tick(ticker: &mut tokio::time::Interval) {
    ticker.tick().await;
    tokio::task::yield_now().await;
}

/// Sole owner of the SimulationRuntime. Drains pending mutations, ticks the
/// world, fans out per-chunk deltas, publishes a new RuntimeReadView, and
/// broadcasts the per-tick economy snapshot. All in one task so no lock is needed.
async fn tick_loop(
    mut runtime: SimulationRuntime,
    mut mutation_rx: tokio::sync::mpsc::UnboundedReceiver<crate::runtime_view::Mutation>,
    view: Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
    deltas: broadcast::Sender<w::ServerMessage>,
    chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>>,
    interval: Duration,
) {
    let mut ticker = simulation_ticker(interval);
    ticker.tick().await;
    loop {
        pace_tick(&mut ticker).await;
        tick_once(
            &mut runtime,
            &mut mutation_rx,
            &view,
            &deltas,
            &chunk_channels,
        )
        .await;
    }
}

/// One iteration of the tick loop, extracted so tests can advance the world
/// without waiting on the real-time scheduler.
async fn tick_once(
    runtime: &mut SimulationRuntime,
    mutation_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::runtime_view::Mutation>,
    view: &Arc<arc_swap::ArcSwap<crate::runtime_view::RuntimeReadView>>,
    deltas: &broadcast::Sender<w::ServerMessage>,
    chunk_channels: &Arc<DashMap<ChunkCoord, broadcast::Sender<w::MobilityChunkDelta>>>,
) {
    // Phase 0: drain the entire mutation queue. We own the runtime exclusively
    // — no lock acquisition between mutations.
    while let Ok(mutation) = mutation_rx.try_recv() {
        apply_mutation_owned(runtime, mutation).await;
    }

    // Phase 1: tick mobility.
    let per_chunk = runtime.tick_world_mobility();
    let world_id = runtime.world_id_for_persist().clone();
    let mobility_tick = runtime.mobility_tick();

    // Phase 2: broadcast per-chunk delta DTOs to subscribed chunk channels.
    if !per_chunk.is_empty() {
        let mobility = runtime.mobility();
        for (chunk, delta) in &per_chunk {
            let Some(sender) = chunk_channels.get(chunk).map(|e| e.clone()) else {
                continue;
            };
            let dto = chunk_delta_to_dto(delta, mobility, &world_id, mobility_tick);
            let _ = sender.send(dto); // best-effort
        }
    }

    // Phase 3: publish the new RuntimeReadView for lock-free HTTP/WS readers.
    // The previous view feeds the version-gated tile-snapshot cache.
    let prev_view = view.load_full();
    let new_view = build_read_view_from_runtime(runtime, &per_chunk, Some(&prev_view));
    view.store(Arc::new(new_view));

    // Phase 4: broadcast the per-tick economy snapshot.
    let economy_msg = w::ServerMessage {
        body: Some(w::server_message::Body::EconomySnapshot(
            view.load().economy.clone(),
        )),
    };
    let _ = deltas.send(economy_msg);
}

fn chunk_snapshot_to_dto(
    snapshot: &sim_core::mobility::MobilityChunkSnapshot,
    world: &sim_core::bevy_ecs::world::World,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> w::MobilityChunkSnapshot {
    w::MobilityChunkSnapshot {
        protocol_version: u32::from(abutown_protocol::PROTOCOL_VERSION),
        world_id: world_id.0.clone(),
        tick,
        chunk: Some(w::ChunkCoord {
            x: snapshot.chunk.x,
            y: snapshot.chunk.y,
        }),
        agents: snapshot
            .agents
            .iter()
            .filter_map(|record| sim_core::mobility::api::agent_dto_for(world, &record.id))
            .map(agent_dto_to_proto)
            .collect(),
        vehicles: snapshot
            .vehicles
            .iter()
            .filter_map(|record| sim_core::mobility::api::vehicle_dto_for(world, &record.id))
            .map(vehicle_dto_to_proto)
            .collect(),
    }
}

async fn handle_client_message(
    state: &AppState,
    message: &w::ClientMessage,
    connection: &mut ConnectionState,
) -> Vec<w::ServerMessage> {
    use w::client_message::Body;
    let mut out: Vec<w::ServerMessage> = Vec::new();

    match &message.body {
        Some(Body::ChunkSubscribe(payload)) => {
            let added: Vec<sim_core::ids::ChunkCoord> = payload
                .coords
                .iter()
                .map(|c| sim_core::ids::ChunkCoord { x: c.x, y: c.y })
                .filter(|c| connection.subscription.insert(*c))
                .collect();

            if !added.is_empty() {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                // WS init snapshots come from the published read view below.
                // Dropping the receiver lets the tick loop apply subscriber
                // counts without rebuilding snapshots no caller will read.
                drop(reply_rx);
                if state
                    .mutations
                    .send(crate::runtime_view::Mutation::SubscriptionDiff {
                        added: added.clone(),
                        removed: Vec::new(),
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    // Mutation channel closed (tick task gone). Roll back the
                    // optimistic insert into connection.subscription so the
                    // disconnect cleanup doesn't try to un-subscribe chunks
                    // the runtime never registered.
                    for coord in &added {
                        connection.subscription.remove(coord);
                    }
                    return out;
                }

                // Set up per-chunk channels immediately so the next tick can
                // fan out deltas as soon as the mutation marks chunks active.
                let chunk_channels = state.chunk_channels();
                for coord in &added {
                    let sender = chunk_channels
                        .entry(*coord)
                        .or_insert_with(|| broadcast::channel(8).0)
                        .clone();
                    let receiver = sender.subscribe();
                    connection
                        .chunk_streams
                        .insert(*coord, BroadcastStream::new(receiver));
                }

                let view = state.view().load();
                for coord in &added {
                    if let Some(snap) = view.mobility_chunk_snapshots.get(coord).cloned() {
                        out.push(w::ServerMessage {
                            body: Some(w::server_message::Body::MobilityChunkSnapshot(snap)),
                        });
                    }
                }
            }
        }

        Some(Body::ChunkUnsubscribe(payload)) => {
            let removed: Vec<sim_core::ids::ChunkCoord> = payload
                .coords
                .iter()
                .map(|c| sim_core::ids::ChunkCoord { x: c.x, y: c.y })
                .filter(|c| connection.subscription.remove(c))
                .collect();

            if !removed.is_empty() {
                let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
                let _ = state
                    .mutations
                    .send(crate::runtime_view::Mutation::SubscriptionDiff {
                        added: Vec::new(),
                        removed: removed.clone(),
                        reply: reply_tx,
                    });

                let chunk_channels = state.chunk_channels();
                for coord in &removed {
                    connection.chunk_streams.remove(coord);
                    // View was published last tick — counts reflect the previous
                    // state. At worst we keep a chunk_channel for one extra tick
                    // before reaping; correctness unaffected.
                    let count = state
                        .view()
                        .load()
                        .chunk_subscriber_counts
                        .get(coord)
                        .copied()
                        .unwrap_or(0);
                    if count == 0 {
                        chunk_channels.remove(coord);
                    }
                }
            }
        }

        None => {}
    }

    out
}

async fn persist_snapshots_once(
    state: &AppState,
) -> Result<usize, sim_core::persistence::ChunkSnapshotStoreError> {
    // Phase 1: ask the tick task to collect everything we need. The reply
    // arrives at the next mutation-drain (≤ one tick interval).
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .mutations
        .send(crate::runtime_view::Mutation::CollectPersistData { reply: reply_tx })
        .is_err()
    {
        return Err(sim_core::persistence::ChunkSnapshotStoreError::unavailable(
            "tick task gone",
        ));
    }
    let payload = match reply_rx.await {
        Ok(p) => p,
        Err(_) => {
            return Err(sim_core::persistence::ChunkSnapshotStoreError::unavailable(
                "collect-persist-data reply dropped",
            ));
        }
    };

    let crate::runtime_view::PersistPayload {
        chunk_snapshots: snapshots,
        world_id,
        mobility_tick,
        mobility_world,
        economy_tick,
        economy_world,
        economy_audit_tick,
        economy_audit_pending,
    } = payload;
    let compatibility = sim_core::persistence::SnapshotCompatibility::new(
        state.base_world.world_id.clone(),
        state.base_world.schema_version,
    );
    let mobility_liveness = state.mobility_liveness();
    let mobility_attempt =
        mobility_liveness.begin_attempt(world_id.0.clone(), mobility_tick, SystemTime::now());

    let coords: Vec<ChunkCoord> = snapshots
        .iter()
        .map(|s| ChunkCoord {
            x: s.coord.x,
            y: s.coord.y,
        })
        .collect();
    let written = coords.len();

    // Phase 2a: chunk DB writes — store-mutex only, no runtime lock held.
    {
        let store = state.snapshot_store();
        let mut store = store.lock().await;
        for snapshot in snapshots {
            if let Err(error) = store.write_snapshot(snapshot, &compatibility).await {
                mobility_liveness.record_failure(
                    mobility_attempt,
                    error.to_string(),
                    SystemTime::now(),
                );
                return Err(error);
            }
        }
    }

    // Phase 2b: mobility DB write — store-mutex only, no runtime lock held.
    {
        if mobility_world.agents.is_empty() {
            let error = "refusing to persist empty mobility snapshot (0 agents)".to_string();
            mobility_liveness.record_failure(mobility_attempt, error.clone(), SystemTime::now());
            tracing::warn!(%error, "refusing to persist invalid mobility snapshot");
            return Ok(written);
        }

        let mob_store = state.mobility_snapshot_store();
        let mut mob_store = mob_store.lock().await;
        if let Err(error) = mob_store
            .write(&world_id.0, mobility_tick, &mobility_world, &compatibility)
            .await
        {
            mobility_liveness.record_failure(
                mobility_attempt,
                error.to_string(),
                SystemTime::now(),
            );
            tracing::warn!(%error, "failed to persist mobility snapshot");
        } else {
            mobility_liveness.record_success(mobility_attempt, SystemTime::now());
        }
    }

    // Phase 2c: economy DB write — store-mutex only, no runtime lock held.
    {
        let econ_store = state.economy_snapshot_store();
        let mut econ_store = econ_store.lock().await;
        if let Err(error) = econ_store
            .write(&world_id.0, economy_tick, &economy_world, &compatibility)
            .await
        {
            tracing::warn!(%error, "failed to persist economy snapshot");
        }
    }

    // Phase 2d: economy audit-log append — best-effort, store-mutex only. Only
    // the DURABLE subset is written (transient high-frequency mechanics stay
    // in-memory/snapshot-tail only — 2026-06-10 retention design), but the
    // commit advances the cursor past the FULL pending batch so transient
    // events are consumed and the live ledger still trims. A failed append
    // leaves the cursor untouched, so the same events retry next cycle.
    if !economy_audit_pending.is_empty() {
        let consumed = economy_audit_pending.len();
        let durable = sim_core::economy::durable_audit_subset(&economy_audit_pending);
        let append_result = if durable.is_empty() {
            Ok(())
        } else {
            let event_store = state.economy_event_store();
            let mut event_store = event_store.lock().await;
            event_store
                .append(&world_id.0, economy_audit_tick, &durable)
                .await
        };
        match append_result {
            Ok(()) => {
                let _ = state
                    .mutations
                    .send(crate::runtime_view::Mutation::CommitLedgerAudit { count: consumed });
            }
            Err(error) => {
                tracing::warn!(%error, "failed to append economy audit events");
            }
        }
    }

    // Phase 3: send a fire-and-forget Mutation to mark snapshots persisted.
    let _ = state
        .mutations
        .send(crate::runtime_view::Mutation::MarkChunkSnapshotsPersisted {
            coords: coords.clone(),
        });

    Ok(written)
}

async fn send_server_message(
    socket: &mut WebSocket,
    message: w::ServerMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bytes = message.encode_to_vec();
    socket.send(Message::Binary(bytes.into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
