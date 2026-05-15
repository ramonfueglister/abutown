use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, ClientCommandDto, CommandAcceptedDto, HealthResponse,
    MobilityDeltaDto, MobilitySnapshotDto, PROTOCOL_VERSION, ServerHelloDto, ServerMessageDto,
    SetTileKindCommandDto, TileKindSetEventDto, TilePulseDeltaDto, WorldEventDto, WorldId,
    WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk,
    events::{InMemoryWorldEventStore, WorldEventStore},
    ids::ChunkCoord,
    mobility::{MobilityWorld, build_mobility_delta_dto, build_mobility_snapshot_dto},
    persistence::{ChunkSnapshotStore, ChunkSnapshotStoreError, InMemoryChunkSnapshotStore},
    scheduler::ChunkActivity,
    tile::TileKind,
};

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
            event_store,
            event_count: 0,
            tick: 0,
            version: 0,
        }
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
        build_mobility_snapshot_dto(
            &self.world_id,
            self.mobility.tick(),
            self.mobility.snapshot(),
        )
    }

    pub fn next_mobility_delta(&mut self) -> MobilityDeltaDto {
        let delta = self.mobility.tick_mobility();
        build_mobility_delta_dto(&self.world_id, self.mobility.tick(), delta)
    }

    pub fn next_server_messages(&mut self) -> Vec<ServerMessageDto> {
        vec![
            self.next_pulse(),
            ServerMessageDto::MobilityDelta(self.next_mobility_delta()),
        ]
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

    pub async fn stored_chunk_snapshot(
        &self,
        coord: ChunkCoord,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        self.snapshot_store.read_snapshot(coord).await
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    pub(crate) async fn apply_client_command(
        &mut self,
        command: ClientCommandDto,
    ) -> Result<AppliedCommand, CommandRejection> {
        match command {
            ClientCommandDto::SetTileKind(command) => self.apply_set_tile_kind(command).await,
        }
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
        self.event_store
            .append(event.clone())
            .await
            .map_err(|error| CommandRejection {
                world_id: Some(self.world_id.clone()),
                command_id: Some(command.command_id.clone()),
                code: error.code(),
                message: error.to_string(),
            })?;

        self.event_count += 1;
        self.registry
            .apply_set_tile_kind(plan)
            .expect("planned mutation should apply after event append");

        let response = CommandAcceptedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            command_id: command.command_id,
            event: event.clone(),
        };

        Ok(AppliedCommand { response, event })
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
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sim_core::persistence::ChunkSnapshotStoreError;

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
        assert_eq!(visible.dirty_tiles.len(), 1);

        let east = runtime
            .stored_chunk_snapshot(ChunkCoord { x: 5, y: 4 })
            .await
            .unwrap()
            .expect("east snapshot stored");
        assert_eq!(east.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(east.dirty_tiles.len(), 1);

        assert_eq!(runtime.persist_chunk_snapshots().await.unwrap(), 3);
        assert!(
            runtime
                .stored_chunk_snapshot(ChunkCoord { x: 4, y: 4 })
                .await
                .unwrap()
                .expect("visible snapshot remains stored")
                .dirty_tiles
                .is_empty()
        );
    }

    #[tokio::test]
    async fn runtime_keeps_dirty_tiles_when_snapshot_store_fails() {
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
                .dirty_tiles,
            before.dirty_tiles
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
        assert!(snapshot.dirty_tiles.iter().any(|tile| {
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
}
