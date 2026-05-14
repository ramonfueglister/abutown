use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoordDto {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStateDto {
    Asleep,
    Warm,
    Active,
    Hot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileKindDto {
    Grass,
    Water,
    Road,
    BuildingFootprint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub service: String,
    pub world_id: WorldId,
    pub ok: bool,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSummaryDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
    pub loaded_chunks: Vec<ChunkCoordDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileMutationDto {
    pub local_index: u16,
    pub kind: TileKindDto,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkSnapshotDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub coord: ChunkCoordDto,
    pub chunk_state: ChunkStateDto,
    pub chunk_version: u64,
    pub tile_count: u16,
    pub dirty_tiles: Vec<TileMutationDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    Error(ServerErrorDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerHelloDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TilePulseDeltaDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub version: u64,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerErrorDto {
    pub protocol_version: u16,
    pub world_id: Option<WorldId>,
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_serializes_versioned_world() {
        let response = HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        };

        let json = serde_json::to_string(&response).expect("health response serializes");

        assert_eq!(
            json,
            r#"{"service":"abutown-sim","world_id":"abutown-main","ok":true,"protocol_version":1}"#
        );
    }

    #[test]
    fn websocket_hello_serializes_with_type_tag() {
        let message = ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            chunk_size: 32,
        });

        let json = serde_json::to_string(&message).expect("hello serializes");

        assert_eq!(
            json,
            r#"{"type":"hello","protocol_version":1,"world_id":"abutown-main","chunk_size":32}"#
        );
    }

    #[test]
    fn websocket_tile_pulse_serializes_chunk_and_version() {
        let message = ServerMessageDto::TilePulse(TilePulseDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            tick: 7,
            version: 11,
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 231,
        });

        let json = serde_json::to_string(&message).expect("tile pulse serializes");

        assert_eq!(
            json,
            r#"{"type":"tile_pulse","protocol_version":1,"world_id":"abutown-main","tick":7,"version":11,"coord":{"x":0,"y":0},"local_index":231}"#
        );
    }
}
