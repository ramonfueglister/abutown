use std::collections::BTreeSet;

use abutown_protocol::ChunkSnapshotDto;
use thiserror::Error;

use crate::ids::ChunkCoord;
use crate::tile::TileRecord;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkError {
    #[error(
        "chunk size {chunk_size} creates {tile_count} tiles, which exceeds max tile count {max_tile_count}"
    )]
    InvalidChunkSize {
        chunk_size: u16,
        tile_count: usize,
        max_tile_count: usize,
    },
    #[error("tile index {index} is outside chunk tile count {tile_count}")]
    IndexOutOfBounds { index: u16, tile_count: u16 },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotDecodeError {
    #[error("snapshot tile_count {tile_count} is not a square number")]
    NonSquareTileCount { tile_count: u16 },
    #[error("tile index {index} is outside snapshot tile_count {tile_count}")]
    IndexOutOfBounds { index: u16, tile_count: u16 },
    #[error("chunk construction failed: {0}")]
    Chunk(ChunkError),
}

#[derive(Debug, Clone)]
pub struct Chunk {
    coord: ChunkCoord,
    chunk_size: u16,
    version: u64,
    tiles: Vec<TileRecord>,
    dirty: BTreeSet<u16>,
}

impl Chunk {
    pub fn new(coord: ChunkCoord, chunk_size: u16) -> Self {
        Self::try_new(coord, chunk_size).expect("chunk size must fit u16 tile indices")
    }

    pub fn try_new(coord: ChunkCoord, chunk_size: u16) -> Result<Self, ChunkError> {
        let tile_count = usize::from(chunk_size) * usize::from(chunk_size);
        if tile_count > usize::from(u16::MAX) {
            return Err(ChunkError::InvalidChunkSize {
                chunk_size,
                tile_count,
                max_tile_count: usize::from(u16::MAX),
            });
        }

        Ok(Self {
            coord,
            chunk_size,
            version: 0,
            tiles: vec![TileRecord::default(); tile_count],
            dirty: BTreeSet::new(),
        })
    }

    pub fn coord(&self) -> ChunkCoord {
        self.coord
    }

