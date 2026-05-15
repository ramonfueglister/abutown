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

impl From<TileKindDto> for TileKind {
    fn from(value: TileKindDto) -> Self {
        match value {
            TileKindDto::Grass => Self::Grass,
            TileKindDto::Water => Self::Water,
            TileKindDto::Road => Self::Road,
            TileKindDto::BuildingFootprint => Self::BuildingFootprint,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_kind_converts_from_protocol_kind() {
        assert_eq!(TileKind::from(TileKindDto::Grass), TileKind::Grass);
        assert_eq!(TileKind::from(TileKindDto::Water), TileKind::Water);
        assert_eq!(TileKind::from(TileKindDto::Road), TileKind::Road);
        assert_eq!(
            TileKind::from(TileKindDto::BuildingFootprint),
            TileKind::BuildingFootprint
        );
    }
}
