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
        ChunkSnapshotStore, ChunkSnapshotStoreError, MobilitySnapshotStore,
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

fn initial_mobility_snapshot_for_base_world(
    bundle: &BaseWorldBundle,
) -> Result<MobilityPersistSnapshot, sim_core::mobility::seed::SeedError> {
    let (seeded_world, _) = sim_core::mobility::seed::from_base_world_bundle(bundle)?;
    Ok(extract_from_world(&seeded_world))
}

fn mobility_snapshot_matches_base_world(
    snapshot: &MobilityPersistSnapshot,
    base_world: &BaseWorldBundle,
) -> bool {
    let expected_cars = expected_base_world_car_routes(base_world);
    snapshot.vehicles.len() == expected_cars.len()
        && snapshot.vehicles.values().all(|vehicle| {
            vehicle.kind == sim_core::mobility::VehicleKind::Car
                && expected_cars
                    .get(&vehicle.id.0)
                    .is_some_and(|route_id| route_id == &vehicle.route_id)
        })
}

fn expected_base_world_car_routes(
    base_world: &BaseWorldBundle,
) -> std::collections::HashMap<String, String> {
    let mut expected = std::collections::HashMap::new();
    for group in &base_world.spawns.car_groups {
        let arterial_index = base_world
            .transport
            .arterial_paths
            .iter()
            .position(|path| path.id == group.arterial_id)
            .unwrap_or_else(|| {
                panic!(
                    "base world car group {} references missing arterial {}",
                    group.id, group.arterial_id
                )
            });
        let route_id = format!("route:arterial:{arterial_index}");
        for n in 0..group.cars_per_arterial {
            expected.insert(
                format!("vehicle:car:{arterial_index}:{n}"),
                route_id.clone(),
            );
        }
    }
    expected
}

#[cfg(test)]
fn expected_base_world_car_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_car_routes(base_world).len()
}

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
        let city_network = bundle.to_city_network();
        let seeded_stops = Vec::new();
        let seeded_walks = sim_core::mobility::seed::seeded_walks_from_network(&city_network);
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
        crate::persistence_plugin::PersistencePlugin {
            world_id: bundle.world_id().to_owned(),
        }
        .install(&mut world, &mut schedule);

        bundle.spawn_all_chunks(&mut world, 0);
        let mobility_snap = initial_mobility_snapshot_for_base_world(&bundle)?;
        apply_into_world(&mut world, mobility_snap);
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

    /// Snapshot of mobility state (for persist callers and tests).
    pub fn mobility_snapshot_for_persist(&self) -> MobilityPersistSnapshot {
        extract_from_world(&self.world)
    }

    pub fn mobility_tick(&self) -> u64 {
        mobility_api::tick(&self.world)
    }

    /// Hydrate a runtime from the given stores.
    ///
    /// Returns `(runtime, snapshot_store, mobility_snapshot_store)` so the
    /// caller (AppState) can place the stores under its own `Arc<Mutex<…>>`.
    pub async fn hydrate_from_stores(
        event_store: Box<dyn WorldEventStore + Send + Sync>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send + Sync>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send + Sync>,
        base_world: &BaseWorldBundle,
    ) -> Result<
        (
            Self,
            Box<dyn ChunkSnapshotStore + Send + Sync>,
            Box<dyn MobilitySnapshotStore + Send + Sync>,
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
        let seeded_walks = sim_core::mobility::seed::seeded_walks_from_network(&network);

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
            Some((_tick, snap)) if mobility_snapshot_matches_base_world(&snap, base_world) => snap,
            None => initial_mobility_snapshot_for_base_world(base_world)
                .map_err(HydrationError::Seed)?,
            Some((_tick, _snap)) => initial_mobility_snapshot_for_base_world(base_world)
                .map_err(HydrationError::Seed)?,
        };
        apply_into_world(&mut world, mobility_snap);
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
        Ok((runtime, snapshot_store, mobility_snapshot_store))
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: self.world_id.clone(),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        }
    }

    pub fn world_summary(&self) -> WorldSummaryDto {
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
        }
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
mod tests {
    use super::*;
    use abutown_protocol::{ChunkStateDto, TileKindDto};

