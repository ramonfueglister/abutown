use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, ClientCommandDto, CommandAcceptedDto, HealthResponse,
    MobilitySnapshotDto, PROTOCOL_VERSION, ServerHelloDto, ServerMessageDto, SetTileKindCommandDto,
    TileKindSetEventDto, TilePulseDeltaDto, WorldEventDto, WorldId, WorldSummaryDto,
};
use sim_core::{
    base_world::BaseWorldBundle,
    chunk::{Chunk, ChunkError, EventApplyError, SnapshotDecodeError},
    events::{InMemoryWorldEventStore, WorldEventStore, WorldEventStoreError},
    ids::ChunkCoord,
    mobility::{
        MobilityPersistSnapshot, MobilityPlugin, api as mobility_api, apply_into_world,
        build_mobility_snapshot_dto, extract_from_world,
    },
    persistence::{
        ChunkSnapshotStore, ChunkSnapshotStoreError, EconomySnapshotStore, MobilitySnapshotStore,
        build_chunk_snapshot_from_parts,
    },
    scheduler::ChunkActivity,
    tile::TileKind,
    world::{
        components::{ChunkVersion, DirtyTiles, LastPersistedVersion, LastSnapshotAt, Tiles},
        plugin::CorePlugin,
        resources::ChunksByCoord,
        schedule::SimPlugin,
        systems::{TileMutationError, apply_set_tile_kind_ecs, chunk_snapshot_data},
    },
};

#[derive(Debug, thiserror::Error)]
pub enum HydrationError {
    #[error("snapshot store error: {0}")]
    Snapshot(ChunkSnapshotStoreError),
    #[error("event store error: {0}")]
    Events(WorldEventStoreError),
    #[error("snapshot decode error: {0}")]
    Decode(SnapshotDecodeError),
    #[error("event apply error: {0}")]
    Apply(EventApplyError),
    #[error("chunk error during seed: {0}")]
    Chunk(ChunkError),
    #[error("mobility store error: {0}")]
    Mobility(sim_core::persistence::MobilitySnapshotStoreError),
    #[error("economy store error: {0}")]
    Economy(sim_core::persistence::EconomySnapshotStoreError),
    #[error("mobility seed error: {0}")]
    Seed(sim_core::mobility::seed::SeedError),
}

use crate::commands::{AppliedCommand, CommandRejection};

const WORLD_ID: &str = "abutopia";
pub const BASE_WORLD_DEFAULT_PATH: &str = "data/worlds/abutopia";
const PULSE_STRIDE: u64 = 37;
pub const TICK_PERIOD_MS: u32 = 100;

pub const SEED_DENSITY: sim_core::mobility::seed::SeedDensity =
    sim_core::mobility::seed::SeedDensity {
        pedestrians_per_corridor: 6,
        cars_per_arterial: 17,
    };

mod base_world_expectations;
use base_world_expectations::*;
pub(crate) use base_world_expectations::{
    expected_base_world_agent_count, initial_mobility_snapshot_for_base_world,
    normalize_seeded_agent_birth_ticks,
};

pub fn default_base_world_path() -> std::path::PathBuf {
    std::env::var("ABUTOWN_BASE_WORLD_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .expect("sim-server crate lives under backend/crates/sim-server")
                .join(BASE_WORLD_DEFAULT_PATH)
        })
}

pub struct SimulationRuntime {
    world_id: WorldId,
    chunk_size: u16,
    pub(crate) world: sim_core::bevy_ecs::world::World,
    pub(crate) schedule: sim_core::bevy_ecs::schedule::Schedule,
    event_store: Box<dyn WorldEventStore + Send + Sync>,
    event_count: usize,
    tick: u64,
    version: u64,
}

fn refresh_flow_field_resources(world: &mut sim_core::bevy_ecs::world::World) {
    if let Some(mut cache) = world.get_resource_mut::<sim_core::routing::FlowFieldCache>() {
        cache.clear();
    } else {
        world.insert_resource(sim_core::routing::FlowFieldCache::default());
    }
}

