use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, ClientCommandDto, CommandAcceptedDto, HealthResponse,
    MobilityDeltaDto, MobilitySnapshotDto, PROTOCOL_VERSION, ServerHelloDto, ServerMessageDto,
    SetTileKindCommandDto, TileKindSetEventDto, TilePulseDeltaDto, WorldEventDto, WorldId,
    WorldSummaryDto,
};
use sim_core::{
    chunk::{Chunk, ChunkError, EventApplyError, SnapshotDecodeError},
    events::{InMemoryWorldEventStore, WorldEventStore, WorldEventStoreError},
    ids::ChunkCoord,
    mobility::{MobilityWorld, build_mobility_delta_dto, build_mobility_snapshot_dto},
    persistence::{
        ChunkSnapshotStore, ChunkSnapshotStoreError, MobilitySnapshotStore,
    },
    scheduler::ChunkActivity,
    tile::TileKind,
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
}

use crate::{
    chunk_registry::{ChunkMutationError, ChunkRegistry},
    commands::{AppliedCommand, CommandRejection},
};

const WORLD_ID: &str = "abutown-main";
const CHUNK_SIZE: u16 = 32;
const SEEDED_CHUNKS: [ChunkCoord; 3] = [
    ChunkCoord { x: 4, y: 4 },
    ChunkCoord { x: 5, y: 4 },
    ChunkCoord { x: 4, y: 5 },
];
const PULSE_STRIDE: u64 = 37;
pub const TICK_PERIOD_MS: u32 = 100;

pub const SEED_DENSITY: sim_core::mobility::seed::SeedDensity =
    sim_core::mobility::seed::SeedDensity {
        pedestrians_per_corridor: 6,
        cars_per_arterial: 17,
        trams_total: 4,
    };

pub struct SimulationRuntime {
    world_id: WorldId,
    registry: ChunkRegistry,
    mobility: MobilityWorld,
    event_store: Box<dyn WorldEventStore + Send + Sync>,
    event_count: usize,
    tick: u64,
    version: u64,
}

