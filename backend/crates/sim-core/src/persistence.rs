use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, PROTOCOL_VERSION, TileMutationDto, WorldId};
use async_trait::async_trait;

use crate::chunk::Chunk;
use crate::ids::ChunkCoord;
use crate::mobility::MobilityWorld;
use crate::scheduler::ChunkActivity;
use crate::tile::TileKind;

pub fn build_chunk_snapshot(
    world_id: impl Into<String>,
    chunk: &Chunk,
    activity: ChunkActivity,
) -> ChunkSnapshotDto {
    let mut tiles: Vec<TileMutationDto> = Vec::new();
    for index in 0..chunk.tile_count() {
        let tile = chunk.tile_at(index).expect("index within tile_count");
        if tile.kind != TileKind::default() {
            tiles.push(TileMutationDto {
                local_index: index,
                kind: tile.kind.into(),
                version: tile.version,
            });
        }
    }

    ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId(world_id.into()),
        coord: chunk.coord().into(),
        chunk_state: activity.into(),
        chunk_version: chunk.version(),
        tile_count: chunk.tile_count(),
        tiles,
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ChunkSnapshotStoreError {
    message: String,
}

impl ChunkSnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait ChunkSnapshotStore: std::fmt::Debug + Send {
    async fn write_snapshot(
        &mut self,
        snapshot: ChunkSnapshotDto,
    ) -> Result<(), ChunkSnapshotStoreError>;

    async fn read_snapshot(
        &self,
        coord: ChunkCoord,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryChunkSnapshotStore {
    snapshots: HashMap<ChunkCoord, ChunkSnapshotDto>,
}

#[async_trait]
impl ChunkSnapshotStore for InMemoryChunkSnapshotStore {
    async fn write_snapshot(
        &mut self,
        snapshot: ChunkSnapshotDto,
    ) -> Result<(), ChunkSnapshotStoreError> {
        InMemoryChunkSnapshotStore::write_snapshot(self, snapshot);
        Ok(())
    }

    async fn read_snapshot(
        &self,
        coord: ChunkCoord,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        Ok(InMemoryChunkSnapshotStore::read_snapshot(self, coord).cloned())
    }
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

    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    pub fn snapshot_coords(&self) -> Vec<ChunkCoord> {
        let mut coords: Vec<ChunkCoord> = self.snapshots.keys().copied().collect();
        coords.sort_by_key(|coord| (coord.y, coord.x));
        coords
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct MobilitySnapshotStoreError {
    message: String,
}

impl MobilitySnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait MobilitySnapshotStore: std::fmt::Debug + Send {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityWorld,
    ) -> Result<(), MobilitySnapshotStoreError>;

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, MobilityWorld)>, MobilitySnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryMobilitySnapshotStore {
    snapshots: HashMap<String, (u64, MobilityWorld)>,
}

#[async_trait]
impl MobilitySnapshotStore for InMemoryMobilitySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityWorld,
    ) -> Result<(), MobilitySnapshotStoreError> {
        self.snapshots
            .insert(world_id.to_string(), (tick, snapshot.clone()));
        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
    ) -> Result<Option<(u64, MobilityWorld)>, MobilitySnapshotStoreError> {
        Ok(self.snapshots.get(world_id).cloned())
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
    fn snapshot_contains_initial_tiles_then_clears_dirty_state() {
        let mut chunk = Chunk::new(ChunkCoord { x: 1, y: 2 }, 32);
        chunk
            .set_tile_kind(3, TileKind::Water)
            .expect("tile exists");
        chunk.set_tile_kind(9, TileKind::Road).expect("tile exists");

        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        assert_eq!(snapshot.tiles.len(), 2);
        assert_eq!(snapshot.tiles[0].local_index, 3);
        assert_eq!(snapshot.tiles[1].local_index, 9);

        chunk.clear_dirty();
        assert!(chunk.dirty_indices().is_empty());
    }

    #[test]
    fn build_chunk_snapshot_emits_all_non_default_tiles_after_clear_dirty() {
        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        chunk.set_tile_kind(0, TileKind::Road).unwrap();
        chunk.set_tile_kind(17, TileKind::Water).unwrap();
        chunk.clear_dirty();
        chunk
            .set_tile_kind(42, TileKind::BuildingFootprint)
            .unwrap();

        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        let indices: Vec<u16> = snapshot.tiles.iter().map(|t| t.local_index).collect();
        assert_eq!(
            indices,
            vec![0, 17, 42],
            "snapshot must include all non-default tiles, not only currently-dirty ones"
        );
        assert_eq!(snapshot.chunk_version, 3);
    }

    #[test]
    fn snapshot_store_reports_count_and_sorted_coords() {
        let mut store = InMemoryChunkSnapshotStore::default();

        let mut east = Chunk::new(ChunkCoord { x: 5, y: 4 }, 32);
        east.set_tile_kind(0, TileKind::Water).expect("tile exists");
        let mut visible = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        visible
            .set_tile_kind(0, TileKind::Road)
            .expect("tile exists");

        store.write_snapshot(build_chunk_snapshot(
            "abutown-main",
            &east,
            ChunkActivity::Warm,
        ));
        store.write_snapshot(build_chunk_snapshot(
            "abutown-main",
            &visible,
            ChunkActivity::Active,
        ));

        assert_eq!(store.snapshot_count(), 2);
        assert_eq!(
            store.snapshot_coords(),
            vec![ChunkCoord { x: 4, y: 4 }, ChunkCoord { x: 5, y: 4 }]
        );
    }

    #[tokio::test]
    async fn chunk_snapshot_store_writes_and_reads_snapshot() {
        let mut store = InMemoryChunkSnapshotStore::default();
        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        chunk.set_tile_kind(0, TileKind::Road).expect("tile exists");
        let snapshot = build_chunk_snapshot("abutown-main", &chunk, ChunkActivity::Active);

        ChunkSnapshotStore::write_snapshot(&mut store, snapshot.clone())
            .await
            .unwrap();

        let stored = ChunkSnapshotStore::read_snapshot(&store, ChunkCoord { x: 4, y: 4 })
            .await
            .unwrap()
            .expect("snapshot exists");
        assert_eq!(stored, snapshot);
    }

    #[tokio::test]
    async fn mobility_snapshot_store_writes_and_reads() {
        use crate::mobility::seed;

        let mut store = InMemoryMobilitySnapshotStore::default();
        let world = seed::initial_world();

        MobilitySnapshotStore::write(&mut store, "abutown-main", 42, &world)
            .await
            .unwrap();

        let (tick, restored) = MobilitySnapshotStore::read(&store, "abutown-main")
            .await
            .unwrap()
            .expect("snapshot exists");

        assert_eq!(tick, 42);
        assert_eq!(restored, world);
    }

    #[tokio::test]
    async fn mobility_snapshot_store_read_returns_none_for_unknown_world() {
        let store = InMemoryMobilitySnapshotStore::default();
        let result = MobilitySnapshotStore::read(&store, "missing-world")
            .await
            .unwrap();
        assert!(result.is_none());
    }

}
