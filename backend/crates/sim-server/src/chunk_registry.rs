use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, WorldId};
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    persistence::{InMemoryChunkSnapshotStore, build_chunk_snapshot},
    scheduler::ChunkActivity,
};

#[derive(Debug)]
struct LoadedChunk {
    chunk: Chunk,
    activity: ChunkActivity,
}

#[derive(Debug)]
pub(crate) struct ChunkRegistry {
    chunk_size: u16,
    chunks: HashMap<ChunkCoord, LoadedChunk>,
}

impl ChunkRegistry {
    pub(crate) fn new(chunk_size: u16) -> Self {
        Self {
            chunk_size,
            chunks: HashMap::new(),
        }
    }

    pub(crate) fn chunk_size(&self) -> u16 {
        self.chunk_size
    }

    pub(crate) fn insert_chunk(&mut self, chunk: Chunk, activity: ChunkActivity) {
        assert_eq!(
            chunk.chunk_size(),
            self.chunk_size,
            "loaded chunk size must match registry chunk size"
        );
        self.chunks
            .insert(chunk.coord(), LoadedChunk { chunk, activity });
    }

    pub(crate) fn loaded_coords(&self) -> Vec<ChunkCoord> {
        let mut coords: Vec<ChunkCoord> = self.chunks.keys().copied().collect();
        coords.sort_by_key(|coord| (coord.y, coord.x));
        coords
    }

    pub(crate) fn chunk_snapshot(
        &self,
        world_id: &WorldId,
        coord: ChunkCoord,
    ) -> Option<ChunkSnapshotDto> {
        let loaded = self.chunks.get(&coord)?;
        Some(build_chunk_snapshot(
            &world_id.0,
            &loaded.chunk,
            loaded.activity,
        ))
    }

    pub(crate) fn tile_count(&self, coord: ChunkCoord) -> Option<u16> {
        self.chunks
            .get(&coord)
            .map(|loaded| loaded.chunk.tile_count())
    }

    pub(crate) fn write_snapshots(
        &mut self,
        world_id: &WorldId,
        store: &mut InMemoryChunkSnapshotStore,
    ) -> usize {
        let coords = self.loaded_coords();
        let mut written = 0;

        for coord in coords {
            let Some(loaded) = self.chunks.get_mut(&coord) else {
                continue;
            };

            let snapshot = build_chunk_snapshot(&world_id.0, &loaded.chunk, loaded.activity);
            store.write_snapshot(snapshot);
            loaded.chunk.clear_dirty();
            written += 1;
        }

        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::tile::TileKind;

    fn chunk_with_seed(coord: ChunkCoord, local_index: u16, kind: TileKind) -> Chunk {
        let mut chunk = Chunk::new(coord, 32);
        chunk
            .set_tile_kind(local_index, kind)
            .expect("seed index exists");
        chunk
    }

    #[test]
    fn registry_lists_loaded_chunks_in_deterministic_order() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 5, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Warm,
        );
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Water),
            ChunkActivity::Active,
        );

        assert_eq!(
            registry.loaded_coords(),
            vec![ChunkCoord { x: 4, y: 4 }, ChunkCoord { x: 5, y: 4 }]
        );
    }

    #[test]
    fn registry_builds_snapshots_only_for_loaded_chunks() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 17, TileKind::Road),
            ChunkActivity::Active,
        );

        let world_id = WorldId("abutown-main".to_string());
        let snapshot = registry
            .chunk_snapshot(&world_id, ChunkCoord { x: 4, y: 4 })
            .expect("loaded chunk snapshot exists");

        assert_eq!(snapshot.coord.x, 4);
        assert_eq!(snapshot.coord.y, 4);
        assert_eq!(
            snapshot.chunk_state,
            abutown_protocol::ChunkStateDto::Active
        );
        assert_eq!(snapshot.tile_count, 1024);
        assert_eq!(snapshot.dirty_tiles.len(), 1);
        assert_eq!(snapshot.dirty_tiles[0].local_index, 17);
        assert_eq!(
            snapshot.dirty_tiles[0].kind,
            abutown_protocol::TileKindDto::Road
        );
        assert!(
            registry
                .chunk_snapshot(&world_id, ChunkCoord { x: 0, y: 0 })
                .is_none()
        );
    }

    #[test]
    fn registry_reports_loaded_tile_counts() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 5 }, 0, TileKind::BuildingFootprint),
            ChunkActivity::Warm,
        );

        assert_eq!(registry.tile_count(ChunkCoord { x: 4, y: 5 }), Some(1024));
        assert_eq!(registry.tile_count(ChunkCoord { x: 9, y: 9 }), None);
    }

    #[test]
    fn registry_writes_snapshots_and_clears_dirty_tiles() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 5, y: 4 }, 7, TileKind::Water),
            ChunkActivity::Warm,
        );
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 3, TileKind::Road),
            ChunkActivity::Active,
        );

        let world_id = WorldId("abutown-main".to_string());
        let mut store = InMemoryChunkSnapshotStore::default();

        assert_eq!(registry.write_snapshots(&world_id, &mut store), 2);
        assert_eq!(store.snapshot_count(), 2);
        assert_eq!(
            store.snapshot_coords(),
            vec![ChunkCoord { x: 4, y: 4 }, ChunkCoord { x: 5, y: 4 }]
        );
        assert_eq!(
            store
                .read_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("visible snapshot exists")
                .dirty_tiles
                .len(),
            1
        );

        assert_eq!(registry.write_snapshots(&world_id, &mut store), 2);
        assert!(
            store
                .read_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("visible snapshot still exists")
                .dirty_tiles
                .is_empty()
        );
    }
}
