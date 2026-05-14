use std::collections::BTreeSet;

use thiserror::Error;

use crate::ids::ChunkCoord;
use crate::tile::{TileKind, TileRecord};

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

    pub fn kind_at(&self, index: u16) -> Option<TileKind> {
        self.tiles.get(usize::from(index)).map(|tile| tile.kind)
    }

    pub fn tile_at(&self, index: u16) -> Option<TileRecord> {
        self.tiles.get(usize::from(index)).copied()
    }

    pub fn dirty_indices(&self) -> Vec<u16> {
        self.dirty.iter().copied().collect()
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    pub fn set_tile_kind(&mut self, index: u16, kind: TileKind) -> Result<(), ChunkError> {
        let tile_count = self.tile_count();
        let tile = self
            .tiles
            .get_mut(usize::from(index))
            .ok_or(ChunkError::IndexOutOfBounds { index, tile_count })?;

        if tile.kind != kind {
            self.version += 1;
            tile.kind = kind;
            tile.version = self.version;
            tile.flags.modified = true;
            self.dirty.insert(index);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::TileKind;

    #[test]
    fn chunk_uses_dense_tiles_and_tracks_dirty_indices() {
        let mut chunk = Chunk::new(ChunkCoord { x: 2, y: -1 }, 32);

        assert_eq!(chunk.tile_count(), 1024);
        assert_eq!(chunk.dirty_indices(), Vec::<u16>::new());

        chunk
            .set_tile_kind(0, TileKind::Water)
            .expect("index 0 exists");
        chunk
            .set_tile_kind(17, TileKind::Road)
            .expect("index 17 exists");

        assert_eq!(chunk.kind_at(0), Some(TileKind::Water));
        assert_eq!(chunk.kind_at(17), Some(TileKind::Road));
        assert_eq!(chunk.version(), 2);
        assert_eq!(chunk.dirty_indices(), vec![0, 17]);
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
