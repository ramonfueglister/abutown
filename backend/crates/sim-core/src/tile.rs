use abutown_protocol::{LayeredTileDto, TileBaseDto, TileCoverDto, TileSurfaceDto};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileBase {
    #[default]
    Grass,
    Water,
    Riverbank,
    Forest,
    Park,
    Reserve,
    Plaza,
}

impl From<TileBase> for TileBaseDto {
    fn from(value: TileBase) -> Self {
        match value {
            TileBase::Grass => Self::Grass,
            TileBase::Water => Self::Water,
            TileBase::Riverbank => Self::Riverbank,
            TileBase::Forest => Self::Forest,
            TileBase::Park => Self::Park,
            TileBase::Reserve => Self::Reserve,
            TileBase::Plaza => Self::Plaza,
        }
    }
}

impl From<TileBaseDto> for TileBase {
    fn from(value: TileBaseDto) -> Self {
        match value {
            TileBaseDto::Grass => Self::Grass,
            TileBaseDto::Water => Self::Water,
            TileBaseDto::Riverbank => Self::Riverbank,
            TileBaseDto::Forest => Self::Forest,
            TileBaseDto::Park => Self::Park,
            TileBaseDto::Reserve => Self::Reserve,
            TileBaseDto::Plaza => Self::Plaza,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileSurface {
    #[default]
    None,
    Street,
    Bridge,
    Rail,
    RailCrossing,
}

impl From<TileSurface> for TileSurfaceDto {
    fn from(value: TileSurface) -> Self {
        match value {
            TileSurface::None => Self::None,
            TileSurface::Street => Self::Street,
            TileSurface::Bridge => Self::Bridge,
            TileSurface::Rail => Self::Rail,
            TileSurface::RailCrossing => Self::RailCrossing,
        }
    }
}

impl From<TileSurfaceDto> for TileSurface {
    fn from(value: TileSurfaceDto) -> Self {
        match value {
            TileSurfaceDto::None => Self::None,
            TileSurfaceDto::Street => Self::Street,
            TileSurfaceDto::Bridge => Self::Bridge,
            TileSurfaceDto::Rail => Self::Rail,
            TileSurfaceDto::RailCrossing => Self::RailCrossing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileCover {
    #[default]
    None,
    Building,
    Tree,
    Detail,
}

impl From<TileCover> for TileCoverDto {
    fn from(value: TileCover) -> Self {
        match value {
            TileCover::None => Self::None,
            TileCover::Building => Self::Building,
            TileCover::Tree => Self::Tree,
            TileCover::Detail => Self::Detail,
        }
    }
}

impl From<TileCoverDto> for TileCover {
    fn from(value: TileCoverDto) -> Self {
        match value {
            TileCoverDto::None => Self::None,
            TileCoverDto::Building => Self::Building,
            TileCoverDto::Tree => Self::Tree,
            TileCoverDto::Detail => Self::Detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayeredTileRecord {
    pub base: TileBase,
    pub surface: TileSurface,
    pub cover: TileCover,
    pub display: Option<String>,
    pub zone_id: Option<String>,
    pub road_mask: Option<u8>,
    pub rail_mask: Option<u8>,
    pub version: u64,
}

pub type TileRecord = LayeredTileRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileValidationError {
    BridgeWithoutWater,
    BuildingOnWater,
    CoverOnTransportSurface,
    RoadMaskWithoutRoadSurface,
    RailMaskWithoutRailSurface,
    RoadSurfaceWithoutRoadMask,
    RailSurfaceWithoutRailMask,
}

impl LayeredTileRecord {
    pub fn validate(&self) -> Vec<TileValidationError> {
        let mut errors = Vec::new();

        if self.surface == TileSurface::Bridge
            && !matches!(self.base, TileBase::Water | TileBase::Riverbank)
        {
            errors.push(TileValidationError::BridgeWithoutWater);
        }

        if self.cover == TileCover::Building && self.base == TileBase::Water {
            errors.push(TileValidationError::BuildingOnWater);
        }

        if matches!(self.cover, TileCover::Building | TileCover::Tree)
            && self.surface != TileSurface::None
        {
            errors.push(TileValidationError::CoverOnTransportSurface);
        }

        if self.road_mask.is_some() && !self.surface_accepts_road_mask() {
            errors.push(TileValidationError::RoadMaskWithoutRoadSurface);
        }

        if self.rail_mask.is_some() && !self.surface_accepts_rail_mask() {
            errors.push(TileValidationError::RailMaskWithoutRailSurface);
        }

        if self.road_mask.is_none() && self.surface_requires_road_mask() {
            errors.push(TileValidationError::RoadSurfaceWithoutRoadMask);
        }

        if self.rail_mask.is_none() && self.surface_requires_rail_mask() {
            errors.push(TileValidationError::RailSurfaceWithoutRailMask);
        }

        errors
    }

    pub fn to_dto(&self, local_index: u16) -> LayeredTileDto {
        LayeredTileDto {
            local_index,
            base: self.base.into(),
            surface: self.surface.into(),
            cover: self.cover.into(),
            display: self.display.clone(),
            zone_id: self.zone_id.clone(),
            road_mask: self.road_mask,
            rail_mask: self.rail_mask,
            version: self.version,
        }
    }

    fn surface_accepts_road_mask(&self) -> bool {
        matches!(
            self.surface,
            TileSurface::Street | TileSurface::Bridge | TileSurface::RailCrossing
        )
    }

    fn surface_accepts_rail_mask(&self) -> bool {
        matches!(self.surface, TileSurface::Rail | TileSurface::RailCrossing)
    }

    fn surface_requires_road_mask(&self) -> bool {
        self.surface_accepts_road_mask()
    }

    fn surface_requires_rail_mask(&self) -> bool {
        self.surface_accepts_rail_mask()
    }
}

impl From<LayeredTileDto> for LayeredTileRecord {
    fn from(value: LayeredTileDto) -> Self {
        Self {
            base: value.base.into(),
            surface: value.surface.into(),
            cover: value.cover.into(),
            display: value.display,
            zone_id: value.zone_id,
            road_mask: value.road_mask,
            rail_mask: value.rail_mask,
            version: value.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_tile_defaults_to_grass_with_empty_layers() {
        let tile = LayeredTileRecord::default();
        assert_eq!(tile.base, TileBase::Grass);
        assert_eq!(tile.surface, TileSurface::None);
        assert_eq!(tile.cover, TileCover::None);
        assert_eq!(tile.version, 0);
    }

    #[test]
    fn layered_tile_validation_rejects_invalid_physical_combinations() {
        let invalid = LayeredTileRecord {
            base: TileBase::Water,
            surface: TileSurface::Street,
            cover: TileCover::Building,
            display: Some("houses".to_string()),
            zone_id: Some("zone:test".to_string()),
            road_mask: Some(1),
            rail_mask: None,
            version: 0,
        };

        let errors = invalid.validate();
        assert!(errors.contains(&TileValidationError::BuildingOnWater));
        assert!(errors.contains(&TileValidationError::CoverOnTransportSurface));
    }

    #[test]
    fn layered_tile_validation_rejects_missing_required_transport_masks() {
        let road = LayeredTileRecord {
            surface: TileSurface::Street,
            ..LayeredTileRecord::default()
        };
        let rail = LayeredTileRecord {
            surface: TileSurface::Rail,
            ..LayeredTileRecord::default()
        };
        let crossing = LayeredTileRecord {
            surface: TileSurface::RailCrossing,
            ..LayeredTileRecord::default()
        };

        assert!(road
            .validate()
            .contains(&TileValidationError::RoadSurfaceWithoutRoadMask));
        assert!(rail
            .validate()
            .contains(&TileValidationError::RailSurfaceWithoutRailMask));

        let crossing_errors = crossing.validate();
        assert!(crossing_errors.contains(&TileValidationError::RoadSurfaceWithoutRoadMask));
        assert!(crossing_errors.contains(&TileValidationError::RailSurfaceWithoutRailMask));
    }
}
