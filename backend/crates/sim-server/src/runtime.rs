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
        ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryChunkSnapshotStore,
        InMemoryMobilitySnapshotStore, InMemoryRoadVehicleSnapshotStore, MobilitySnapshotStore,
        MobilitySnapshotStoreError, RoadVehicleSnapshotStore, RoadVehicleSnapshotStoreError,
    },
    road_vehicles::{self, RoadVehicleWorld, build_road_vehicle_delta_dto},
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
    #[error("road vehicle store error: {0}")]
    RoadVehicle(sim_core::persistence::RoadVehicleSnapshotStoreError),
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

pub struct SimulationRuntime {
    world_id: WorldId,
    registry: ChunkRegistry,
    mobility: MobilityWorld,
    snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
    road_vehicle_world: RoadVehicleWorld,
    road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>,
    event_store: Box<dyn WorldEventStore + Send>,
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
        Self::new_with_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
        )
    }

    pub fn default_world_id() -> WorldId {
        WorldId(WORLD_ID.to_string())
    }

    pub fn new_with_event_store(event_store: Box<dyn WorldEventStore + Send>) -> Self {
        Self::new_with_stores(event_store, Box::new(InMemoryChunkSnapshotStore::default()))
    }

    pub fn new_with_stores(
        event_store: Box<dyn WorldEventStore + Send>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
    ) -> Self {
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
            snapshot_store,
            mobility_snapshot_store: Box::new(InMemoryMobilitySnapshotStore::default()),
            road_vehicle_world: road_vehicles::seed::initial_road_vehicles(),
            road_vehicle_snapshot_store: Box::new(InMemoryRoadVehicleSnapshotStore::default()),
            event_store,
            event_count: 0,
            tick: 0,
            version: 0,
        }
    }

    pub fn new_with_all_stores(
        event_store: Box<dyn WorldEventStore + Send>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
    ) -> Self {
        let mut runtime = Self::new_with_stores(event_store, snapshot_store);
        runtime.mobility_snapshot_store = mobility_snapshot_store;
        runtime
    }

    pub fn new_with_full_stores(
        event_store: Box<dyn WorldEventStore + Send>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
        road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>,
    ) -> Self {
        let mut runtime =
            Self::new_with_all_stores(event_store, snapshot_store, mobility_snapshot_store);
        runtime.road_vehicle_snapshot_store = road_vehicle_snapshot_store;
        runtime
    }

    pub fn set_mobility_for_test(&mut self, mobility: MobilityWorld) {
        self.mobility = mobility;
    }

    pub fn set_road_vehicle_world_for_test(&mut self, world: RoadVehicleWorld) {
        self.road_vehicle_world = world;
    }

    pub fn road_vehicle_world_clone_for_test(&self) -> RoadVehicleWorld {
        self.road_vehicle_world.clone()
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

    pub async fn hydrate_from_stores(
        event_store: Box<dyn WorldEventStore + Send>,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send>,
        road_vehicle_snapshot_store: Box<dyn RoadVehicleSnapshotStore + Send>,
    ) -> Result<Self, HydrationError> {
        let world_id = Self::default_world_id();
        let mobility = match mobility_snapshot_store
            .read(&world_id.0)
            .await
            .map_err(HydrationError::Mobility)?
        {
            Some((_tick, world)) => world,
            None => sim_core::mobility::seed::initial_world(),
        };
        let road_vehicle_world = match road_vehicle_snapshot_store
            .read(&world_id.0)
            .await
            .map_err(HydrationError::RoadVehicle)?
        {
            Some((_tick, world)) => world,
            None => sim_core::road_vehicles::seed::initial_road_vehicles(),
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

        Ok(Self {
            world_id,
            registry,
            mobility,
            snapshot_store,
            mobility_snapshot_store,
            road_vehicle_world,
            road_vehicle_snapshot_store,
            event_store,
            event_count,
            tick: global_tick,
            version: global_version,
        })
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
        }
    }

    pub fn chunk_snapshot(&self, coord: ChunkCoord) -> Option<ChunkSnapshotDto> {
        self.registry.chunk_snapshot(&self.world_id, coord)
    }

    pub fn mobility_snapshot(&self) -> MobilitySnapshotDto {
        build_mobility_snapshot_dto(&self.world_id, self.mobility.tick(), &self.mobility)
    }

    pub fn next_mobility_delta(&mut self) -> MobilityDeltaDto {
        let delta = self.mobility.tick_mobility();
        build_mobility_delta_dto(&self.world_id, self.mobility.tick(), &self.mobility, &delta)
    }

    pub fn next_server_messages(&mut self) -> Vec<ServerMessageDto> {
        let mut messages = vec![
            self.next_pulse(),
            ServerMessageDto::MobilityDelta(self.next_mobility_delta()),
        ];
        let road_delta = self.road_vehicle_world.tick_road_vehicles();
        messages.push(ServerMessageDto::RoadVehicleDelta(
            build_road_vehicle_delta_dto(&self.world_id, &self.road_vehicle_world, &road_delta),
        ));
        messages
    }

    pub async fn persist_chunk_snapshots(&mut self) -> Result<usize, ChunkSnapshotStoreError> {
        let snapshots = self.registry.collect_snapshots(&self.world_id);
        let persisted_coords: Vec<ChunkCoord> = snapshots
            .iter()
            .map(|snapshot| ChunkCoord {
                x: snapshot.coord.x,
                y: snapshot.coord.y,
            })
            .collect();

        for snapshot in snapshots {
            self.snapshot_store.write_snapshot(snapshot).await?;
        }

        self.registry.mark_snapshots_persisted(&persisted_coords);
        Ok(persisted_coords.len())
    }

    pub async fn persist_mobility_snapshot(&mut self) -> Result<(), MobilitySnapshotStoreError> {
        self.mobility_snapshot_store
            .write(&self.world_id.0, self.mobility.tick(), &self.mobility)
            .await
    }

    pub async fn persist_road_vehicle_snapshot(
        &mut self,
    ) -> Result<(), RoadVehicleSnapshotStoreError> {
        self.road_vehicle_snapshot_store
            .write(
                &self.world_id.0,
                self.road_vehicle_world.tick(),
                &self.road_vehicle_world,
            )
            .await
    }

    pub fn road_vehicle_snapshot_dto(&self) -> abutown_protocol::RoadVehicleSnapshotDto {
        road_vehicles::build_road_vehicle_snapshot_dto(&self.world_id, &self.road_vehicle_world)
    }

    pub async fn stored_chunk_snapshot(
        &self,
        coord: ChunkCoord,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        self.snapshot_store.read_snapshot(coord).await
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
    use async_trait::async_trait;
    use sim_core::persistence::{ChunkSnapshotStoreError, build_chunk_snapshot};

    #[derive(Debug)]
    struct FailingChunkSnapshotStore;

    #[async_trait]
    impl ChunkSnapshotStore for FailingChunkSnapshotStore {
        async fn write_snapshot(
            &mut self,
            _snapshot: ChunkSnapshotDto,
        ) -> Result<(), ChunkSnapshotStoreError> {
            Err(ChunkSnapshotStoreError::unavailable("database offline"))
        }

        async fn read_snapshot(
            &self,
            _coord: ChunkCoord,
        ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
            Ok(None)
        }
    }

    fn tile_pulse(message: ServerMessageDto) -> TilePulseDeltaDto {
        let ServerMessageDto::TilePulse(delta) = message else {
            panic!("message should be a tile pulse");
        };
        delta
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
    async fn runtime_persists_loaded_chunk_snapshots_and_clears_dirty_state() {
        let mut runtime = SimulationRuntime::new();

        assert_eq!(runtime.persist_chunk_snapshots().await.unwrap(), 3);

        let visible = runtime
            .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .await
            .unwrap()
            .expect("visible snapshot stored");
        assert_eq!(visible.coord, ChunkCoordDto { x: 4, y: 4 });
        assert_eq!(visible.tiles.len(), 1);

        let east = runtime
            .stored_chunk_snapshot(ChunkCoord { x: 5, y: 4 })
            .await
            .unwrap()
            .expect("east snapshot stored");
        assert_eq!(east.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(east.tiles.len(), 1);

        // After the first persist with no further events and well within the
        // 30s snapshot ceiling, the registry must skip every chunk.
        assert_eq!(runtime.persist_chunk_snapshots().await.unwrap(), 0);

        // Previously-stored rows remain intact in the snapshot store.
        assert_eq!(
            runtime
                .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
                .await
                .unwrap()
                .expect("visible snapshot remains stored")
                .tiles
                .len(),
            1
        );

        // A new event on one chunk re-arms only that chunk for the next
        // persist.
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

        assert_eq!(runtime.persist_chunk_snapshots().await.unwrap(), 1);
        assert_eq!(
            runtime
                .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
                .await
                .unwrap()
                .expect("visible snapshot reflects new event")
                .tiles
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn runtime_keeps_dirty_state_when_snapshot_store_fails() {
        let mut runtime = SimulationRuntime::new_with_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(FailingChunkSnapshotStore),
        );
        let before = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible chunk loaded");

        let error = runtime.persist_chunk_snapshots().await.unwrap_err();

        assert_eq!(error.to_string(), "database offline");
        assert_eq!(
            runtime
                .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("visible chunk remains loaded")
                .tiles,
            before.tiles
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

        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(event_store),
            Box::new(snapshot_store),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryRoadVehicleSnapshotStore::default()),
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
        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryRoadVehicleSnapshotStore::default()),
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
    async fn runtime_persists_mobility_snapshot_and_reloads_through_store() {
        use sim_core::mobility::seed;
        use sim_core::persistence::InMemoryMobilitySnapshotStore;

        let store: Box<dyn MobilitySnapshotStore + Send> =
            Box::new(InMemoryMobilitySnapshotStore::default());
        let mut runtime = SimulationRuntime::new_with_all_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            store,
        );
        runtime.set_mobility_for_test(seed::initial_world());
        runtime.persist_mobility_snapshot().await.unwrap();

        let (tick, world) = runtime
            .mobility_snapshot_store
            .read(&runtime.world_id.0)
            .await
            .unwrap()
            .expect("snapshot exists");

        assert_eq!(tick, 0);
        assert_eq!(world, seed::initial_world());
    }

    #[tokio::test]
    async fn race_handler_returns_winner_when_append_reports_duplicate() {
        use abutown_protocol::{
            ChunkCoordDto, ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto,
            TileKindSetEventDto, WorldEventDto, WorldId,
        };
        use sim_core::persistence::InMemoryChunkSnapshotStore;

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
        let mut runtime = SimulationRuntime::new_with_stores(
            Box::new(RaceyEventStore::new(winner.clone())),
            Box::new(InMemoryChunkSnapshotStore::default()),
        );

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
        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryRoadVehicleSnapshotStore::default()),
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), 0);
        assert_eq!(runtime.mobility_agent_count_for_test(), 20);
        assert_eq!(runtime.mobility_vehicle_count_for_test(), 4);
    }

    #[tokio::test]
    async fn runtime_ticks_road_vehicles_and_persists_snapshot() {
        use sim_core::persistence::InMemoryRoadVehicleSnapshotStore;

        let mut runtime = SimulationRuntime::new_with_full_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryRoadVehicleSnapshotStore::default()),
        );

        let initial_tick = runtime.road_vehicle_world.tick();
        runtime.next_server_messages();
        assert_eq!(runtime.road_vehicle_world.tick(), initial_tick + 1);

        runtime.persist_road_vehicle_snapshot().await.unwrap();
        let stored = runtime
            .road_vehicle_snapshot_store
            .read(&runtime.world_id.0)
            .await
            .unwrap()
            .expect("persisted snapshot");
        assert_eq!(stored.0, runtime.road_vehicle_world.tick());
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

        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(mobility_store),
            Box::new(InMemoryRoadVehicleSnapshotStore::default()),
        )
        .await
        .unwrap();

        assert_eq!(runtime.mobility_tick_for_test(), persisted_tick);
    }

    #[tokio::test]
    async fn hydrate_restores_road_vehicles_from_store_when_present() {
        use sim_core::persistence::{InMemoryRoadVehicleSnapshotStore, RoadVehicleSnapshotStore};
        use sim_core::road_vehicles::seed;

        let mut store = InMemoryRoadVehicleSnapshotStore::default();
        let mut authored = seed::initial_road_vehicles();
        authored.tick_road_vehicles();
        let persisted_tick = authored.tick();
        RoadVehicleSnapshotStore::write(&mut store, "abutown-main", persisted_tick, &authored)
            .await
            .unwrap();

        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(store),
        )
        .await
        .unwrap();

        assert_eq!(runtime.road_vehicle_world.tick(), persisted_tick);
        assert_eq!(runtime.road_vehicle_world, authored);
    }

    #[tokio::test]
    async fn hydrate_seeds_road_vehicles_when_store_is_empty() {
        use sim_core::persistence::InMemoryRoadVehicleSnapshotStore;

        let runtime = SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryRoadVehicleSnapshotStore::default()),
        )
        .await
        .unwrap();

        assert!(runtime.road_vehicle_world.vehicles.len() >= 80);
    }
}
