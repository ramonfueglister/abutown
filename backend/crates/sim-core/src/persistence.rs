use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, PROTOCOL_VERSION, TileMutationDto, WorldId};

use crate::chunk::Chunk;
use crate::ids::ChunkCoord;
use crate::scheduler::ChunkActivity;

pub fn build_chunk_snapshot(
    world_id: impl Into<String>,
    chunk: &Chunk,
    activity: ChunkActivity,
) -> ChunkSnapshotDto {
    let dirty_tiles = chunk
        .dirty_indices()
        .into_iter()
        .filter_map(|index| {
            chunk.tile_at(index).map(|tile| TileMutationDto {
                local_index: index,
                kind: tile.kind.into(),
                version: tile.version,
            })
        })
        .collect();

    ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId(world_id.into()),
        coord: chunk.coord().into(),
        chunk_state: activity.into(),
        chunk_version: chunk.version(),
        tile_count: chunk.tile_count(),
        dirty_tiles,
    }
}

#[derive(Default)]
pub struct InMemoryChunkSnapshotStore {
    snapshots: HashMap<ChunkCoord, ChunkSnapshotDto>,
}

impl InMemoryChunkSnapshotStore {
    pub fn write_snapshot(&mut self, snapshot: ChunkSnapshotDto) {
        self.snapshots.insert(
            ChunkCoord {
                x: snapshot.coord.x,
                y: snapshot.coord.y,
            },
            snapshot,
        );
    }

    pub fn read_snapshot(&self, coord: ChunkCoord) -> Option<&ChunkSnapshotDto> {
        self.snapshots.get(&coord)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use crate::ids::ChunkCoord;
    use crate::scheduler::ChunkActivity;
    use crate::tile::TileKind;

    #[test]
    fn snapshot_contains_only_dirty_tiles_then_clears_dirty_state() {
        let mut chunk = Chunk::new(ChunkCoord { x: 1, y: 2 }, 32);
        chunk
            .set_tile_kind(3, TileKind::Water)
            .expect("tile exists");
        chunk.set_tile_kind(9, TileKind::Road).expect("tile exists");

        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        assert_eq!(snapshot.dirty_tiles.len(), 2);
        assert_eq!(snapshot.dirty_tiles[0].local_index, 3);
        assert_eq!(snapshot.dirty_tiles[1].local_index, 9);

        chunk.clear_dirty();
        assert!(chunk.dirty_indices().is_empty());
    }
}
