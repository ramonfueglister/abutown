use abutown_protocol::ChunkCoordDto;
use serde::{Deserialize, Serialize};

// `Ord` is x-major lexicographic (field-declaration order). All ChunkCoord
// sort sites — snapshot key order, mobility serde tuple lists — use this
// single ordering for byte-stable output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

impl From<ChunkCoord> for ChunkCoordDto {
    fn from(value: ChunkCoord) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

impl From<&ChunkCoordDto> for ChunkCoord {
    fn from(value: &ChunkCoordDto) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StableEntityId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VehicleId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StopId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RouteId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Serialize, Deserialize)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}
