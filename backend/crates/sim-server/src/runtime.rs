use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, HealthResponse, PROTOCOL_VERSION, ServerHelloDto,
    ServerMessageDto, TilePulseDeltaDto, WorldId, WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk, ids::ChunkCoord, persistence::build_chunk_snapshot, scheduler::ChunkActivity,
    tile::TileKind,
};

const WORLD_ID: &str = "abutown-main";
const CHUNK_SIZE: u16 = 32;
const VISIBLE_CHUNK_COORD: ChunkCoord = ChunkCoord { x: 4, y: 4 };
const PULSE_STRIDE: u64 = 37;

#[derive(Debug)]
pub struct SimulationRuntime {
    world_id: WorldId,
    chunk: Chunk,
    tick: u64,
    version: u64,
}

impl SimulationRuntime {
    pub fn new() -> Self {
        let mut chunk = Chunk::new(VISIBLE_CHUNK_COORD, CHUNK_SIZE);
        chunk
            .set_tile_kind(0, TileKind::Road)
            .expect("seed tile index is valid for visible chunk");

        Self {
            world_id: WorldId(WORLD_ID.to_string()),
            chunk,
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
            chunk_size: self.chunk.chunk_size(),
            loaded_chunks: vec![ChunkCoordDto {
                x: self.chunk.coord().x,
                y: self.chunk.coord().y,
            }],
        }
    }

    pub fn chunk_snapshot(&self, coord: ChunkCoord) -> Option<ChunkSnapshotDto> {
        if coord != self.chunk.coord() {
            return None;
        }

        Some(build_chunk_snapshot(
            &self.world_id.0,
            &self.chunk,
            ChunkActivity::Active,
        ))
    }

    pub fn hello(&self) -> ServerMessageDto {
        ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            chunk_size: self.chunk.chunk_size(),
        })
    }

    pub fn next_pulse(&mut self) -> ServerMessageDto {
        self.tick += 1;
        self.version += 1;
        let tile_count = u64::from(self.chunk.tile_count());
        let local_index = ((self.tick * PULSE_STRIDE) % tile_count) as u16;

        ServerMessageDto::TilePulse(TilePulseDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            tick: self.tick,
            version: self.version,
            coord: ChunkCoordDto {
                x: self.chunk.coord().x,
                y: self.chunk.coord().y,
            },
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

    #[test]
    fn runtime_produces_monotonic_pulses_inside_seed_chunk() {
        let mut runtime = SimulationRuntime::new();

        let first = runtime.next_pulse();
        let second = runtime.next_pulse();

        let ServerMessageDto::TilePulse(first) = first else {
            panic!("first message should be a tile pulse");
        };
        let ServerMessageDto::TilePulse(second) = second else {
            panic!("second message should be a tile pulse");
        };

        assert_eq!(first.tick, 1);
        assert_eq!(first.version, 1);
        assert_eq!(first.coord, ChunkCoordDto { x: 4, y: 4 });
        assert!(first.local_index < 1024);
        assert_eq!(second.tick, 2);
        assert_eq!(second.version, 2);
        assert!(second.local_index < 1024);
        assert_ne!(first.local_index, second.local_index);
    }
}
