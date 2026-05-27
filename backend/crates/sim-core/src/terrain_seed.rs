use serde::{Deserialize, Serialize};

use crate::ids::ChunkCoord;
use crate::tile::{LayeredTileRecord, TileBase, TileCover, TileSurface};

const ZURICH_LAYERED_TERRAIN_SEED_JSON: &str =
    include_str!("../../../../data/city/zurich-layered-terrain-seed.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayeredTerrainSeed {
    pub version: u32,
    pub world_id: String,
    pub width: u32,
    pub height: u32,
    pub chunk_size: u16,
    pub tiles: Vec<SeedTile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedTile {
    pub x: u32,
    pub y: u32,
    pub base: TileBase,
    pub surface: TileSurface,
    pub cover: TileCover,
    pub display: Option<String>,
    pub zone_id: Option<String>,
    pub road_mask: Option<u8>,
    pub rail_mask: Option<u8>,
    pub version: u64,
}

impl SeedTile {
    pub fn to_record(&self) -> LayeredTileRecord {
        LayeredTileRecord {
            base: self.base,
            surface: self.surface,
            cover: self.cover,
            display: self.display.clone(),
            zone_id: self.zone_id.clone(),
            road_mask: self.road_mask,
            rail_mask: self.rail_mask,
            version: self.version,
        }
    }
}

pub fn load_zurich_layered_terrain_seed() -> Result<LayeredTerrainSeed, serde_json::Error> {
    serde_json::from_str(ZURICH_LAYERED_TERRAIN_SEED_JSON)
}

pub fn validate_seed(seed: &LayeredTerrainSeed) -> Vec<String> {
    let mut errors = Vec::new();

    if seed.width == 0 {
        errors.push("dimensions:width_zero".to_string());
    }
    if seed.height == 0 {
        errors.push("dimensions:height_zero".to_string());
    }

    let expected_tile_count = usize::try_from(u64::from(seed.width) * u64::from(seed.height)).ok();

    match expected_tile_count {
        Some(expected) if seed.tiles.len() == expected => {}
        Some(expected) => errors.push(format!(
            "tile_count:expected:{expected}:actual:{}",
            seed.tiles.len()
        )),
        None => errors.push("tile_count:overflow".to_string()),
    }

    if seed.chunk_size == 0 {
        errors.push("chunk_partitioning:chunk_size_zero".to_string());
    } else {
        let chunk_size = u32::from(seed.chunk_size);
        if seed.width % chunk_size != 0 {
            errors.push(format!(
                "chunk_partitioning:width:{}:chunk_size:{}",
                seed.width, seed.chunk_size
            ));
        }
        if seed.height % chunk_size != 0 {
            errors.push(format!(
                "chunk_partitioning:height:{}:chunk_size:{}",
                seed.height, seed.chunk_size
            ));
        }
    }

    if let Some(width) = usize::try_from(seed.width).ok().filter(|width| *width != 0) {
        for (index, tile) in seed.tiles.iter().enumerate() {
            let expected_x = u32::try_from(index % width).unwrap_or(u32::MAX);
            let expected_y = u32::try_from(index / width).unwrap_or(u32::MAX);
            if tile.x != expected_x || tile.y != expected_y {
                errors.push(format!(
                    "tile:{}:{}:expected:{}:{}",
                    tile.x, tile.y, expected_x, expected_y
                ));
            }
        }
    }

    for tile in &seed.tiles {
        for error in tile.to_record().validate() {
            errors.push(format!("tile:{}:{}:{error:?}", tile.x, tile.y));
        }
    }

    errors
}

pub fn chunk_tiles_from_seed(
    seed: &LayeredTerrainSeed,
    coord: ChunkCoord,
) -> Option<Vec<LayeredTileRecord>> {
    if coord.x < 0 || coord.y < 0 || seed.chunk_size == 0 {
        return None;
    }

    let chunk_size = u32::from(seed.chunk_size);
    let start_x = u32::try_from(coord.x).ok()?.checked_mul(chunk_size)?;
    let start_y = u32::try_from(coord.y).ok()?.checked_mul(chunk_size)?;
    let end_x = start_x.checked_add(chunk_size)?;
    let end_y = start_y.checked_add(chunk_size)?;

    if end_x > seed.width || end_y > seed.height {
        return None;
    }

    let width = usize::try_from(seed.width).ok()?;
    let chunk_tile_count = usize::from(seed.chunk_size) * usize::from(seed.chunk_size);
    let mut tiles = Vec::with_capacity(chunk_tile_count);

    for y in start_y..end_y {
        for x in start_x..end_x {
            let index = usize::try_from(y)
                .ok()?
                .checked_mul(width)?
                .checked_add(usize::try_from(x).ok()?)?;
            let tile = seed.tiles.get(index)?;
            if tile.x != x || tile.y != y {
                return None;
            }
            tiles.push(tile.to_record());
        }
    }

    Some(tiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::{TileBase, TileCover, TileSurface};

    #[test]
    fn loads_zurich_layered_seed_from_repo_data() {
        let seed = load_zurich_layered_terrain_seed().expect("seed JSON deserializes");

        assert_eq!(seed.version, 1);
        assert_eq!(seed.world_id, "zurich-river-city-v1");
        assert_eq!(seed.width, 256);
        assert_eq!(seed.height, 256);
        assert_eq!(seed.chunk_size, 32);
        assert_eq!(seed.tiles.len(), 256 * 256);
        assert!(validate_seed(&seed).is_empty());
    }

    #[test]
    fn converts_dense_seed_to_chunk_tiles() {
        let seed = load_zurich_layered_terrain_seed().expect("seed JSON deserializes");

        let tiles = chunk_tiles_from_seed(&seed, ChunkCoord { x: 0, y: 0 })
            .expect("origin chunk exists");

        assert_eq!(tiles.len(), 32 * 32);
    }

    #[test]
    fn validate_seed_rejects_shuffled_coordinates() {
        let mut seed = test_seed(2, 2, 2);
        seed.tiles.swap(0, 1);

        let errors = validate_seed(&seed);

        assert!(errors.contains(&"tile:1:0:expected:0:0".to_string()));
        assert!(errors.contains(&"tile:0:0:expected:1:0".to_string()));
    }

    #[test]
    fn validate_seed_rejects_invalid_physical_layer_combinations() {
        let mut seed = test_seed(1, 1, 1);
        seed.tiles[0].base = TileBase::Water;
        seed.tiles[0].surface = TileSurface::Street;
        seed.tiles[0].cover = TileCover::Building;
        seed.tiles[0].road_mask = None;

        let errors = validate_seed(&seed);

        assert!(errors.contains(&"tile:0:0:BuildingOnWater".to_string()));
        assert!(errors.contains(&"tile:0:0:CoverOnTransportSurface".to_string()));
        assert!(errors.contains(&"tile:0:0:RoadSurfaceWithoutRoadMask".to_string()));
    }

    #[test]
    fn chunk_tiles_from_seed_rejects_invalid_chunk_requests() {
        let seed = test_seed(2, 2, 2);

        assert!(chunk_tiles_from_seed(&seed, ChunkCoord { x: -1, y: 0 }).is_none());
        assert!(chunk_tiles_from_seed(&seed, ChunkCoord { x: 1, y: 0 }).is_none());

        let mut misaligned = seed.clone();
        misaligned.tiles[0].x = 1;
        assert!(chunk_tiles_from_seed(&misaligned, ChunkCoord { x: 0, y: 0 }).is_none());

        let mut missing = seed;
        missing.tiles.pop();
        assert!(chunk_tiles_from_seed(&missing, ChunkCoord { x: 0, y: 0 }).is_none());
    }

    fn test_seed(width: u32, height: u32, chunk_size: u16) -> LayeredTerrainSeed {
        let mut tiles = Vec::new();
        for y in 0..height {
            for x in 0..width {
                tiles.push(SeedTile {
                    x,
                    y,
                    base: TileBase::Grass,
                    surface: TileSurface::None,
                    cover: TileCover::None,
                    display: None,
                    zone_id: None,
                    road_mask: None,
                    rail_mask: None,
                    version: 0,
                });
            }
        }

        LayeredTerrainSeed {
            version: 1,
            world_id: "test".to_string(),
            width,
            height,
            chunk_size,
            tiles,
        }
    }
}