    pub fn chunk_size(&self) -> u16 {
        self.chunk_size
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn tile_count(&self) -> u16 {
        self.tiles.len() as u16
    }

    pub fn tile_at(&self, index: u16) -> Option<TileRecord> {
        self.tiles.get(usize::from(index)).cloned()
    }

    pub fn dirty_indices(&self) -> Vec<u16> {
        self.dirty.iter().copied().collect()
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub fn from_snapshot(snapshot: &ChunkSnapshotDto) -> Result<Self, SnapshotDecodeError> {
        let tile_count = snapshot.tile_count;
        let chunk_size = (tile_count as f64).sqrt() as u16;
        if usize::from(chunk_size) * usize::from(chunk_size) != usize::from(tile_count) {
            return Err(SnapshotDecodeError::NonSquareTileCount { tile_count });
        }

        let mut chunk = Self::try_new(
            ChunkCoord {
                x: snapshot.coord.x,
                y: snapshot.coord.y,
            },
            chunk_size,
        )
        .map_err(SnapshotDecodeError::Chunk)?;

        for tile in &snapshot.tiles {
            let slot = chunk
                .tiles
                .get_mut(usize::from(tile.local_index))
                .ok_or(SnapshotDecodeError::IndexOutOfBounds {
                    index: tile.local_index,
                    tile_count,
                })?;
            *slot = tile.clone().into();
        }
        chunk.version = snapshot.chunk_version;
        Ok(chunk)
    }

    pub fn set_tile_record(&mut self, index: u16, mut record: TileRecord) -> Result<(), ChunkError> {
        let tile_count = self.tile_count();
        let tile = self
            .tiles
            .get_mut(usize::from(index))
            .ok_or(ChunkError::IndexOutOfBounds { index, tile_count })?;

        let mut current_physical = tile.clone();
        current_physical.version = 0;
        record.version = 0;

        if current_physical != record {
            self.version += 1;
            record.version = self.version;
            *tile = record;
            self.dirty.insert(index);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::{TileBase, TileSurface};

    #[test]
    fn chunk_uses_dense_tiles_and_tracks_dirty_indices() {
        let mut chunk = Chunk::new(ChunkCoord { x: 2, y: -1 }, 32);

        assert_eq!(chunk.tile_count(), 1024);
        assert_eq!(chunk.dirty_indices(), Vec::<u16>::new());

        chunk
            .set_tile_record(
                0,
                TileRecord {
                    base: TileBase::Water,
                    ..TileRecord::default()
                },
            )
            .expect("index 0 exists");
        chunk
            .set_tile_record(
                17,
                TileRecord {
                    surface: TileSurface::Street,
                    road_mask: Some(5),
                    ..TileRecord::default()
                },
            )
            .expect("index 17 exists");

        assert_eq!(chunk.tile_at(0).unwrap().base, TileBase::Water);
        assert_eq!(chunk.tile_at(17).unwrap().surface, TileSurface::Street);
        assert_eq!(chunk.version(), 2);
        assert_eq!(chunk.dirty_indices(), vec![0, 17]);
    }

    #[test]
    fn chunk_from_snapshot_round_trips_full_state() {
        use crate::persistence::build_chunk_snapshot;
        use crate::scheduler::ChunkActivity;

        let mut original = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        original
            .set_tile_record(
                0,
                TileRecord {
                    surface: TileSurface::Street,
                    road_mask: Some(5),
                    ..TileRecord::default()
                },
            )
            .unwrap();
        original
            .set_tile_record(
                17,
                TileRecord {
                    base: TileBase::Water,
                    ..TileRecord::default()
                },
            )
            .unwrap();
        original
            .set_tile_record(
                42,
                TileRecord {
                    base: TileBase::Park,
                    ..TileRecord::default()
                },
            )
            .unwrap();

        let snapshot = build_chunk_snapshot("abutown-main", &original, ChunkActivity::Active);
        let restored = Chunk::from_snapshot(&snapshot).unwrap();

        assert_eq!(restored.coord(), original.coord());
        assert_eq!(restored.chunk_size(), original.chunk_size());
        assert_eq!(restored.version(), original.version());
        assert_eq!(restored.tile_at(0).unwrap().surface, TileSurface::Street);
        assert_eq!(restored.tile_at(0).unwrap().version, 1);
        assert_eq!(restored.tile_at(17).unwrap().base, TileBase::Water);
        assert_eq!(restored.tile_at(17).unwrap().version, 2);
        assert_eq!(restored.tile_at(42).unwrap().base, TileBase::Park);
        assert_eq!(restored.tile_at(42).unwrap().version, 3);
        assert_eq!(restored.tile_at(1), Some(TileRecord::default()));
        assert_eq!(restored.dirty_indices(), Vec::<u16>::new());
    }

    #[test]
    fn chunk_from_snapshot_rejects_oversized_local_index() {
        use crate::scheduler::ChunkActivity;
        use abutown_protocol::{
            ChunkCoordDto, ChunkSnapshotDto, LayeredTileDto, PROTOCOL_VERSION, TileBaseDto,
            TileCoverDto, TileSurfaceDto, WorldId,
        };

        let snapshot = ChunkSnapshotDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            coord: ChunkCoordDto { x: 4, y: 4 },
            chunk_state: ChunkActivity::Active.into(),
            chunk_version: 1,
            tile_count: 1024,
            tiles: vec![LayeredTileDto {
                local_index: 9999,
                base: TileBaseDto::Grass,
                surface: TileSurfaceDto::Street,
                cover: TileCoverDto::None,
                display: None,
                zone_id: None,
                road_mask: Some(5),
                rail_mask: None,
                version: 1,
            }],
        };

        let err = Chunk::from_snapshot(&snapshot).unwrap_err();
        assert!(matches!(
            err,
            SnapshotDecodeError::IndexOutOfBounds { index: 9999, .. }
        ));
    }

    #[test]
    fn chunk_from_snapshot_rejects_non_square_tile_count() {
        use crate::scheduler::ChunkActivity;
        use abutown_protocol::{ChunkCoordDto, ChunkSnapshotDto, PROTOCOL_VERSION, WorldId};

        let snapshot = ChunkSnapshotDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            coord: ChunkCoordDto { x: 0, y: 0 },
            chunk_state: ChunkActivity::Active.into(),
            chunk_version: 0,
            tile_count: 1000,
            tiles: vec![],
        };

        let err = Chunk::from_snapshot(&snapshot).unwrap_err();
        assert!(matches!(
            err,
            SnapshotDecodeError::NonSquareTileCount { tile_count: 1000 }
        ));
    }

    #[test]
    fn oversized_chunk_size_is_rejected() {
        let result = Chunk::try_new(ChunkCoord { x: 0, y: 0 }, 256);

        assert!(matches!(
            result,
            Err(ChunkError::InvalidChunkSize {
                chunk_size: 256,
                tile_count: 65_536,
                max_tile_count: 65_535,
            })
        ));
    }
}
