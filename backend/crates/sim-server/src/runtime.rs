use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, HealthResponse, MobilitySnapshotDto, PROTOCOL_VERSION,
    ServerHelloDto, ServerMessageDto, TilePulseDeltaDto, WorldId, WorldSummaryDto,
};
use sim_core::{
    chunk::{Chunk, SnapshotDecodeError},
    events::{InMemoryWorldEventStore, WorldEventStore},
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
    world::{
        components::{ChunkVersion, DirtyTiles, LastPersistedVersion, LastSnapshotAt, Tiles},
        plugin::CorePlugin,
        resources::ChunksByCoord,
        schedule::SimPlugin,
        systems::{chunk_snapshot_data, spawn_chunk_entity},
    },
};

#[derive(Debug, thiserror::Error)]
pub enum HydrationError {
    #[error("snapshot store error: {0}")]
    Snapshot(ChunkSnapshotStoreError),
    #[error("snapshot decode error: {0}")]
    Decode(SnapshotDecodeError),
    #[error("terrain seed error: {0}")]
    Seed(String),
    #[error("mobility store error: {0}")]
    Mobility(sim_core::persistence::MobilitySnapshotStoreError),
}

const WORLD_ID: &str = "abutown-main";
const PULSE_STRIDE: u64 = 37;
pub const TICK_PERIOD_MS: u32 = 100;

pub const SEED_DENSITY: sim_core::mobility::seed::SeedDensity =
    sim_core::mobility::seed::SeedDensity {
        pedestrians_per_corridor: 6,
        cars_per_arterial: 17,
        trams_total: 4,
    };

type LayeredTerrainSeed = sim_core::terrain_seed::LayeredTerrainSeed;

fn load_validated_layered_seed() -> Result<LayeredTerrainSeed, HydrationError> {
    let seed = sim_core::terrain_seed::load_zurich_layered_terrain_seed()
        .map_err(|error| HydrationError::Seed(error.to_string()))?;
    let errors = sim_core::terrain_seed::validate_seed(&seed);
    if !errors.is_empty() {
        return Err(HydrationError::Seed(format!(
            "bundled terrain seed invalid: {errors:?}"
        )));
    }
    Ok(seed)
}

fn spawn_all_seed_chunks(
    world: &mut sim_core::bevy_ecs::world::World,
    seed: &LayeredTerrainSeed,
) -> Result<(), HydrationError> {
    let chunk_size = u32::from(seed.chunk_size);
    for chunk_y in 0..(seed.height / chunk_size) {
        for chunk_x in 0..(seed.width / chunk_size) {
            let coord = ChunkCoord {
                x: chunk_x as i32,
                y: chunk_y as i32,
            };
            let tiles = sim_core::terrain_seed::chunk_tiles_from_seed(seed, coord).ok_or_else(
                || {
                    HydrationError::Seed(format!(
                        "seed missing chunk tiles for {}:{}",
                        coord.x, coord.y
                    ))
                },
            )?;
            spawn_chunk_entity(world, coord, seed.chunk_size, tiles, 0, ChunkActivity::Active);
        }
    }
    Ok(())
}

pub struct SimulationRuntime {
    world_id: WorldId,
    chunk_size: u16,
    pub(crate) world: sim_core::bevy_ecs::world::World,
    pub(crate) schedule: sim_core::bevy_ecs::schedule::Schedule,
    event_count: usize,
    tick: u64,
    version: u64,
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

    pub fn new_with_event_store(_event_store: Box<dyn WorldEventStore + Send + Sync>) -> Self {
        // Build a fresh World + Schedule with CorePlugin + mobility installed
        // directly. Phase 8a Task 9 dissolved the `MobilityWorld` wrapper —
        // SimulationRuntime now owns the shared `World` + `Schedule` directly.
        let mut world = sim_core::bevy_ecs::world::World::new();
        let mut schedule = sim_core::bevy_ecs::schedule::Schedule::default();

        // Load city network from disk and insert as resource before plugins run.
        let network_path = std::env::var("ABUTOWN_CITY_NETWORK_PATH")
            .unwrap_or_else(|_| "data/city/zurich-network.json".to_string());
        let city_network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .unwrap_or_else(|_| {
                sim_core::city_network::CityNetwork::empty_for_world("abutown-main")
            });
        let seeded_stops = sim_core::mobility::seed::legacy_seeded_stops();
        let seeded_walks = sim_core::mobility::seed::legacy_seeded_walks(&city_network);
        world.insert_resource(city_network);

        CorePlugin::default().install(&mut world, &mut schedule);

        sim_core::routing::RoutingPlugin {
            seeded_stops,
            seeded_walks,
        }
        .install(&mut world, &mut schedule);

        sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);

