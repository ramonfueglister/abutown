use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, HealthResponse, MobilityDeltaDto, MobilitySnapshotDto,
    PROTOCOL_VERSION, ServerHelloDto, ServerMessageDto, TilePulseDeltaDto, WorldId,
    WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    mobility::{MobilityWorld, build_mobility_delta_dto, build_mobility_snapshot_dto},
    scheduler::ChunkActivity,
    tile::TileKind,
};

use crate::chunk_registry::ChunkRegistry;

const WORLD_ID: &str = "abutown-main";
const CHUNK_SIZE: u16 = 32;
const SEEDED_CHUNKS: [ChunkCoord; 3] = [
    ChunkCoord { x: 4, y: 4 },
    ChunkCoord { x: 5, y: 4 },
    ChunkCoord { x: 4, y: 5 },
];
const PULSE_STRIDE: u64 = 37;

#[derive(Debug)]
pub struct SimulationRuntime {
    world_id: WorldId,
    registry: ChunkRegistry,
    mobility: MobilityWorld,
    tick: u64,
    version: u64,
}

impl SimulationRuntime {
    pub fn new() -> Self {
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
            world_id: WorldId(WORLD_ID.to_string()),
            registry,
            mobility: MobilityWorld::seeded_demo(),
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
        build_mobility_snapshot_dto(&self.world_id, self.mobility.tick(), self.mobility.snapshot())
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
}