    fn populated_flow_field_cache() -> sim_core::routing::FlowFieldCache {
        use sim_core::routing::{
            Edge, EdgeId, EdgeKind, FlowFieldCache, FlowFieldCacheKey, FlowFieldScope, Graph, Node,
            NodeId, NodeKind, RoutingProfile, RoutingProfileKey,
        };

        let graph = Graph::new(
            vec![
                Node {
                    id: NodeId(0),
                    position: (0.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(1),
                    position: (1.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
            ],
            vec![
                Edge {
                    id: EdgeId(0),
                    from: NodeId(0),
                    to: NodeId(1),
                    polyline: vec![(0.0, 0.0), (1.0, 0.0)],
                    length: 1.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: None,
                },
                Edge {
                    id: EdgeId(1),
                    from: NodeId(1),
                    to: NodeId(0),
                    polyline: vec![(1.0, 0.0), (0.0, 0.0)],
                    length: 1.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: None,
                },
            ],
        );
        let mut cache = FlowFieldCache::with_capacity(2);
        cache
            .get_or_build(
                &graph,
                FlowFieldCacheKey::all_edges(NodeId(1), RoutingProfileKey::Walk, 0),
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("test flow field should build");
        assert_eq!(cache.len(), 1);
        cache
    }

    #[test]
    fn simulation_runtime_holds_world_directly() {
        let runtime = SimulationRuntime::new();
        // After Task 9 dissolved MobilityWorld, SimulationRuntime owns the
        // shared bevy World + Schedule directly.
        let _world: &sim_core::bevy_ecs::world::World = &runtime.world;
        let _schedule: &sim_core::bevy_ecs::schedule::Schedule = &runtime.schedule;
    }

    #[test]
    fn runtime_materializes_base_world_instead_of_demo_chunks() {
        let fixture_root = workspace_root().join("data/worlds/abutopia");
        let runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
            .expect("base world fixture must load");
        let summary = runtime.world_summary();

        assert_eq!(summary.world_id.0, "abutopia");
        assert_eq!(summary.chunk_size, 32);
        assert_eq!(summary.loaded_chunks, vec![ChunkCoordDto { x: 0, y: 0 }]);
    }

    #[test]
    fn runtime_seeds_backend_pedestrian_from_base_world() {
        let fixture_root = workspace_root().join("data/worlds/abutopia");
        let runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
            .expect("base world fixture must load");
        let agents = sim_core::mobility::api::agents(&runtime.world);
        let vehicles = sim_core::mobility::api::vehicles(&runtime.world);

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id.0, "agent:walk:0");
        assert!(vehicles.is_empty());
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("sim-server crate lives under backend/crates/sim-server")
            .to_path_buf()
    }

    fn base_world_fixture() -> sim_core::base_world::BaseWorldBundle {
        sim_core::base_world::BaseWorldBundle::load_from_dir(
            workspace_root().join("data/worlds/abutopia"),
        )
        .expect("base world fixture loads")
    }

    #[test]
    fn mobility_snapshot_base_world_match_rejects_stale_vehicle() {
        use sim_core::mobility::{extract_from_world, seed};

        let base_world = base_world_fixture();
        let (authored, _) =
            seed::from_base_world_bundle(&base_world).expect("base world mobility seed succeeds");
        let mut authored_snap = extract_from_world(&authored);
        assert!(mobility_snapshot_matches_base_world(
            &authored_snap,
            &base_world
        ));

        authored_snap.vehicles.insert(
            sim_core::ids::VehicleId("vehicle:stale".to_string()),
            sim_core::mobility::VehicleRecord {
                id: sim_core::ids::VehicleId("vehicle:stale".to_string()),
                kind: sim_core::mobility::VehicleKind::Car,
                route_id: "route:arterial:stale".to_string(),
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.0,
                capacity: 1,
                occupants: Vec::new(),
                dwell_ticks_remaining: 0,
            },
        );

        assert!(!mobility_snapshot_matches_base_world(
            &authored_snap,
            &base_world
        ));
    }

    #[test]
    fn runtime_has_populated_routing_graph() {
        let network = base_world_fixture().to_city_network();
        let runtime = SimulationRuntime::new_from_network(&network);
        let world = &runtime.world;
        let graph = world.resource::<sim_core::routing::Graph>();
        assert!(
            graph.node_count() > 0,
            "graph must have nodes after hydration"
        );
        assert!(
            graph.edge_count() > 0,
            "graph must have edges after hydration"
        );
        let traffic_routes = world.resource::<sim_core::routing::TrafficRoutes>();
        assert_eq!(traffic_routes.count(), 0);
        let spatial = world.resource::<sim_core::routing::NodeSpatialIndex>();
        assert_eq!(spatial.size(), graph.node_count());
    }

    #[test]
    fn runtime_has_pathfinding_resources() {
        let runtime = SimulationRuntime::new();
        assert!(
            runtime
                .world
                .contains_resource::<sim_core::routing::PathCache>()
        );
    }

    #[test]
    fn runtime_installs_flow_field_cache() {
        let runtime = SimulationRuntime::new();
        assert!(
            runtime
                .world
                .contains_resource::<sim_core::routing::FlowFieldCache>()
        );
        assert_eq!(
            runtime
                .world
                .resource::<sim_core::routing::FlowFieldCache>()
                .len(),
            0
        );
    }

    #[test]
    fn runtime_installs_hpa_index_for_seeded_graph() {
        let network = base_world_fixture().to_city_network();
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();

        assert!(hpa.cluster_count() > 0);
        assert_eq!(hpa.portal_count(), 0);
        assert!(hpa.cluster_count() <= graph.node_count());
    }

    #[test]
    fn runtime_can_find_seeded_hierarchical_walk_path() {
        let network = base_world_fixture().to_city_network();
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();
        let walk_edge = graph
            .edges()
            .iter()
            .find(|edge| edge.kind == sim_core::routing::EdgeKind::Footway)
            .expect("seeded runtime should contain a footway edge");

        let (path, stats) = sim_core::routing::HpaRouter::find_path(
            graph,
            hpa,
            sim_core::routing::PathRequest {
                from: walk_edge.from,
                to: walk_edge.to,
                profile: sim_core::routing::RoutingProfileKey::Walk,
            },
            sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Walk),
        )
        .expect("seeded footway edge endpoints should route through HPA");

        assert!(!path.edges.is_empty());
        assert!(stats.corridor_cluster_count >= 1);
        assert!(
            path.edges
                .iter()
                .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::Footway)
        );
    }

    #[test]
    fn runtime_can_find_seeded_walk_path() {
        let network = base_world_fixture().to_city_network();
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let walk_edge = graph
            .edges()
            .iter()
            .find(|edge| edge.kind == sim_core::routing::EdgeKind::Footway)
            .expect("seeded runtime should contain a footway edge");
        let path = sim_core::routing::AStarRouter::find_path(
            graph,
            sim_core::routing::PathRequest {
                from: walk_edge.from,
                to: walk_edge.to,
                profile: sim_core::routing::RoutingProfileKey::Walk,
            },
            sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Walk),
        )
        .expect("seeded footway edge endpoints should be connected by the routing graph");
        assert!(!path.edges.is_empty());
        assert!(
            path.edges
                .iter()
                .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::Footway)
        );
    }

    #[test]
    fn set_mobility_for_test_refreshes_hpa_index() {
        let network = base_world_fixture().to_city_network();
        let mut runtime = SimulationRuntime::new_from_network(&network);

        runtime.set_mobility_for_test(sim_core::mobility::seed::from_network(
            &network,
            SEED_DENSITY,
        ));

        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();
        let expected =
            sim_core::routing::HpaIndex::build(graph, sim_core::routing::HpaConfig::default())
                .expect("current graph should build an HPA index");

        assert_eq!(hpa.cluster_count(), expected.cluster_count());
        assert_eq!(hpa.portal_count(), expected.portal_count());
        for node in graph.nodes() {
            assert_eq!(
                hpa.cluster_of_node(node.id),
                expected.cluster_of_node(node.id)
            );
        }
    }

    #[test]
    fn set_mobility_for_test_refreshes_flow_field_cache() {
        let network = base_world_fixture().to_city_network();
        let mut runtime = SimulationRuntime::new_from_network(&network);
        runtime.world.insert_resource(populated_flow_field_cache());

        runtime.set_mobility_for_test(sim_core::mobility::seed::from_network(
            &network,
            SEED_DENSITY,
        ));

        assert!(
            runtime
                .world
                .contains_resource::<sim_core::routing::FlowFieldCache>()
        );
        assert_eq!(
            runtime
                .world
                .resource::<sim_core::routing::FlowFieldCache>()
                .len(),
            0
        );
    }

    #[test]
    fn hydration_spawns_chunk_entity_per_loaded_chunk() {
        let runtime = SimulationRuntime::new();
        let world = &runtime.world;
        let by_coord = world.resource::<sim_core::world::resources::ChunksByCoord>();
        assert_eq!(by_coord.0.len(), 1);
        assert!(
            by_coord
                .0
                .contains_key(&sim_core::ids::ChunkCoord { x: 0, y: 0 })
        );
    }
    use sim_core::persistence::{
        InMemoryChunkSnapshotStore, InMemoryMobilitySnapshotStore, build_chunk_snapshot,
    };

    fn tile_pulse(message: ServerMessageDto) -> TilePulseDeltaDto {
        let ServerMessageDto::TilePulse(delta) = message else {
            panic!("message should be a tile pulse");
        };
        delta
    }

    #[test]
    fn runtime_summarizes_abutopia_loaded_chunk() {
        let runtime = SimulationRuntime::new();

        let summary = runtime.world_summary();

        assert_eq!(summary.chunk_size, 32);
        assert_eq!(summary.world_id.0, "abutopia");
        assert_eq!(summary.loaded_chunks.len(), 1);
        assert_eq!(
            summary.loaded_chunks.first(),
            Some(&ChunkCoordDto { x: 0, y: 0 })
        );
    }

    #[test]
    fn runtime_returns_snapshot_for_abutopia_chunk() {
        let runtime = SimulationRuntime::new();

        let visible = runtime
            .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
            .expect("visible chunk loaded");

        assert_eq!(visible.coord, ChunkCoordDto { x: 0, y: 0 });
        assert!(runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).is_some());
        assert!(runtime.chunk_snapshot(ChunkCoord { x: 1, y: 0 }).is_none());
    }

    #[test]
    fn runtime_pulses_the_single_abutopia_chunk() {
        let mut runtime = SimulationRuntime::new();

        let first = tile_pulse(runtime.next_pulse());
        let second = tile_pulse(runtime.next_pulse());
        let third = tile_pulse(runtime.next_pulse());
        let fourth = tile_pulse(runtime.next_pulse());

        assert_eq!(first.tick, 1);
        assert_eq!(first.version, 1);
        assert_eq!(first.coord, ChunkCoordDto { x: 0, y: 0 });
        assert!(first.local_index < 1024);
        assert_eq!(second.tick, 2);
        assert_eq!(second.coord, ChunkCoordDto { x: 0, y: 0 });
        assert_eq!(third.tick, 3);
        assert_eq!(third.coord, ChunkCoordDto { x: 0, y: 0 });
        assert_eq!(fourth.tick, 4);
        assert_eq!(fourth.coord, ChunkCoordDto { x: 0, y: 0 });
    }

    #[tokio::test]
    async fn collect_provider_items_routes_dirty_chunk_to_chunk_store() {
        // Issue #1 acceptance: construct a runtime, mutate a tile (so a
        // chunk becomes dirty), drive the persist path via SnapshotProviders
        // (not the legacy `collect_chunk_snapshots()` shortcut), and verify
        // a `ChunkSnapshotStore` receives the chunk snapshot.
        use sim_core::persistence::InMemoryChunkSnapshotStore;

        let mut runtime = SimulationRuntime::new();
        // Mutate the authored abutopia chunk to make sure the dirty path
        // through SnapshotProviders is exercised.
        runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    command_id: "command:provider-path:1".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                    local_index: 9,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect("command applies cleanly");

        let items = runtime.collect_provider_items();
        // Expect at least one chunk item (for the dirty chunk) and exactly
        // one mobility item.
        let chunk_items: Vec<_> = items.iter().filter(|i| i.key.kind == "chunk").collect();
        let mobility_items: Vec<_> = items.iter().filter(|i| i.key.kind == "mobility").collect();
        assert!(
            !chunk_items.is_empty(),
            "expected at least one chunk SnapshotItem from provider path",
        );
        assert_eq!(
            mobility_items.len(),
            1,
            "MobilitySnapshotProvider emits exactly one item per collect",
        );

        // Dispatch chunk items to the in-memory ChunkSnapshotStore via the
        // same code path as the persist loop in `app.rs`.
        let mut store = InMemoryChunkSnapshotStore::default();
        let compatibility = base_world_fixture().snapshot_compatibility();
        for item in chunk_items {
            let dto: abutown_protocol::ChunkSnapshotDto = serde_json::from_slice(&item.payload)
                .expect("provider emits valid ChunkSnapshotDto JSON");
            ChunkSnapshotStore::write_snapshot(&mut store, dto, &compatibility)
                .await
                .expect("in-memory store write");
        }

        let stored = store
            .read_snapshot(ChunkCoord { x: 0, y: 0 }, &compatibility)
            .expect("snapshot for the mutated chunk landed in the store");
        assert_eq!(stored.coord, abutown_protocol::ChunkCoordDto { x: 0, y: 0 });
    }

    #[tokio::test]
    async fn runtime_collects_chunk_snapshots_and_marks_persisted() {
        use sim_core::persistence::InMemoryChunkSnapshotStore;

        let mut runtime = SimulationRuntime::new();
        let mut store = InMemoryChunkSnapshotStore::default();
        let compatibility = base_world_fixture().snapshot_compatibility();

        let snapshots = runtime.collect_chunk_snapshots();
        assert_eq!(snapshots.len(), 0);

        // After marking persisted with no further events and within the 30s ceiling,
        // the registry must skip every chunk.
        assert_eq!(runtime.collect_chunk_snapshots().len(), 0);

        // A new event on one chunk re-arms only that chunk for the next collect.
        runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    command_id: "command:persist-test:1".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect("command should apply");

        let next_snapshots = runtime.collect_chunk_snapshots();
        assert_eq!(next_snapshots.len(), 1);
        for snapshot in &next_snapshots {
            store.write_snapshot(snapshot.clone(), &compatibility);
        }
        let next_coords: Vec<ChunkCoord> = next_snapshots
            .iter()
            .map(|s| ChunkCoord {
                x: s.coord.x,
                y: s.coord.y,
            })
            .collect();
        runtime.mark_chunk_snapshots_persisted(&next_coords);

        let visible = store
            .read_snapshot(ChunkCoord { x: 0, y: 0 }, &compatibility)
            .expect("visible snapshot reflects new event");
        assert!(visible.tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }

    #[tokio::test]
    async fn runtime_applies_set_tile_kind_command_and_appends_event() {
        let mut runtime = SimulationRuntime::new();

        let applied = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    command_id: "command:test:1".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect("command should apply");

        let abutown_protocol::WorldEventDto::TileKindSet(event) = &applied.event;
        assert!(event.event_id.starts_with("event:"));
        assert_eq!(event.command_id, "command:test:1");
        assert_eq!(event.version, 1);
        assert_eq!(event.local_index, 11);
        assert_eq!(event.kind, abutown_protocol::TileKindDto::Water);
        assert_eq!(runtime.event_count(), 1);

        let snapshot = runtime
            .chunk_snapshot(sim_core::ids::ChunkCoord { x: 0, y: 0 })
            .expect("mutated chunk snapshot exists");
        assert!(snapshot.tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }

    #[tokio::test]
    async fn runtime_rejects_commands_for_other_worlds() {
        let mut runtime = SimulationRuntime::new();

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("other-world".to_string()),
                    command_id: "command:test:2".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect_err("wrong world should reject");

        assert_eq!(rejection.code, "wrong_world");
        assert_eq!(runtime.event_count(), 0);
    }

    #[tokio::test]
    async fn runtime_rejects_commands_for_unloaded_chunks() {
        let mut runtime = SimulationRuntime::new();

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    command_id: "command:test:3".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 9, y: 9 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect_err("unloaded chunk should reject");

        assert_eq!(rejection.code, "chunk_not_loaded");
        assert_eq!(runtime.event_count(), 0);
    }

    #[tokio::test]
    async fn runtime_rejects_no_op_tile_kind_commands_without_appending_event() {
        let mut runtime = SimulationRuntime::new();
        let current_kind = runtime
            .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
            .and_then(|snapshot| {
                snapshot
                    .tiles
                    .into_iter()
                    .find(|tile| tile.local_index == 11)
                    .map(|tile| tile.kind)
            })
            .unwrap_or(abutown_protocol::TileKindDto::Grass);

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    command_id: "command:test:4".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                    local_index: 11,
                    kind: current_kind,
                },
            ))
            .await
            .expect_err("no-op command should reject");

        assert_eq!(rejection.code, "no_state_change");
        assert_eq!(runtime.event_count(), 0);
    }

    #[tokio::test]
    async fn hydrate_from_stores_restores_chunk_from_snapshot_and_replays_tail_events() {
        // Seed: a chunk with tile 0 = Road at version 1, snapshotted.
        let mut authoring_chunk = Chunk::new(ChunkCoord { x: 0, y: 0 }, 32);
        authoring_chunk.set_tile_kind(0, TileKind::Road).unwrap();
        let snapshot = build_chunk_snapshot("abutopia", &authoring_chunk, ChunkActivity::Active);

        let mut snapshot_store = InMemoryChunkSnapshotStore::default();
        let base_world = base_world_fixture();
        let compatibility = base_world.snapshot_compatibility();
        ChunkSnapshotStore::write_snapshot(&mut snapshot_store, snapshot, &compatibility)
            .await
            .unwrap();

        // Tail event after the snapshot: tile 7 = Water at chunk_version 2.
        let tail_event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:tail".to_string(),
            command_id: "command:tail".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 2,
            version: 2,
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 7,
            kind: TileKindDto::Water,
        });
        let mut event_store = InMemoryWorldEventStore::default();
        WorldEventStore::append(&mut event_store, tail_event)
            .await
            .unwrap();

        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            &base_world,
        )
        .await
        .unwrap();

        let restored = runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).unwrap();
        assert_eq!(restored.chunk_version, 2);
        let kinds: std::collections::HashMap<u16, TileKindDto> = restored
            .tiles
            .iter()
            .map(|t| (t.local_index, t.kind))
            .collect();
        assert_eq!(kinds.get(&0), Some(&TileKindDto::Road));
        assert_eq!(kinds.get(&7), Some(&TileKindDto::Water));
        assert_eq!(restored.chunk_state, ChunkStateDto::Active);
    }

    #[tokio::test]
    async fn hydrate_from_stores_seeds_when_no_snapshot() {
        let base_world = base_world_fixture();
        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            &base_world,
        )
        .await
        .unwrap();

        let snap = runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).unwrap();
        assert_eq!(
            snap.chunk_version, 0,
            "base world chunks start at version 0 before player mutations"
        );
    }

    #[tokio::test]
    async fn runtime_rejects_store_failure_without_mutating_chunk() {
        let mut runtime = SimulationRuntime::new_with_event_store(Box::new(
            sim_core::events::FailingWorldEventStore::new("database offline"),
        ));

        let before = runtime
            .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
            .expect("chunk exists");

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutopia".to_string()),
                    command_id: "command:test:store-failure".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect_err("store failure should reject");

        assert_eq!(rejection.code, "event_store_unavailable");
        assert_eq!(runtime.event_count(), 0);
        assert_eq!(
            runtime
                .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
                .expect("chunk still exists"),
            before
        );
    }

    #[tokio::test]
    async fn duplicate_command_id_is_idempotent_and_writes_only_one_event() {
        use abutown_protocol::{
            ChunkCoordDto, ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto,
            WorldId,
        };

        let mut runtime = SimulationRuntime::new();
        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutopia".to_string()),
            command_id: "command:dup".to_string(),
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 12,
            kind: TileKindDto::Water,
        });

        let first = runtime.apply_client_command(command.clone()).await.unwrap();
        let second = runtime.apply_client_command(command).await.unwrap();

        assert_eq!(
            first.response, second.response,
            "duplicate command must return identical response"
        );
        assert_eq!(
            first.event, second.event,
            "duplicate command must return identical event"
        );
        assert_eq!(runtime.event_count(), 1, "only one event must be appended");
    }

    #[derive(Debug)]
    struct RaceyEventStore {
        planted_winner: WorldEventDto,
        appended: bool,
    }

    impl RaceyEventStore {
        fn new(planted_winner: WorldEventDto) -> Self {
            Self {
                planted_winner,
                appended: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl WorldEventStore for RaceyEventStore {
        async fn append(
            &mut self,
            _event: WorldEventDto,
        ) -> Result<(), sim_core::events::WorldEventStoreError> {
            self.appended = true;
            Err(sim_core::events::WorldEventStoreError::duplicate_command(
                "command:race",
            ))
        }
        async fn find_event_by_command(
            &self,
            _world_id: &str,
            command_id: &str,
        ) -> Result<Option<WorldEventDto>, sim_core::events::WorldEventStoreError> {
            // Pre-flight call (before append) returns None so we fall through to the append path.
            // Refetch call (after append, in the race handler) returns the planted winner.
            if self.appended && command_id == "command:race" {
                Ok(Some(self.planted_winner.clone()))
            } else {
                Ok(None)
            }
        }
        async fn read_chunk_events_since(
            &self,
            _world_id: &str,
            _coord: abutown_protocol::ChunkCoordDto,
            _after_chunk_version: u64,
        ) -> Result<Vec<WorldEventDto>, sim_core::events::WorldEventStoreError> {
            Ok(Vec::new())
        }
        async fn max_tick(
            &self,
            _world_id: &str,
        ) -> Result<Option<u64>, sim_core::events::WorldEventStoreError> {
            Ok(None)
        }
        async fn max_version(
            &self,
            _world_id: &str,
        ) -> Result<Option<u64>, sim_core::events::WorldEventStoreError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn race_handler_returns_winner_when_append_reports_duplicate() {
        use abutown_protocol::{
            ChunkCoordDto, ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto,
            TileKindSetEventDto, WorldEventDto, WorldId,
        };

        let winner = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:winner".to_string(),
            command_id: "command:race".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 7,
            version: 7,
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 0,
            kind: TileKindDto::Water,
        });
        let mut runtime =
            SimulationRuntime::new_with_event_store(Box::new(RaceyEventStore::new(winner.clone())));

        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutopia".to_string()),
            command_id: "command:race".to_string(),
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 13,
            kind: TileKindDto::Road,
        });

        let result = runtime.apply_client_command(command).await.unwrap();
        assert_eq!(
            result.event, winner,
            "race handler must return the planted winner event"
        );
        assert_eq!(result.response.event, winner);
    }

    #[tokio::test]
    async fn hydrate_seeds_fresh_mobility_when_store_is_empty() {
        let base_world = base_world_fixture();
        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            &base_world,
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), 0);
        assert_eq!(runtime.mobility_agent_count_for_test(), 1);
        assert_eq!(
            runtime.mobility_vehicle_count_for_test(),
            expected_base_world_car_count(&base_world)
        );
    }

    #[tokio::test]
    async fn hydrate_restores_mobility_from_store_when_present() {
        use sim_core::mobility::api::tick_mobility as api_tick;
        use sim_core::mobility::{extract_from_world, seed};

        let base_world = base_world_fixture();
        let (mut authored, mut sched) =
            seed::from_base_world_bundle(&base_world).expect("base world mobility seed succeeds");
        // Advance one tick so the persisted state differs from a fresh seed.
        let _ = api_tick(&mut authored, &mut sched);
        let persisted_tick = sim_core::mobility::api::tick(&authored);
        let authored_snap = extract_from_world(&authored);

        let mut mobility_store = InMemoryMobilitySnapshotStore::default();
        MobilitySnapshotStore::write(
            &mut mobility_store,
            "abutopia",
            persisted_tick,
            &authored_snap,
            &base_world.snapshot_compatibility(),
        )
        .await
        .unwrap();

        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(mobility_store),
            &base_world,
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), persisted_tick);
    }

    #[tokio::test]
    async fn hydrate_ignores_snapshot_with_stale_vehicle_for_carless_abutopia() {
        use sim_core::mobility::seed;

        let base_world = base_world_fixture();
        let (authored, _) =
            seed::from_base_world_bundle(&base_world).expect("base world mobility seed succeeds");
        let mut authored_snap = extract_from_world(&authored);
        authored_snap.vehicles.insert(
            sim_core::ids::VehicleId("vehicle:stale".to_string()),
            sim_core::mobility::VehicleRecord {
                id: sim_core::ids::VehicleId("vehicle:stale".to_string()),
                kind: sim_core::mobility::VehicleKind::Car,
                route_id: "route:arterial:stale".to_string(),
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.0,
                capacity: 1,
                occupants: Vec::new(),
                dwell_ticks_remaining: 0,
            },
        );

        let mut mobility_store = InMemoryMobilitySnapshotStore::default();
        MobilitySnapshotStore::write(
            &mut mobility_store,
            "abutopia",
            99,
            &authored_snap,
            &base_world.snapshot_compatibility(),
        )
        .await
        .unwrap();

        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(mobility_store),
            &base_world,
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), 0);
        assert_eq!(
            runtime.mobility_vehicle_count_for_test(),
            expected_base_world_car_count(&base_world)
        );
    }
}
