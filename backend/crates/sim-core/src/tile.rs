use abutown_protocol::TileKindDto;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileKind {
    #[default]
    Grass,
    Water,
    Road,
    BuildingFootprint,
}

impl From<TileKind> for TileKindDto {
    fn from(value: TileKind) -> Self {
        match value {
            TileKind::Grass => Self::Grass,
            TileKind::Water => Self::Water,
            TileKind::Road => Self::Road,
            TileKind::BuildingFootprint => Self::BuildingFootprint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TileFlags {
    pub blocks_movement: bool,
    pub modified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TileRecord {
    pub kind: TileKind,
    pub flags: TileFlags,
    pub version: u64,
}