        MobilityPlugin.install(&mut world, &mut schedule);
        crate::persistence_plugin::PersistencePlugin {
            world_id: WORLD_ID.to_string(),
        }
        .install(&mut world, &mut schedule);

        let seed = load_validated_layered_seed().expect("bundled Zurich terrain seed is valid");
        spawn_all_seed_chunks(&mut world, &seed).expect("bundled Zurich seed chunks exist");

        Self {
            world_id: Self::default_world_id(),
            chunk_size: seed.chunk_size,
            world,
            schedule,
            event_count: 0,
            tick: 0,
            version: 0,
        }
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
        _event_store: Box<dyn WorldEventStore + Send + Sync>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send + Sync>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send + Sync>,
        network: &sim_core::city_network::CityNetwork,
    ) -> Result<
        (
            Self,
            Box<dyn ChunkSnapshotStore + Send + Sync>,
            Box<dyn MobilitySnapshotStore + Send + Sync>,
        ),
        HydrationError,
    > {
        let world_id = Self::default_world_id();

        // Build a fresh World + Schedule and install both plugins.
        let mut world = sim_core::bevy_ecs::world::World::new();
        let mut schedule = sim_core::bevy_ecs::schedule::Schedule::default();

        let seeded_stops = sim_core::mobility::seed::legacy_seeded_stops();
        let seeded_walks = sim_core::mobility::seed::legacy_seeded_walks(network);

        // Insert city network as resource before plugins run.
        world.insert_resource(network.clone());

        CorePlugin::default().install(&mut world, &mut schedule);

        sim_core::routing::RoutingPlugin {
            seeded_stops,
            seeded_walks,
        }
        .install(&mut world, &mut schedule);

        sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);

        MobilityPlugin.install(&mut world, &mut schedule);
        crate::persistence_plugin::PersistencePlugin {
            world_id: world_id.0.clone(),
        }
        .install(&mut world, &mut schedule);

        // Hydrate mobility state from the snapshot store if present, else seed
        // from the network descriptor.
        let mobility_snap = match mobility_snapshot_store
            .read(&world_id.0)
            .await
            .map_err(HydrationError::Mobility)?
        {
            Some((_tick, snap)) => snap,
            None => {
                let (seeded_world, _) = if network.arterial_paths.is_empty()
                    && network.pedestrian_corridors.is_empty()
                {
                    sim_core::mobility::seed::tiny_world()
                } else {
                    sim_core::mobility::seed::from_network(network, SEED_DENSITY)
                };
                extract_from_world(&seeded_world)
            }
        };
        apply_into_world(&mut world, mobility_snap);
        sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);

        let seed = load_validated_layered_seed()?;
        let seed_chunk_size = u32::from(seed.chunk_size);
        for chunk_y in 0..(seed.height / seed_chunk_size) {
            for chunk_x in 0..(seed.width / seed_chunk_size) {
                let coord = ChunkCoord {
                    x: chunk_x as i32,
                    y: chunk_y as i32,
                };
                let snap = snapshot_store
                    .read_snapshot(coord)
                    .await
                    .map_err(HydrationError::Snapshot)?;

                let (chunk_size, tiles, chunk_version, activity) = match snap {
                    Some(snapshot) => {
                        let version = snapshot.chunk_version;
                        let activity = ChunkActivity::from(snapshot.chunk_state);
                        let chunk =
                            Chunk::from_snapshot(&snapshot).map_err(HydrationError::Decode)?;
                        let tiles = (0..chunk.tile_count())
                            .filter_map(|i| chunk.tile_at(i))
                            .collect();
                        (chunk.chunk_size(), tiles, version, activity)
                    }
                    None => {
                        let tiles = sim_core::terrain_seed::chunk_tiles_from_seed(&seed, coord)
                            .ok_or_else(|| {
                                HydrationError::Seed(format!(
                                    "seed missing chunk tiles for {}:{}",
                                    coord.x, coord.y
                                ))
                            })?;
                        (seed.chunk_size, tiles, 0, ChunkActivity::Active)
                    }
                };

                spawn_chunk_entity(
                    &mut world,
                    coord,
                    chunk_size,
                    tiles,
                    chunk_version,
                    activity,
                );
            }
        }

        let runtime = Self {
            world_id,
            chunk_size: seed.chunk_size,
            world,
            schedule,
            event_count: 0,
            tick: 0,
            version: 0,
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
    use abutown_protocol::{TileBaseDto, TileSurfaceDto};

    #[test]
    fn simulation_runtime_holds_world_directly() {
        let runtime = SimulationRuntime::new();
        // After Task 9 dissolved MobilityWorld, SimulationRuntime owns the
        // shared bevy World + Schedule directly.
        let _world: &sim_core::bevy_ecs::world::World = &runtime.world;
        let _schedule: &sim_core::bevy_ecs::schedule::Schedule = &runtime.schedule;
    }

    #[test]
    fn runtime_has_populated_routing_graph() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
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
        let transit = world.resource::<sim_core::routing::TransitLines>();
        assert!(transit.count() > 0, "must have at least one transit line");
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
    fn runtime_installs_hpa_index_for_seeded_graph() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();

        assert!(hpa.cluster_count() > 0);
        assert!(hpa.portal_count() > 0);
        assert!(hpa.cluster_count() <= graph.node_count());
    }

    #[test]
    fn runtime_can_find_seeded_hierarchical_path() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();
        let transit_lines = runtime.world.resource::<sim_core::routing::TransitLines>();
        let line = transit_lines
            .iter()
            .find(|line| !line.edges.is_empty())
            .expect("seeded runtime should contain a non-empty transit line");
        let tram_edge = graph.edge(*line.edges.first().expect("line has first edge"));

        let (path, stats) = sim_core::routing::HpaRouter::find_path(
            graph,
            hpa,
            sim_core::routing::PathRequest {
                from: tram_edge.from,
                to: tram_edge.to,
                profile: sim_core::routing::RoutingProfileKey::Tram,
            },
            sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Tram),
        )
        .expect("seeded tram edge endpoints should route through HPA");

        assert!(!path.edges.is_empty());
        assert!(stats.corridor_cluster_count >= 1);
        assert!(path
            .edges
            .iter()
            .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::TramTrack));
    }

    #[test]
    fn runtime_can_find_seeded_tram_path() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
        let runtime = SimulationRuntime::new_from_network(&network);
        let graph = runtime.world.resource::<sim_core::routing::Graph>();
        let transit_lines = runtime.world.resource::<sim_core::routing::TransitLines>();
        let line = transit_lines
            .iter()
            .find(|line| !line.edges.is_empty())
            .expect("seeded runtime should contain a non-empty transit line");
        let tram_edge = graph.edge(*line.edges.first().expect("line has first edge"));
        let path = sim_core::routing::AStarRouter::find_path(
            graph,
            sim_core::routing::PathRequest {
                from: tram_edge.from,
                to: tram_edge.to,
                profile: sim_core::routing::RoutingProfileKey::Tram,
            },
            sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Tram),
        )
        .expect("seeded tram edge endpoints should be connected by the routing graph");
        assert!(!path.edges.is_empty());
        assert!(path
            .edges
            .iter()
            .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::TramTrack));
    }

    #[test]
    fn set_mobility_for_test_refreshes_hpa_index() {
        let network_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../data/city/zurich-network.json");
        let network = sim_core::city_network::CityNetwork::load_from_path(&network_path)
            .expect("zurich fixture network must load");
        let mut runtime = SimulationRuntime::new_from_network(&network);

        runtime.set_mobility_for_test(sim_core::mobility::seed::tiny_world());

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
    fn hydration_spawns_chunk_entity_per_loaded_chunk() {
        let runtime = SimulationRuntime::new();
        let world = &runtime.world;
        let by_coord = world.resource::<sim_core::world::resources::ChunksByCoord>();
        assert_eq!(by_coord.0.len(), 64);
        assert!(by_coord.0.contains_key(&sim_core::ids::ChunkCoord { x: 4, y: 4 }));
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

    fn empty_test_network() -> sim_core::city_network::CityNetwork {
        sim_core::city_network::CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: sim_core::city_network::WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![],
            pedestrian_corridors: vec![],
        }
    }

    #[test]
    fn runtime_summarizes_multiple_loaded_chunks() {
        let runtime = SimulationRuntime::new();

        let summary = runtime.world_summary();

        assert_eq!(summary.chunk_size, 32);
        assert_eq!(summary.loaded_chunks.len(), 64);
        assert_eq!(summary.loaded_chunks.first(), Some(&ChunkCoordDto { x: 0, y: 0 }));
        assert_eq!(summary.loaded_chunks.last(), Some(&ChunkCoordDto { x: 7, y: 7 }));
    }

    #[test]
    fn runtime_returns_snapshots_for_each_loaded_chunk() {
        let runtime = SimulationRuntime::new();

        let visible = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible chunk loaded");
        let east = runtime
            .chunk_snapshot(ChunkCoord { x: 5, y: 4 })
            .expect("east chunk loaded");
        let south = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 5 })
            .expect("south chunk loaded");

        assert_eq!(visible.coord, ChunkCoordDto { x: 4, y: 4 });
        assert_eq!(east.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(south.coord, ChunkCoordDto { x: 4, y: 5 });
        assert!(runtime.chunk_snapshot(ChunkCoord { x: 8, y: 8 }).is_none());
    }

    #[test]
    fn fresh_runtime_hydrates_chunks_from_layered_terrain_seed() {
        let runtime = SimulationRuntime::new();

        let snapshot = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("seeded chunk snapshot");

        assert_eq!(snapshot.tile_count, 1024);
        assert!(
            snapshot
                .tiles
                .iter()
                .any(|tile| tile.base == TileBaseDto::Water)
        );
        assert!(
            snapshot
                .tiles
                .iter()
                .any(|tile| tile.surface == TileSurfaceDto::Street)
        );
    }

    #[test]
    fn runtime_rotates_pulses_through_loaded_chunks() {
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
        assert_eq!(second.coord, ChunkCoordDto { x: 1, y: 0 });
        assert_eq!(third.tick, 3);
        assert_eq!(third.coord, ChunkCoordDto { x: 2, y: 0 });
        assert_eq!(fourth.tick, 4);
        assert_eq!(fourth.coord, ChunkCoordDto { x: 3, y: 0 });
    }

    #[tokio::test]
    async fn collect_provider_items_routes_dirty_chunk_to_chunk_store() {
        use sim_core::persistence::InMemoryChunkSnapshotStore;
        use sim_core::tile::{TileBase, TileRecord};

        let mut runtime = SimulationRuntime::new();
        let coord = ChunkCoord { x: 4, y: 4 };
        let entity = runtime.world.resource::<ChunksByCoord>().0[&coord];
        {
            let mut ent = runtime.world.entity_mut(entity);
            ent.get_mut::<Tiles>().unwrap().0[9] = TileRecord {
                base: TileBase::Water,
                version: 1,
                ..TileRecord::default()
            };
            ent.get_mut::<ChunkVersion>().unwrap().0 = 1;
        }

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
        for item in chunk_items {
            let dto: abutown_protocol::ChunkSnapshotDto = serde_json::from_slice(&item.payload)
                .expect("provider emits valid ChunkSnapshotDto JSON");
            ChunkSnapshotStore::write_snapshot(&mut store, dto)
                .await
                .expect("in-memory store write");
        }

        let stored = store
            .read_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("snapshot for the mutated chunk landed in the store");
        assert_eq!(stored.coord, abutown_protocol::ChunkCoordDto { x: 4, y: 4 });
    }

    #[tokio::test]
    async fn runtime_collects_chunk_snapshots_and_marks_persisted() {
        use sim_core::persistence::InMemoryChunkSnapshotStore;
        use sim_core::tile::{TileBase, TileRecord};

        let mut runtime = SimulationRuntime::new();
        let mut store = InMemoryChunkSnapshotStore::default();
        let coord = ChunkCoord { x: 4, y: 4 };
        let entity = runtime.world.resource::<ChunksByCoord>().0[&coord];
        {
            let mut ent = runtime.world.entity_mut(entity);
            ent.get_mut::<Tiles>().unwrap().0[11] = TileRecord {
                base: TileBase::Water,
                version: 1,
                ..TileRecord::default()
            };
            ent.get_mut::<ChunkVersion>().unwrap().0 = 1;
        }

        let snapshots = runtime.collect_chunk_snapshots();
        assert_eq!(snapshots.len(), 1);
        let coords: Vec<ChunkCoord> = snapshots
            .iter()
            .map(|s| ChunkCoord {
                x: s.coord.x,
                y: s.coord.y,
            })
            .collect();
        for snapshot in &snapshots {
            store.write_snapshot(snapshot.clone());
        }
        runtime.mark_chunk_snapshots_persisted(&coords);

        let visible = store
            .read_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible snapshot stored");
        assert_eq!(visible.coord, ChunkCoordDto { x: 4, y: 4 });
        assert!(visible.tiles.iter().any(|tile| tile.local_index == 11));

        // After marking persisted with no further events and within the 30s ceiling,
        // the registry must skip every chunk.
        assert_eq!(runtime.collect_chunk_snapshots().len(), 0);
    }

    #[tokio::test]
    async fn hydrate_from_stores_restores_layered_chunk_snapshot_without_event_replay() {
        use sim_core::tile::{TileBase, TileRecord, TileSurface};

        let mut authoring_chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        authoring_chunk
            .set_tile_record(
                0,
                TileRecord {
                    surface: TileSurface::Street,
                    road_mask: Some(5),
                    ..TileRecord::default()
                },
            )
            .unwrap();
        authoring_chunk
            .set_tile_record(
                7,
                TileRecord {
                    base: TileBase::Water,
                    ..TileRecord::default()
                },
            )
            .unwrap();
        let snapshot =
            build_chunk_snapshot("abutown-main", &authoring_chunk, ChunkActivity::Active);

        let mut snapshot_store = InMemoryChunkSnapshotStore::default();
        ChunkSnapshotStore::write_snapshot(&mut snapshot_store, snapshot)
            .await
            .unwrap();

        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(snapshot_store),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            &empty_test_network(),
        )
        .await
        .unwrap();

        let restored = runtime.chunk_snapshot(ChunkCoord { x: 4, y: 4 }).unwrap();
        assert_eq!(restored.chunk_version, 2);
        let tiles: std::collections::HashMap<u16, _> = restored
            .tiles
            .iter()
            .map(|tile| (tile.local_index, tile))
            .collect();
        assert_eq!(tiles.get(&0).unwrap().surface, TileSurfaceDto::Street);
        assert_eq!(tiles.get(&7).unwrap().base, TileBaseDto::Water);
        assert_eq!(restored.chunk_state, abutown_protocol::ChunkStateDto::Active);
    }

    #[tokio::test]
    async fn hydrate_from_stores_falls_back_to_seed_when_no_snapshot() {
        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            &empty_test_network(),
        )
        .await
        .unwrap();

        let snap = runtime.chunk_snapshot(ChunkCoord { x: 4, y: 4 }).unwrap();
        assert_eq!(snap.chunk_version, 0);
        assert_eq!(snap.tile_count, 1024);
        assert!(snap.tiles.iter().any(|tile| tile.base == TileBaseDto::Water));
        assert!(
            snap.tiles
                .iter()
                .any(|tile| tile.surface == TileSurfaceDto::Street)
        );
    }

    #[tokio::test]
    async fn hydrate_seeds_fresh_mobility_when_store_is_empty() {
        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            &empty_test_network(),
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), 0);
        assert_eq!(runtime.mobility_agent_count_for_test(), 20);
        assert_eq!(runtime.mobility_vehicle_count_for_test(), 4);
    }

    #[tokio::test]
    async fn hydrate_restores_mobility_from_store_when_present() {
        use sim_core::mobility::api::tick_mobility as api_tick;
        use sim_core::mobility::{extract_from_world, seed};

        let (mut authored, mut sched) = seed::initial_world();
        // Advance one tick so the persisted state differs from a fresh seed.
        let _ = api_tick(&mut authored, &mut sched);
        let persisted_tick = sim_core::mobility::api::tick(&authored);
        let authored_snap = extract_from_world(&authored);

        let mut mobility_store = InMemoryMobilitySnapshotStore::default();
        MobilitySnapshotStore::write(
            &mut mobility_store,
            "abutown-main",
            persisted_tick,
            &authored_snap,
        )
        .await
        .unwrap();

        let (runtime, _, _) = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(mobility_store),
            &empty_test_network(),
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), persisted_tick);
    }
}