impl std::fmt::Debug for SimulationRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimulationRuntime")
            .field("world_id", &self.world_id)
            .field("registry", &self.registry)
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
        let mut registry = ChunkRegistry::new(CHUNK_SIZE);
        for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate() {
            let mut chunk = Chunk::new(coord, CHUNK_SIZE);
            let seed_index = (offset as u16) * 17;
            let seed_kind = match offset {
                0 => TileKind::Road,
                1 => TileKind::Water,
                _ => TileKind::BuildingFootprint,
            };
            chunk
                .set_tile_kind(seed_index, seed_kind)
                .expect("seed tile index is valid for seeded chunk");

            let activity = if offset == 0 {
                ChunkActivity::Active
            } else {
                ChunkActivity::Warm
            };
            registry.insert_chunk(chunk, activity);
        }

        Self {
            world_id: Self::default_world_id(),
            registry,
            mobility: MobilityWorld::default(),
            event_store,
            event_count: 0,
            tick: 0,
            version: 0,
        }
    }

    /// Build an in-memory runtime whose mobility world is seeded from the
    /// shared city network descriptor instead of the tiny developer seed.
    pub fn new_from_network(network: &sim_core::city_network::CityNetwork) -> Self {
        let mut runtime = Self::new();
        runtime.mobility = sim_core::mobility::seed::from_network(network, SEED_DENSITY);
        runtime
    }

    pub fn set_mobility_for_test(&mut self, mobility: MobilityWorld) {
        self.mobility = mobility;
    }

    pub fn override_world_id_for_test(&mut self, world_id: &str) {
        self.world_id = WorldId(world_id.to_string());
    }

    pub fn next_mobility_delta_for_test(&mut self) -> MobilityDeltaDto {
        self.next_mobility_delta()
    }

    pub fn mobility_world_clone_for_test(&self) -> MobilityWorld {
        self.mobility.clone()
    }

    pub fn mobility_tick(&self) -> u64 {
        self.mobility.tick()
    }

    /// Hydrate a runtime from the given stores.
    ///
    /// Returns `(runtime, snapshot_store, mobility_snapshot_store)` so the
    /// caller (AppState) can place the stores under its own `Arc<Mutex<…>>`.
    pub async fn hydrate_from_stores(
        event_store: Box<dyn WorldEventStore + Send + Sync>,
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
        let fallback_mobility = || {
            if network.arterial_paths.is_empty() && network.pedestrian_corridors.is_empty() {
                sim_core::mobility::seed::tiny_world()
            } else {
                sim_core::mobility::seed::from_network(network, SEED_DENSITY)
            }
        };
        let mobility = match mobility_snapshot_store
            .read(&world_id.0)
            .await
            .map_err(HydrationError::Mobility)?
        {
            Some((_tick, world)) => world,
            None => fallback_mobility(),
        };
        let mut registry = ChunkRegistry::new(CHUNK_SIZE);

        for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate() {
            let snap = snapshot_store
                .read_snapshot(coord)
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
                    let mut chunk = Chunk::new(coord, CHUNK_SIZE);
                    let seed_index = (offset as u16) * 17;
                    let seed_kind = match offset {
                        0 => TileKind::Road,
                        1 => TileKind::Water,
                        _ => TileKind::BuildingFootprint,
                    };
                    chunk
                        .set_tile_kind(seed_index, seed_kind)
                        .map_err(HydrationError::Chunk)?;
                    let activity = if offset == 0 {
                        ChunkActivity::Active
                    } else {
                        ChunkActivity::Warm
                    };
                    let v = chunk.version();
                    (chunk, v, activity)
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

            registry.insert_hydrated(chunk, chunk_version, activity);
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
            registry,
            mobility,
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
            chunk_size: self.registry.chunk_size(),
            loaded_chunks: self
                .registry
                .loaded_coords()
                .into_iter()
                .map(ChunkCoordDto::from)
                .collect(),
            tick_period_ms: TICK_PERIOD_MS,
        }
    }

    pub fn chunk_snapshot(&self, coord: ChunkCoord) -> Option<ChunkSnapshotDto> {
        self.registry.chunk_snapshot(&self.world_id, coord)
    }

    pub fn mobility_snapshot(&self) -> MobilitySnapshotDto {
        build_mobility_snapshot_dto(&self.world_id, self.mobility.tick(), &self.mobility)
    }

    pub fn next_mobility_delta(&mut self) -> MobilityDeltaDto {
        let per_chunk = self.mobility.tick_mobility();
        // Glue: flatten per-chunk map back into a global delta so the old
        // broadcast path keeps working until Task 8 deletes it.
        let mut changed_agents = Vec::new();
        let mut changed_vehicles = Vec::new();
        for delta in per_chunk.into_values() {
            changed_agents.extend(delta.changed_agents);
            changed_vehicles.extend(delta.changed_vehicles);
        }
        let delta = sim_core::mobility::MobilityDelta {
            changed_agents,
            changed_vehicles,
        };
        build_mobility_delta_dto(&self.world_id, self.mobility.tick(), &self.mobility, &delta)
    }

    pub fn filtered_mobility_delta_from_dto(
        &self,
        raw_delta_dto: &abutown_protocol::MobilityDeltaDto,
        subscription: &std::collections::HashSet<sim_core::ids::ChunkCoord>,
        last_visible_agents: &mut std::collections::HashSet<abutown_protocol::EntityId>,
        last_visible_vehicles: &mut std::collections::HashSet<abutown_protocol::EntityId>,
    ) -> abutown_protocol::MobilityDeltaDto {
        let changed_agents: Vec<sim_core::mobility::AgentRecord> = raw_delta_dto
            .changed_agents
            .iter()
            .filter_map(|dto| {
                self.mobility
                    .agent(&sim_core::ids::AgentId(dto.id.0.clone()))
            })
            .collect();
        let changed_vehicles: Vec<sim_core::mobility::VehicleRecord> = raw_delta_dto
            .changed_vehicles
            .iter()
            .filter_map(|dto| {
                self.mobility
                    .vehicle(&sim_core::ids::VehicleId(dto.id.0.clone()))
            })
            .collect();
        let delta = sim_core::mobility::MobilityDelta {
            changed_agents,
            changed_vehicles,
        };
        sim_core::mobility::build_filtered_mobility_delta_dto(
            &self.world_id,
            self.mobility.tick(),
            &self.mobility,
            &delta,
            subscription,
            last_visible_agents,
            last_visible_vehicles,
        )
    }

    pub fn synthetic_mobility_delta_for_subscription(
        &self,
        subscription: &std::collections::HashSet<sim_core::ids::ChunkCoord>,
        last_visible_agents: &mut std::collections::HashSet<abutown_protocol::EntityId>,
        last_visible_vehicles: &mut std::collections::HashSet<abutown_protocol::EntityId>,
    ) -> abutown_protocol::MobilityDeltaDto {
        let empty_delta = sim_core::mobility::MobilityDelta {
            changed_agents: vec![],
            changed_vehicles: vec![],
        };
        sim_core::mobility::build_filtered_mobility_delta_dto(
            &self.world_id,
            self.mobility.tick(),
            &self.mobility,
            &empty_delta,
            subscription,
            last_visible_agents,
            last_visible_vehicles,
        )
    }

    /// Forward a per-connection chunk-subscription delta into the mobility
    /// world's `ChunkSubscribers` resource.
    pub fn apply_subscription_diff<'a, A, R>(&mut self, added: A, removed: R)
    where
        A: IntoIterator<Item = &'a sim_core::ids::ChunkCoord>,
        R: IntoIterator<Item = &'a sim_core::ids::ChunkCoord>,
    {
        self.mobility.apply_subscription_diff(added, removed);
    }

    /// Expose the per-chunk tick result for the new fan-out path (Task 7).
    /// This is the authoritative ticker for mobility; `next_mobility_delta`
    /// (used by the legacy broadcast path) MUST NOT also be called in the same
    /// interval — it would tick mobility twice.  See `spawn_delta_loop` in app.rs.
    pub fn tick_world_mobility(
        &mut self,
    ) -> std::collections::HashMap<
        sim_core::ids::ChunkCoord,
        sim_core::mobility::MobilityChunkDelta,
    > {
        self.mobility.tick_mobility()
    }

    pub fn next_server_messages(&mut self) -> Vec<ServerMessageDto> {
        vec![
            self.next_pulse(),
            ServerMessageDto::MobilityDelta(self.next_mobility_delta()),
        ]
    }

    /// Collect all chunk snapshots that are due for persistence.
    /// Does NOT mark them as persisted (call `mark_chunk_snapshots_persisted` after writing).
    pub fn collect_chunk_snapshots(&self) -> Vec<ChunkSnapshotDto> {
        self.registry.collect_snapshots(&self.world_id)
    }

    /// Mark the given chunks as persisted (clear dirty state, update timestamps).
    pub fn mark_chunk_snapshots_persisted(&mut self, coords: &[ChunkCoord]) {
        self.registry.mark_snapshots_persisted(coords);
    }

    /// Clone the mobility world so persist functions can release the runtime
    /// read-lock before performing DB writes.
    pub fn mobility_world_clone_for_persist(&self) -> MobilityWorld {
        self.mobility.clone()
    }

    /// Borrow the mobility world — usable inside a runtime read/write lock block
    /// without paying the clone cost.
    pub fn mobility(&self) -> &MobilityWorld {
        &self.mobility
    }

    /// Return the number of active WS subscribers for a chunk.
    pub fn chunk_subscriber_count(&self, chunk: sim_core::ids::ChunkCoord) -> u8 {
        self.mobility.chunk_subscriber_count(chunk)
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
        let plan = self
            .registry
            .plan_set_tile_kind(coord, command.local_index, kind)
            .map_err(|error| match error {
                ChunkMutationError::ChunkNotLoaded { coord } => CommandRejection {
                    world_id: Some(command.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "chunk_not_loaded",
                    message: format!("chunk {}:{} is not loaded", coord.x, coord.y),
                },
                ChunkMutationError::TileOutOfBounds { index, tile_count } => CommandRejection {
                    world_id: Some(command.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "tile_out_of_bounds",
                    message: format!("tile index {index} is outside chunk tile count {tile_count}"),
                },
                ChunkMutationError::NoStateChange { coord, local_index } => CommandRejection {
                    world_id: Some(command.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "no_state_change",
                    message: format!(
                        "tile {local_index} in chunk {}:{} already has the requested kind",
                        coord.x, coord.y
                    ),
                },
            })?;

        let event_id = format!("event:{}", uuid::Uuid::now_v7());
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id,
            command_id: command.command_id.clone(),
            world_id: self.world_id.clone(),
            tick: self.tick,
            version: plan.version,
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
        self.registry
            .apply_set_tile_kind(plan)
            .expect("planned mutation should apply after event append");

        Ok(self.build_accepted(command.command_id, event))
    }

    pub fn hello(&self) -> ServerMessageDto {
        ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            chunk_size: self.registry.chunk_size(),
        })
    }

    pub fn next_pulse(&mut self) -> ServerMessageDto {
        self.tick += 1;
        self.version += 1;
        let loaded_coords = self.registry.loaded_coords();
        let coord = loaded_coords[((self.tick - 1) as usize) % loaded_coords.len()];
        let tile_count = u64::from(
            self.registry
                .tile_count(coord)
                .expect("pulse chunk should be loaded"),
        );
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
        self.mobility.tick()
    }
    pub fn mobility_agent_count_for_test(&self) -> usize {
        self.mobility.snapshot().agents.len()
    }
    pub fn mobility_vehicle_count_for_test(&self) -> usize {
        self.mobility.snapshot().vehicles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abutown_protocol::{ChunkStateDto, TileKindDto};
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
        assert_eq!(
            summary.loaded_chunks,
            vec![
                ChunkCoordDto { x: 4, y: 4 },
                ChunkCoordDto { x: 5, y: 4 },
                ChunkCoordDto { x: 4, y: 5 },
            ]
        );
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
        assert!(runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).is_none());
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
        assert_eq!(first.coord, ChunkCoordDto { x: 4, y: 4 });
        assert!(first.local_index < 1024);
        assert_eq!(second.tick, 2);
        assert_eq!(second.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(third.tick, 3);
        assert_eq!(third.coord, ChunkCoordDto { x: 4, y: 5 });
        assert_eq!(fourth.tick, 4);
        assert_eq!(fourth.coord, ChunkCoordDto { x: 4, y: 4 });
    }

    #[tokio::test]
    async fn runtime_collects_chunk_snapshots_and_marks_persisted() {
        use sim_core::persistence::InMemoryChunkSnapshotStore;

        let mut runtime = SimulationRuntime::new();
        let mut store = InMemoryChunkSnapshotStore::default();

        let snapshots = runtime.collect_chunk_snapshots();
        assert_eq!(snapshots.len(), 3);
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
        assert_eq!(visible.tiles.len(), 1);

        let east = store
            .read_snapshot(ChunkCoord { x: 5, y: 4 })
            .expect("east snapshot stored");
        assert_eq!(east.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(east.tiles.len(), 1);

        // After marking persisted with no further events and within the 30s ceiling,
        // the registry must skip every chunk.
        assert_eq!(runtime.collect_chunk_snapshots().len(), 0);

        // A new event on one chunk re-arms only that chunk for the next collect.
        runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:persist-test:1".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect("command should apply");

        let next_snapshots = runtime.collect_chunk_snapshots();
        assert_eq!(next_snapshots.len(), 1);
        for snapshot in &next_snapshots {
            store.write_snapshot(snapshot.clone());
        }
        let next_coords: Vec<ChunkCoord> = next_snapshots
            .iter()
            .map(|s| ChunkCoord {
                x: s.coord.x,
                y: s.coord.y,
            })
            .collect();
        runtime.mark_chunk_snapshots_persisted(&next_coords);

        assert_eq!(
            store
                .read_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("visible snapshot reflects new event")
                .tiles
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn runtime_applies_set_tile_kind_command_and_appends_event() {
        let mut runtime = SimulationRuntime::new();

        let applied = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:test:1".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect("command should apply");

        let abutown_protocol::WorldEventDto::TileKindSet(event) = &applied.event;
        assert!(event.event_id.starts_with("event:"));
        assert_eq!(event.command_id, "command:test:1");
        assert_eq!(event.version, 2);
        assert_eq!(event.local_index, 11);
        assert_eq!(event.kind, abutown_protocol::TileKindDto::Water);
        assert_eq!(runtime.event_count(), 1);

        let snapshot = runtime
            .chunk_snapshot(sim_core::ids::ChunkCoord { x: 4, y: 4 })
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
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
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
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
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

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:test:4".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Grass,
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
        let mut authoring_chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        authoring_chunk.set_tile_kind(0, TileKind::Road).unwrap();
        let snapshot =
            build_chunk_snapshot("abutown-main", &authoring_chunk, ChunkActivity::Active);

        let mut snapshot_store = InMemoryChunkSnapshotStore::default();
        ChunkSnapshotStore::write_snapshot(&mut snapshot_store, snapshot)
            .await
            .unwrap();

        // Tail event after the snapshot: tile 7 = Water at chunk_version 2.
        let tail_event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:tail".to_string(),
            command_id: "command:tail".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            tick: 2,
            version: 2,
            coord: ChunkCoordDto { x: 4, y: 4 },
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
            &empty_test_network(),
        )
        .await
        .unwrap();

        let restored = runtime.chunk_snapshot(ChunkCoord { x: 4, y: 4 }).unwrap();
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
        assert_eq!(
            snap.chunk_version, 1,
            "seeded chunk has one tile mutation by default"
        );
    }

    #[tokio::test]
    async fn runtime_rejects_store_failure_without_mutating_chunk() {
        let mut runtime = SimulationRuntime::new_with_event_store(Box::new(
            sim_core::events::FailingWorldEventStore::new("database offline"),
        ));

        let before = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("chunk exists");

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:test:store-failure".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
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
                .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
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
            world_id: WorldId("abutown-main".to_string()),
            command_id: "command:dup".to_string(),
            coord: ChunkCoordDto { x: 4, y: 4 },
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
            world_id: WorldId("abutown-main".to_string()),
            tick: 7,
            version: 7,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 0,
            kind: TileKindDto::Water,
        });
        let mut runtime =
            SimulationRuntime::new_with_event_store(Box::new(RaceyEventStore::new(winner.clone())));

        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            command_id: "command:race".to_string(),
            coord: ChunkCoordDto { x: 4, y: 4 },
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
        use sim_core::mobility::seed;

        let mut authored = seed::initial_world();
        // Advance one tick so the persisted state differs from a fresh seed.
        let _ = authored.tick_mobility();
        let persisted_tick = authored.tick();

        let mut mobility_store = InMemoryMobilitySnapshotStore::default();
        MobilitySnapshotStore::write(
            &mut mobility_store,
            "abutown-main",
            persisted_tick,
            &authored,
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
