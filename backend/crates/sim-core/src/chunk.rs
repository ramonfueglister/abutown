use std::collections::BTreeSet;

use abutown_protocol::ChunkSnapshotDto;
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotDecodeError {
    #[error("snapshot tile_count {tile_count} is not a square number")]
    NonSquareTileCount { tile_count: u16 },
    #[error("tile index {index} is outside snapshot tile_count {tile_count}")]
    IndexOutOfBounds { index: u16, tile_count: u16 },
    #[error("chunk construction failed: {0}")]
    Chunk(ChunkError),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventApplyError {
    #[error("event coord ({event_x},{event_y}) does not match chunk coord ({chunk_x},{chunk_y})")]
    WrongChunkCoord {
        event_x: i32,
        event_y: i32,
        chunk_x: i32,
        chunk_y: i32,
    },
    #[error(
        "event chunk_version {event_version} is older than current chunk version {chunk_version}"
    )]
    StaleEvent {
        event_version: u64,
        chunk_version: u64,
    },
    #[error("event chunk_version {event_version} skips past current chunk version {chunk_version}")]
    GapEvent {
        event_version: u64,
        chunk_version: u64,
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

    pub fn from_records(
        coord: ChunkCoord,
        chunk_size: u16,
        tiles: Vec<TileRecord>,
        version: u64,
    ) -> Result<Self, ChunkError> {
        let expected_tile_count = usize::from(chunk_size) * usize::from(chunk_size);
        if expected_tile_count > usize::from(u16::MAX) {
            return Err(ChunkError::InvalidChunkSize {
                chunk_size,
                tile_count: expected_tile_count,
                max_tile_count: usize::from(u16::MAX),
            });
        }
        if tiles.len() != expected_tile_count {
            return Err(ChunkError::InvalidChunkSize {
                chunk_size,
                tile_count: tiles.len(),
                max_tile_count: expected_tile_count,
            });
        }

        Ok(Self {
            coord,
            chunk_size,
            version,
            tiles,
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

        for mutation in &snapshot.tiles {
            let slot = chunk
                .tiles
                .get_mut(usize::from(mutation.local_index))
                .ok_or(SnapshotDecodeError::IndexOutOfBounds {
                    index: mutation.local_index,
                    tile_count,
                })?;
            slot.kind = mutation.kind.into();
            slot.version = mutation.version;
            // Mark restored non-default tiles as modified so subsequent snapshots include them.
            slot.flags.modified = true;
        }
        chunk.version = snapshot.chunk_version;
        Ok(chunk)
    }

    pub fn apply_event(
        &mut self,
        event: &abutown_protocol::WorldEventDto,
        event_chunk_version: u64,
    ) -> Result<(), EventApplyError> {
        use abutown_protocol::WorldEventDto;

        // Coord check first — applies to all variants and runs even on idempotent re-delivery.
        match event {
            WorldEventDto::TileKindSet(payload) => {
                if payload.coord.x != self.coord.x || payload.coord.y != self.coord.y {
                    return Err(EventApplyError::WrongChunkCoord {
                        event_x: payload.coord.x,
                        event_y: payload.coord.y,
                        chunk_x: self.coord.x,
                        chunk_y: self.coord.y,
                    });
                }
            }
        }

        // Version checks.
        if event_chunk_version == self.version {
            return Ok(());
        }
        if event_chunk_version < self.version {
            return Err(EventApplyError::StaleEvent {
                event_version: event_chunk_version,
                chunk_version: self.version,
            });
        }
        if event_chunk_version != self.version + 1 {
            return Err(EventApplyError::GapEvent {
                event_version: event_chunk_version,
                chunk_version: self.version,
            });
        }

        // Apply mutation.
        match event {
            WorldEventDto::TileKindSet(payload) => {
                let tile_count = self.tile_count();
                let slot = self.tiles.get_mut(usize::from(payload.local_index)).ok_or(
                    EventApplyError::IndexOutOfBounds {
                        index: payload.local_index,
                        tile_count,
                    },
                )?;
                self.version = event_chunk_version;
                slot.kind = payload.kind.into();
                slot.version = self.version;
                slot.flags.modified = true;
                self.dirty.insert(payload.local_index);
            }
        }

        Ok(())
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
    fn chunk_from_snapshot_round_trips_full_state() {
        use crate::persistence::build_chunk_snapshot;
        use crate::scheduler::ChunkActivity;

        let mut original = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        original.set_tile_kind(0, TileKind::Road).unwrap();
        original.set_tile_kind(17, TileKind::Water).unwrap();
        original
            .set_tile_kind(42, TileKind::BuildingFootprint)
            .unwrap();

        let snapshot = build_chunk_snapshot("abutopia", &original, ChunkActivity::Active);
        let restored = Chunk::from_snapshot(&snapshot).unwrap();

        assert_eq!(restored.coord(), original.coord());
        assert_eq!(restored.chunk_size(), original.chunk_size());
        assert_eq!(restored.version(), original.version());
        assert_eq!(restored.kind_at(0), Some(TileKind::Road));
        assert_eq!(restored.kind_at(17), Some(TileKind::Water));
        assert_eq!(restored.kind_at(42), Some(TileKind::BuildingFootprint));
        assert_eq!(restored.kind_at(1), Some(TileKind::default()));
        assert_eq!(restored.dirty_indices(), Vec::<u16>::new());
    }

    #[test]
    fn chunk_from_snapshot_rejects_oversized_local_index() {
        use crate::scheduler::ChunkActivity;
        use abutown_protocol::{
            ChunkCoordDto, ChunkSnapshotDto, PROTOCOL_VERSION, TileKindDto, TileMutationDto,
            WorldId,
        };

        let snapshot = ChunkSnapshotDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutopia".to_string()),
            coord: ChunkCoordDto { x: 4, y: 4 },
            chunk_state: ChunkActivity::Active.into(),
            chunk_version: 1,
            tile_count: 1024,
            tiles: vec![TileMutationDto {
                local_index: 9999,
                kind: TileKindDto::Road,
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
            world_id: WorldId("abutopia".to_string()),
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
    fn chunk_apply_event_advances_version_and_mutates_tile() {
        use abutown_protocol::{
            ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto,
            WorldId,
        };

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:1".to_string(),
            command_id: "command:1".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 1,
            version: 1,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 7,
            kind: TileKindDto::Road,
        });

        chunk.apply_event(&event, 1).unwrap();

        assert_eq!(chunk.version(), 1);
        assert_eq!(chunk.kind_at(7), Some(TileKind::Road));
    }

    #[test]
    fn chunk_apply_event_rejects_event_for_wrong_coord() {
        use abutown_protocol::{
            ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto,
            WorldId,
        };

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:1".to_string(),
            command_id: "command:1".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 1,
            version: 1,
            coord: ChunkCoordDto { x: 9, y: 9 },
            local_index: 7,
            kind: TileKindDto::Road,
        });

        let err = chunk.apply_event(&event, 1).unwrap_err();
        assert!(matches!(err, EventApplyError::WrongChunkCoord { .. }));
    }

    #[test]
    fn chunk_apply_event_idempotent_for_same_chunk_version() {
        use abutown_protocol::{
            ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto,
            WorldId,
        };

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:1".to_string(),
            command_id: "command:1".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 1,
            version: 1,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 7,
            kind: TileKindDto::Road,
        });

        chunk.apply_event(&event, 1).unwrap();
        chunk.apply_event(&event, 1).unwrap();

        assert_eq!(
            chunk.version(),
            1,
            "re-applying the same chunk_version must not bump version"
        );
        assert_eq!(chunk.kind_at(7), Some(TileKind::Road));
    }

    #[test]
    fn chunk_apply_event_rejects_stale_chunk_version() {
        use abutown_protocol::{
            ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto,
            WorldId,
        };

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        let mk_event = |chunk_version: u64| {
            WorldEventDto::TileKindSet(TileKindSetEventDto {
                protocol_version: PROTOCOL_VERSION,
                event_id: format!("event:{chunk_version}"),
                command_id: format!("command:{chunk_version}"),
                world_id: WorldId("abutopia".to_string()),
                tick: chunk_version,
                version: chunk_version,
                coord: ChunkCoordDto { x: 4, y: 4 },
                local_index: 0,
                kind: TileKindDto::Road,
            })
        };

        chunk.apply_event(&mk_event(1), 1).unwrap();
        chunk.apply_event(&mk_event(2), 2).unwrap();

        let err = chunk.apply_event(&mk_event(1), 1).unwrap_err();
        // chunk is now at version 2; replaying at version 1 is stale.
        assert!(
            matches!(
                err,
                EventApplyError::StaleEvent {
                    event_version: 1,
                    chunk_version: 2
                }
            ),
            "expected StaleEvent {{ event: 1, chunk: 2 }}, got {err:?}"
        );
    }

    #[test]
    fn chunk_apply_event_rejects_gap_in_chunk_version() {
        use abutown_protocol::{
            ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto,
            WorldId,
        };

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:5".to_string(),
            command_id: "command:5".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 5,
            version: 5,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 3,
            kind: TileKindDto::Water,
        });

        let err = chunk.apply_event(&event, 5).unwrap_err();
        assert!(
            matches!(
                err,
                EventApplyError::GapEvent {
                    event_version: 5,
                    chunk_version: 0
                }
            ),
            "expected GapEvent {{ event: 5, chunk: 0 }}, got {err:?}"
        );
    }

    #[test]
    fn chunk_apply_event_rejects_index_out_of_bounds() {
        use abutown_protocol::{
            ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto,
            WorldId,
        };

        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:oob".to_string(),
            command_id: "command:oob".to_string(),
            world_id: WorldId("abutopia".to_string()),
            tick: 1,
            version: 1,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 9999,
            kind: TileKindDto::Road,
        });

        let err = chunk.apply_event(&event, 1).unwrap_err();
        assert!(
            matches!(
                err,
                EventApplyError::IndexOutOfBounds {
                    index: 9999,
                    tile_count: 1024
                }
            ),
            "expected IndexOutOfBounds {{ index: 9999, tile_count: 1024 }}, got {err:?}"
        );
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