fn pin_base_world_mobility_chunks(
    world: &mut sim_core::bevy_ecs::world::World,
    base_world: &BaseWorldBundle,
) {
    let chunk_size = base_world.chunk_size();
    let mut pins = std::collections::HashSet::new();

    for group in &base_world.spawns.pedestrian_groups {
        if let Some(corridor) = base_world
            .transport
            .pedestrian_corridors
            .iter()
            .find(|path| path.id == group.corridor_id)
        {
            pins.extend(
                corridor
                    .points
                    .iter()
                    .map(|point| sim_core::mobility::chunk_of(point.x, point.y, chunk_size)),
            );
        }
    }

    for group in &base_world.spawns.car_groups {
        if let Some(arterial) = base_world
            .transport
            .arterial_paths
            .iter()
            .find(|path| path.id == group.arterial_id)
        {
            pins.extend(
                arterial
                    .points
                    .iter()
                    .map(|point| sim_core::mobility::chunk_of(point.x, point.y, chunk_size)),
            );
        }
    }

    world
        .resource_mut::<sim_core::world::resources::PinnedActiveChunks>()
        .0
        .extend(pins);
}

impl std::fmt::Debug for SimulationRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulationRuntime")
            .field("world_id", &self.world_id)
            .field("chunk_size", &self.chunk_size)
            .field("tick", &self.tick)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl SimulationRuntime {
    pub fn new() -> Self {
        Self::new_with_event_store(Box::new(InMemoryWorldEventStore::default()))
    }

    pub fn default_world_id() -> WorldId {
        WorldId(WORLD_ID.to_string())
    }

    pub fn new_with_event_store(event_store: Box<dyn WorldEventStore + Send + Sync>) -> Self {
        Self::new_from_base_world_dir_with_event_store(default_base_world_path(), event_store)
            .expect("base world bundle is required for runtime startup")
    }

    pub fn new_from_base_world_dir(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        Self::new_from_base_world_dir_with_event_store(
            path,
            Box::new(InMemoryWorldEventStore::default()),
        )
    }

    pub fn new_from_base_world(bundle: BaseWorldBundle) -> anyhow::Result<Self> {
        Self::new_with_event_store_and_base_world(
            Box::new(InMemoryWorldEventStore::default()),
            bundle,
        )
    }

    pub fn new_from_base_world_dir_with_event_store(
        path: impl AsRef<std::path::Path>,
        event_store: Box<dyn WorldEventStore + Send + Sync>,
    ) -> anyhow::Result<Self> {
        let bundle = BaseWorldBundle::load_from_dir(path)?;
        Self::new_with_event_store_and_base_world(event_store, bundle)
    }

    pub fn new_with_event_store_and_base_world(
        event_store: Box<dyn WorldEventStore + Send + Sync>,
        bundle: BaseWorldBundle,
    ) -> anyhow::Result<Self> {
        let mut world = sim_core::bevy_ecs::world::World::new();
        let mut schedule = sim_core::bevy_ecs::schedule::Schedule::default();
        let seeded_stops = Vec::new();
        let seeded_walks = sim_core::mobility::seed::seeded_walks_from_base_world(&bundle);
        let city_network = bundle.to_city_network();
        world.insert_resource(city_network);

        CorePlugin::default().install(&mut world, &mut schedule);
        sim_core::time::TimePlugin.install(&mut world, &mut schedule);

        sim_core::routing::RoutingPlugin {
            seeded_stops,
            seeded_walks,
        }
        .install(&mut world, &mut schedule);

        sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::FlowFieldPlugin::default().install(&mut world, &mut schedule);

        MobilityPlugin.install(&mut world, &mut schedule);
        sim_core::mobility::seed::insert_activity_waypoints_from_base_world(&mut world, &bundle)?;
        sim_core::economy::EconomyPlugin.install(&mut world, &mut schedule);
        sim_core::population::PopulationPlugin.install(&mut world, &mut schedule);
        // Population carrying capacity = base-world seed count, re-applied every boot
        // (PopulationConfig is not persisted). `resource_mut` (not `get_resource_mut`) so a
        // broken PopulationPlugin-install contract surfaces as a panic rather than silently
        // reverting to unbounded growth.
        world
            .resource_mut::<sim_core::population::PopulationConfig>()
            .carrying_capacity = expected_base_world_agent_count(&bundle) as f32;
        crate::persistence_plugin::PersistencePlugin {
            world_id: bundle.world_id().to_owned(),
        }
        .install(&mut world, &mut schedule);

        bundle.spawn_all_chunks(&mut world, 0);
        let mobility_snap = initial_mobility_snapshot_for_base_world(&bundle)?;
        apply_into_world(&mut world, mobility_snap);
        pin_base_world_mobility_chunks(&mut world, &bundle);

        // Seed the economy from the authored markets layer AFTER `apply_into_world`
        // so each `MarketSite`'s node_id resolves against the graph apply installs
        // (seeding earlier leaves stale node_ids — see `hydrate_from_stores`), then
        // rebind the agents apply just spawned (each is `home_market = 0` until the
        // markets exist).
        sim_core::economy::seed_from_markets_layer(&mut world, &bundle.markets);
        sim_core::mobility::api::rebind_unassigned_market_agents(&mut world);

        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);
        refresh_flow_field_resources(&mut world);

        Ok(Self {
            world_id: WorldId(bundle.world_id().to_owned()),
            chunk_size: bundle.chunk_size(),
            world,
            schedule,
            event_store,
            event_count: 0,
            tick: 0,
            version: 0,
        })
    }

    /// Build an in-memory runtime whose mobility world is seeded from the
    /// shared city network descriptor instead of the tiny developer seed.
    pub fn new_from_network(network: &sim_core::city_network::CityNetwork) -> Self {
        let mut runtime = Self::new();
        let (seeded_world, _) = sim_core::mobility::seed::from_network(network, SEED_DENSITY);
        let snap = extract_from_world(&seeded_world);
        apply_into_world(&mut runtime.world, snap);
        sim_core::routing::HierarchicalRoutingPlugin::default()
            .install(&mut runtime.world, &mut runtime.schedule);
        refresh_flow_field_resources(&mut runtime.world);
        runtime
    }

    /// Test-only helper: replace the runtime's mobility state with the state
    /// extracted from `(world, _schedule)` produced by `seed::*`. Chunk
    /// entities already in the runtime are preserved.
    pub fn set_mobility_for_test(
        &mut self,
        seeded: (
            sim_core::bevy_ecs::world::World,
            sim_core::bevy_ecs::schedule::Schedule,
        ),
    ) {
        let (seeded_world, _schedule) = seeded;
        let snap = extract_from_world(&seeded_world);
        apply_into_world(&mut self.world, snap);
        sim_core::routing::HierarchicalRoutingPlugin::default()
            .install(&mut self.world, &mut self.schedule);
        refresh_flow_field_resources(&mut self.world);
    }

    pub fn override_world_id_for_test(&mut self, world_id: &str) {
        self.world_id = WorldId(world_id.to_string());
    }

    /// Advance the mobility world by one tick (discards the per-chunk delta).
    pub fn advance_mobility_tick_for_test(&mut self) {
        let _ = mobility_api::tick_mobility(&mut self.world, &mut self.schedule);
    }

    /// Append events to the live trade ledger so a test can drive the audit
    /// flush with deterministic, identifiable events instead of relying on the
    /// economy systems to produce them.
    pub fn push_ledger_events_for_test(&mut self, events: Vec<sim_core::economy::EconomyEvent>) {
        self.world
            .resource_mut::<sim_core::economy::TradeLedger>()
            .0
            .extend(events);
    }

    /// Snapshot of mobility state (for persist callers and tests).
    pub fn mobility_snapshot_for_persist(&self) -> MobilityPersistSnapshot {
        extract_from_world(&self.world)
    }

    pub fn mobility_tick(&self) -> u64 {
        mobility_api::tick(&self.world)
    }

    /// Hydrate a runtime from the given stores.
    ///
    /// Returns `(runtime, snapshot_store, mobility_snapshot_store,
    /// economy_snapshot_store)` so the caller (AppState) can place the stores
    /// under its own `Arc<Mutex<…>>`.
    pub async fn hydrate_from_stores(
        event_store: Box<dyn WorldEventStore + Send + Sync>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send + Sync>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send + Sync>,
        economy_snapshot_store: Box<dyn EconomySnapshotStore + Send + Sync>,
        base_world: &BaseWorldBundle,
    ) -> Result<
        (
            Self,
            Box<dyn ChunkSnapshotStore + Send + Sync>,
            Box<dyn MobilitySnapshotStore + Send + Sync>,
            Box<dyn EconomySnapshotStore + Send + Sync>,
        ),
        HydrationError,
    > {
        let world_id = WorldId(base_world.world_id().to_owned());
        let network = base_world.to_city_network();
        let snapshot_compatibility = base_world.snapshot_compatibility();

        // Build a fresh World + Schedule and install both plugins.
        let mut world = sim_core::bevy_ecs::world::World::new();
        let mut schedule = sim_core::bevy_ecs::schedule::Schedule::default();

        let seeded_stops = Vec::new();
        let seeded_walks = sim_core::mobility::seed::seeded_walks_from_base_world(base_world);

        // Insert city network as resource before plugins run.
        world.insert_resource(network);

        CorePlugin::default().install(&mut world, &mut schedule);
        sim_core::time::TimePlugin.install(&mut world, &mut schedule);

        sim_core::routing::RoutingPlugin {
            seeded_stops,
            seeded_walks,
        }
        .install(&mut world, &mut schedule);

        sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::FlowFieldPlugin::default().install(&mut world, &mut schedule);

        MobilityPlugin.install(&mut world, &mut schedule);
        sim_core::mobility::seed::insert_activity_waypoints_from_base_world(&mut world, base_world)
            .map_err(HydrationError::Seed)?;
        sim_core::economy::EconomyPlugin.install(&mut world, &mut schedule);
        sim_core::population::PopulationPlugin.install(&mut world, &mut schedule);
        // Population carrying capacity = base-world seed count, re-applied every boot
        // (PopulationConfig is not persisted). `resource_mut` (not `get_resource_mut`) so a
        // broken PopulationPlugin-install contract surfaces as a panic rather than silently
        // reverting to unbounded growth.
        world
            .resource_mut::<sim_core::population::PopulationConfig>()
            .carrying_capacity = expected_base_world_agent_count(base_world) as f32;
        crate::persistence_plugin::PersistencePlugin {
            world_id: world_id.0.clone(),
        }
        .install(&mut world, &mut schedule);

        // Hydrate mobility state from a current base-world snapshot if present;
        // otherwise initialize from the canonical base world.
        let mobility_snap = match mobility_snapshot_store
            .read(&world_id.0, &snapshot_compatibility)
            .await
            .map_err(HydrationError::Mobility)?
        {
            Some((tick, mut snap)) if mobility_snapshot_matches_base_world(&snap, base_world) => {
                tracing::info!(tick, "resuming mobility from persisted snapshot");
                normalize_seeded_agent_birth_ticks(&mut snap, base_world);
                snap
            }
            None => initial_mobility_snapshot_for_base_world(base_world)
                .map_err(HydrationError::Seed)?,
            // Discarding a present-but-mismatched snapshot resets the world to
            // tick 0 — that must never happen silently (the static-era check
            // did exactly that on every restart, unnoticed for days).
            Some((tick, snap)) => {
                tracing::warn!(
                    tick,
                    agents = snap.agents.len(),
                    vehicles = snap.vehicles.len(),
                    "persisted mobility snapshot does not match this base-world generation — RESEEDING world at tick 0"
                );
                initial_mobility_snapshot_for_base_world(base_world)
                    .map_err(HydrationError::Seed)?
            }
        };

        apply_into_world(&mut world, mobility_snap);
        pin_base_world_mobility_chunks(&mut world, base_world);

        // Restore the economy from a current base-world snapshot if present.
        if let Some((_tick, econ_snap)) = economy_snapshot_store
            .read(&world_id.0, &snapshot_compatibility)
            .await
            .map_err(HydrationError::Economy)?
        {
            sim_core::economy::apply_into_world(&mut world, &econ_snap);
        }

        // Seed the economy from the authored markets layer AFTER `apply_into_world`:
        // each `MarketSite` stores a graph `node_id`, and the authoritative routing
        // graph is the one `apply_into_world` installs from the mobility snapshot.
        // Seeding against the pre-apply plugin graph would leave stale node_ids whose
        // positions scramble once the snapshot graph replaces them (markets resolve
        // to the wrong tiles). Idempotent: no-ops when an economy was already
        // restored from a snapshot above.
        sim_core::economy::seed_from_markets_layer(&mut world, &base_world.markets);

        // `apply_into_world` spawns the seeded agents BEFORE any market exists, so
        // each freezes at `home_market = 0` (unbound); with markets now seeded
        // (against the correct graph), rebind them — otherwise economy attribution
        // routes nobody (`routed=0`).
        sim_core::mobility::api::rebind_unassigned_market_agents(&mut world);

        // Treat the restored ledger tail as already durably appended so the audit
        // flush only persists events produced after this boot. Must run after the
        // economy restore + seed (which finalize the ledger length) and after
        // EconomyPlugin install (which inserted the cursor at its default 0).
        sim_core::economy::init_ledger_audit_cursor(&mut world);

        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);
        refresh_flow_field_resources(&mut world);

        for coord in base_world.chunk_coords() {
            let snap = snapshot_store
                .read_snapshot(coord, &snapshot_compatibility)
                .await
                .map_err(HydrationError::Snapshot)?;

            let (mut chunk, mut chunk_version, activity) = match snap {
                Some(snapshot) => {
                    let version = snapshot.chunk_version;
                    let activity = ChunkActivity::from(snapshot.chunk_state);
                    let chunk = Chunk::from_snapshot(&snapshot).map_err(HydrationError::Decode)?;
                    (chunk, version, activity)
                }
                None => {
                    let chunk = Chunk::from_records(
                        coord,
                        base_world.chunk_size(),
                        base_world.tiles_for_chunk(coord, 0),
                        0,
                    )
                    .map_err(HydrationError::Chunk)?;
                    (chunk, 0, ChunkActivity::Warm)
                }
            };

            let events = event_store
                .read_chunk_events_since(
                    &world_id.0,
                    ChunkCoordDto {
                        x: coord.x,
                        y: coord.y,
                    },
                    chunk_version,
                )
                .await
                .map_err(HydrationError::Events)?;

            for event in &events {
                let next_version = chunk_version + 1;
                chunk
                    .apply_event(event, next_version)
                    .map_err(HydrationError::Apply)?;
                chunk_version = next_version;
            }

            // Materialize the chunk's tile vec then spawn a chunk entity in
            // the ECS world. The Chunk value is consumed here; the entity is
            // the sole source of truth thereafter.
            let tiles: Vec<sim_core::tile::TileRecord> = (0..chunk.tile_count())
                .filter_map(|i| chunk.tile_at(i))
                .collect();
            sim_core::world::systems::spawn_chunk_entity(
                &mut world,
                coord,
                base_world.chunk_size(),
                tiles,
                chunk_version,
                activity,
            );
        }

        let global_tick = event_store
            .max_tick(&world_id.0)
            .await
            .map_err(HydrationError::Events)?
            .unwrap_or(0);
        let global_version = event_store
            .max_version(&world_id.0)
            .await
            .map_err(HydrationError::Events)?
            .unwrap_or(0);
        // event_count is bootstrapped from version because today they advance 1:1;
        // revisit if version bumps ever decouple from event appends.
        let event_count = global_version as usize;

        let runtime = Self {
            world_id,
            chunk_size: base_world.chunk_size(),
            world,
            schedule,
            event_store,
            event_count,
            tick: global_tick,
            version: global_version,
        };
        Ok((
            runtime,
            snapshot_store,
            mobility_snapshot_store,
            economy_snapshot_store,
        ))
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: self.world_id.clone(),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
            persistence: None,
        }
    }

    pub fn world_summary(&self) -> WorldSummaryDto {
        let current_tick = mobility_api::tick(&self.world);
        let sim_time = self
            .world
            .resource::<sim_core::time::SimClock>()
            .sim_seconds(current_tick);
        WorldSummaryDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            chunk_size: self.chunk_size,
            loaded_chunks: self
                .loaded_coords()
                .into_iter()
                .map(ChunkCoordDto::from)
                .collect(),
            tick_period_ms: TICK_PERIOD_MS,
            sim_time,
        }
    }

    /// Current `ChunkVersion` for a loaded chunk — the cheap dirtiness probe
    /// the read view uses to decide whether a cached tile snapshot can be
    /// reused (version unchanged) or must be rebuilt.
    pub fn chunk_version(&self, coord: ChunkCoord) -> Option<u64> {
        let entity = *self.world.resource::<ChunksByCoord>().0.get(&coord)?;
        Some(self.world.get::<ChunkVersion>(entity)?.0)
    }

    pub fn chunk_snapshot(&self, coord: ChunkCoord) -> Option<ChunkSnapshotDto> {
        let (_chunk_size, version, tiles, activity) = chunk_snapshot_data(&self.world, coord)?;
        Some(build_chunk_snapshot_from_parts(
            &self.world_id.0,
            coord,
            &tiles,
            version,
            activity,
        ))
    }

    /// Deterministically sorted list (`(y, x)`) of loaded chunk coords from
    /// the ECS world.
    ///
    /// Excludes "stub" chunk entities — empty-tiles entities spawned solely
    /// to track WS subscriptions or LOD activity for chunks that the
    /// persistence layer hasn't loaded yet. Those chunks have no terrain
    /// payload, so the world-summary + pulse rotation must skip them.
    fn loaded_coords(&self) -> Vec<ChunkCoord> {
        let by_coord = self.world.resource::<ChunksByCoord>();
        let mut coords: Vec<ChunkCoord> = by_coord
            .0
            .iter()
            .filter_map(|(coord, entity)| {
                let tile_count = self.world.get::<Tiles>(*entity)?.0.len();
                if tile_count > 0 { Some(*coord) } else { None }
            })
            .collect();
        coords.sort_by_key(|c| (c.y, c.x));
        coords
    }

    /// Read-only ECS world view.
    pub fn world_view(&self) -> &sim_core::bevy_ecs::world::World {
        &self.world
    }

    pub fn mobility_snapshot(&self) -> MobilitySnapshotDto {
        build_mobility_snapshot_dto(&self.world_id, self.mobility_tick(), &self.world)
    }

    /// Forward a per-connection chunk-subscription delta into the
    /// `ChunkSubscribers` resource.
    pub fn apply_subscription_diff<'a, A, R>(&mut self, added: A, removed: R)
    where
        A: IntoIterator<Item = &'a sim_core::ids::ChunkCoord>,
        R: IntoIterator<Item = &'a sim_core::ids::ChunkCoord>,
    {
        mobility_api::apply_subscription_diff(&mut self.world, added, removed);
    }

    /// Expose the per-chunk tick result for the new fan-out path (Task 7).
    pub fn tick_world_mobility(
        &mut self,
    ) -> std::collections::HashMap<sim_core::ids::ChunkCoord, sim_core::mobility::MobilityChunkDelta>
    {
        mobility_api::tick_mobility(&mut self.world, &mut self.schedule)
    }

    /// Collect all chunk snapshots that are due for persistence.
    pub fn collect_chunk_snapshots(&self) -> Vec<ChunkSnapshotDto> {
        let ceiling = std::time::Duration::from_secs(30);
        let now = std::time::Instant::now();
        let world = &self.world;
        let by_coord = world.resource::<ChunksByCoord>();
        let mut due: Vec<ChunkCoord> = by_coord
            .0
            .iter()
            .filter_map(|(coord, entity)| {
                let version = world.get::<ChunkVersion>(*entity)?.0;
                let last_persisted = world.get::<LastPersistedVersion>(*entity)?.0;
                let last_at = world.get::<LastSnapshotAt>(*entity)?.0;
                let is_due = version > last_persisted || now.duration_since(last_at) >= ceiling;
                if is_due { Some(*coord) } else { None }
            })
            .collect();
        due.sort_by_key(|c| (c.y, c.x));
        due.into_iter()
            .filter_map(|coord| self.chunk_snapshot(coord))
            .collect()
    }

    /// Mark the given chunks as persisted (clear dirty state, update timestamps).
    pub fn mark_chunk_snapshots_persisted(&mut self, coords: &[ChunkCoord]) {
        let now = std::time::Instant::now();
        let world = &mut self.world;
        // Snapshot (entity, current_version) pairs first to release the &World
        // borrow before we take entity_mut for each.
        let updates: Vec<(sim_core::bevy_ecs::entity::Entity, u64)> = {
            let by_coord = world.resource::<ChunksByCoord>();
            coords
                .iter()
                .filter_map(|coord| {
                    let entity = *by_coord.0.get(coord)?;
                    let version = world.get::<ChunkVersion>(entity)?.0;
                    Some((entity, version))
                })
                .collect()
        };
        for (entity, version) in updates {
            let mut ent = world.entity_mut(entity);
            if let Some(mut last) = ent.get_mut::<LastPersistedVersion>() {
                last.0 = version;
            }
            if let Some(mut at) = ent.get_mut::<LastSnapshotAt>() {
                at.0 = now;
            }
            if let Some(mut dirty) = ent.get_mut::<DirtyTiles>() {
                dirty.0.clear();
            }
        }
    }

    /// Extract a persist-snapshot of mobility state so persist functions can
    /// release the runtime read-lock before performing DB writes.
    pub fn mobility_persist_snapshot(&self) -> MobilityPersistSnapshot {
        extract_from_world(&self.world)
    }

    /// Live economy snapshot for the debug endpoint.
    pub fn economy_snapshot(&self) -> sim_core::economy::EconomyPersistSnapshot {
        sim_core::economy::extract_from_world(&self.world)
    }

    /// The current tick plus the un-appended tail of the trade ledger, for the
    /// persist loop's audit flush. Non-mutating: the cursor only advances once the
    /// durable append succeeds (`commit_ledger_audit`), so a failed flush retries
    /// the same events next cycle.
    pub fn pending_ledger_audit(&self) -> (u64, Vec<sim_core::economy::EconomyEvent>) {
        sim_core::economy::pending_ledger_audit(&self.world)
    }

    /// Acknowledge a successful audit append of `appended` events: advance the
    /// cursor and bound the live ledger to its persisted tail.
    pub fn commit_ledger_audit(&mut self, appended: usize) {
        sim_core::economy::commit_ledger_audit(&mut self.world, appended);
    }

    /// Collect snapshot items from every registered `SnapshotProvider`.
    ///
    /// This is the source-of-truth collection path post-Phase-8a: instead of
    /// dual-purpose helpers like `collect_chunk_snapshots()` +
    /// `mobility_persist_snapshot()`, the persist loop iterates the items
    /// returned here and dispatches by `key.kind` to the matching store.
    /// Items are already serialized — the storage layer deserializes only
    /// when it needs to inspect the DTO shape.
    pub fn collect_provider_items(&self) -> Vec<sim_core::world::persistence::SnapshotItem> {
        let providers = self
            .world
            .resource::<sim_core::world::persistence::SnapshotProviders>();
        let mut items = Vec::new();
        for provider in &providers.0 {
            items.extend(provider.collect(&self.world));
        }
        items
    }

    /// Borrow the runtime's bevy world — for callers that want to issue
    /// `mobility::api` reads without paying the snapshot-extract cost.
    pub fn mobility(&self) -> &sim_core::bevy_ecs::world::World {
        &self.world
    }

    /// Return the number of active WS subscribers for a chunk.
    pub fn chunk_subscriber_count(&self, chunk: sim_core::ids::ChunkCoord) -> u8 {
        mobility_api::chunk_subscriber_count(&self.world, chunk)
    }

    /// Return the world ID for use by persist functions outside the runtime lock.
    pub fn world_id_for_persist(&self) -> &WorldId {
        &self.world_id
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    pub async fn apply_client_command(
        &mut self,
        command: ClientCommandDto,
    ) -> Result<AppliedCommand, CommandRejection> {
        match command {
            ClientCommandDto::SetTileKind(command) => self.apply_set_tile_kind(command).await,
        }
    }

    fn build_accepted(&self, command_id: String, event: WorldEventDto) -> AppliedCommand {
        let response = CommandAcceptedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            command_id,
            event: event.clone(),
        };
        AppliedCommand { response, event }
    }

    async fn apply_set_tile_kind(
        &mut self,
        command: SetTileKindCommandDto,
    ) -> Result<AppliedCommand, CommandRejection> {
        if command.protocol_version != PROTOCOL_VERSION {
            return Err(CommandRejection {
                world_id: Some(command.world_id),
                command_id: Some(command.command_id),
                code: "protocol_mismatch",
                message: format!(
                    "protocol version {} is not supported by server version {}",
                    command.protocol_version, PROTOCOL_VERSION
                ),
            });
        }

        if command.world_id != self.world_id {
            return Err(CommandRejection {
                world_id: Some(command.world_id),
                command_id: Some(command.command_id),
                code: "wrong_world",
                message: format!("command targets a different world than {}", self.world_id.0),
            });
        }

        match self
            .event_store
            .find_event_by_command(&self.world_id.0, &command.command_id)
            .await
        {
            Ok(Some(existing_event)) => {
                return Ok(self.build_accepted(command.command_id.clone(), existing_event));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(CommandRejection {
                    world_id: Some(command.world_id),
                    command_id: Some(command.command_id),
                    code: error.code(),
                    message: error.to_string(),
                });
            }
        }

        let coord = ChunkCoord {
            x: command.coord.x,
            y: command.coord.y,
        };
        let kind = TileKind::from(command.kind);

        // Pre-flight validation against ECS state (no mutation yet — we only
        // commit after the event store accepts the append).
        let (preview_version, _existing_kind) = {
            let world = &self.world;
            let entity = world
                .resource::<ChunksByCoord>()
                .0
                .get(&coord)
                .copied()
                .ok_or_else(|| CommandRejection {
                    world_id: Some(command.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "chunk_not_loaded",
                    message: format!("chunk {}:{} is not loaded", coord.x, coord.y),
                })?;
            let tiles = world.get::<Tiles>(entity).expect("Tiles on chunk entity");
            let tile_count = tiles.0.len() as u32;
            if command.local_index as u32 >= tile_count {
                return Err(CommandRejection {
                    world_id: Some(command.world_id),
                    command_id: Some(command.command_id),
                    code: "tile_out_of_bounds",
                    message: format!(
                        "tile index {} is outside chunk tile count {}",
                        command.local_index, tile_count
                    ),
                });
            }
            let existing_kind = tiles.0[command.local_index as usize].kind;
            if existing_kind == kind {
                return Err(CommandRejection {
                    world_id: Some(command.world_id),
                    command_id: Some(command.command_id),
                    code: "no_state_change",
                    message: format!(
                        "tile {} in chunk {}:{} already has the requested kind",
                        command.local_index, coord.x, coord.y
                    ),
                });
            }
            let version = world
                .get::<ChunkVersion>(entity)
                .expect("ChunkVersion on chunk entity")
                .0;
            (version + 1, existing_kind)
        };

        let event_id = format!("event:{}", uuid::Uuid::now_v7());
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id,
            command_id: command.command_id.clone(),
            world_id: self.world_id.clone(),
            tick: self.tick,
            version: preview_version,
            coord: command.coord,
            local_index: command.local_index,
            kind: command.kind,
        });
        match self.event_store.append(event.clone()).await {
            Ok(()) => {}
            Err(error) if error.code() == "duplicate_command_id" => {
                let winner = self
                    .event_store
                    .find_event_by_command(&self.world_id.0, &command.command_id)
                    .await
                    .map_err(|error| CommandRejection {
                        world_id: Some(self.world_id.clone()),
                        command_id: Some(command.command_id.clone()),
                        code: error.code(),
                        message: error.to_string(),
                    })?
                    .ok_or_else(|| CommandRejection {
                        world_id: Some(self.world_id.clone()),
                        command_id: Some(command.command_id.clone()),
                        code: "event_store_inconsistent",
                        message: "duplicate command_id reported but lookup returned none"
                            .to_string(),
                    })?;
                return Ok(self.build_accepted(command.command_id.clone(), winner));
            }
            Err(error) => {
                return Err(CommandRejection {
                    world_id: Some(self.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: error.code(),
                    message: error.to_string(),
                });
            }
        }

        self.event_count += 1;
        let mutation_result =
            apply_set_tile_kind_ecs(&mut self.world, coord, command.local_index, kind, self.tick)
                .map_err(|error| match error {
                TileMutationError::ChunkNotLoaded { coord } => CommandRejection {
                    world_id: Some(self.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "chunk_not_loaded",
                    message: format!("chunk {}:{} is not loaded", coord.x, coord.y),
                },
                TileMutationError::TileOutOfBounds { index, tile_count } => CommandRejection {
                    world_id: Some(self.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "tile_out_of_bounds",
                    message: format!("tile index {index} is outside chunk tile count {tile_count}"),
                },
                TileMutationError::NoStateChange {
                    coord, local_index, ..
                } => CommandRejection {
                    world_id: Some(self.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "no_state_change",
                    message: format!(
                        "tile {local_index} in chunk {}:{} already has the requested kind",
                        coord.x, coord.y
                    ),
                },
            })?;
        debug_assert_eq!(mutation_result.new_version, preview_version);

        Ok(self.build_accepted(command.command_id, event))
    }

    pub fn hello(&self) -> ServerMessageDto {
        ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            chunk_size: self.chunk_size,
        })
    }

    pub fn next_pulse(&mut self) -> ServerMessageDto {
        self.tick += 1;
        self.version += 1;
        let loaded_coords = self.loaded_coords();
        assert!(
            !loaded_coords.is_empty(),
            "next_pulse called on a runtime with no loaded chunks — \
             callers must seed or hydrate at least one chunk first",
        );
        let coord = loaded_coords[((self.tick - 1) as usize) % loaded_coords.len()];
        let tile_count = {
            let world = &self.world;
            let entity = world
                .resource::<ChunksByCoord>()
                .0
                .get(&coord)
                .copied()
                .expect("pulse chunk should be loaded");
            world
                .get::<Tiles>(entity)
                .expect("Tiles on chunk entity")
                .0
                .len() as u64
        };
        let local_index = ((self.tick * PULSE_STRIDE) % tile_count) as u16;

        ServerMessageDto::TilePulse(TilePulseDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            tick: self.tick,
            version: self.version,
            coord: coord.into(),
            local_index,
        })
    }
}

impl Default for SimulationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl SimulationRuntime {
    pub fn mobility_tick_for_test(&self) -> u64 {
        mobility_api::tick(&self.world)
    }
    pub fn mobility_agent_count_for_test(&self) -> usize {
        mobility_api::agents(&self.world).len()
    }
    pub fn mobility_vehicle_count_for_test(&self) -> usize {
        mobility_api::vehicles(&self.world).len()
    }
}

#[cfg(test)]
mod tests;
