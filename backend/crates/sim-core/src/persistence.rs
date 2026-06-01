use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, PROTOCOL_VERSION, TileMutationDto, WorldId};
use async_trait::async_trait;

use crate::chunk::Chunk;
use crate::economy::EconomyPersistSnapshot;
use crate::ids::ChunkCoord;
use crate::mobility::MobilityPersistSnapshot;
use crate::scheduler::ChunkActivity;
use crate::tile::TileKind;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotCompatibility {
    pub base_world_id: String,
    pub base_world_schema_version: u32,
}

impl SnapshotCompatibility {
    pub fn new(base_world_id: impl Into<String>, base_world_schema_version: u32) -> Self {
        Self {
            base_world_id: base_world_id.into(),
            base_world_schema_version,
        }
    }
}

/// Build a `ChunkSnapshotDto` from a `Chunk` value. Delegates to
/// `build_chunk_snapshot_from_parts` after extracting the dense tile vector
/// from the chunk — the canonical implementation lives there.
pub fn build_chunk_snapshot(
    world_id: impl Into<String>,
    chunk: &Chunk,
    activity: ChunkActivity,
) -> ChunkSnapshotDto {
    let tiles: Vec<crate::tile::TileRecord> = (0..chunk.tile_count())
        .filter_map(|i| chunk.tile_at(i))
        .collect();
    build_chunk_snapshot_from_parts(world_id, chunk.coord(), &tiles, chunk.version(), activity)
}

/// Build a `ChunkSnapshotDto` from raw ECS data (tiles, version, coord,
/// activity). Canonical builder — both the `Chunk`-based path
/// (`build_chunk_snapshot`) and the ECS-world path
/// (`ChunkSnapshotProvider::collect`) funnel through here, so the
/// serialized JSONB payload is byte-identical regardless of source.
pub fn build_chunk_snapshot_from_parts(
    world_id: impl Into<String>,
    coord: ChunkCoord,
    tiles: &[crate::tile::TileRecord],
    chunk_version: u64,
    activity: ChunkActivity,
) -> ChunkSnapshotDto {
    let tile_count = u16::try_from(tiles.len())
        .expect("chunk tile count exceeds u16; chunk_size must be <= 255 (see Chunk::try_new)");
    let mut emitted: Vec<TileMutationDto> = Vec::new();
    for (index, tile) in tiles.iter().enumerate() {
        if tile.kind != TileKind::default() {
            emitted.push(TileMutationDto {
                local_index: u16::try_from(index).expect(
                    "tile index exceeds u16; chunk_size must be <= 255 (see Chunk::try_new)",
                ),
                kind: tile.kind.into(),
                version: tile.version,
            });
        }
    }
    ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId(world_id.into()),
        coord: coord.into(),
        chunk_state: activity.into(),
        chunk_version,
        tile_count,
        tiles: emitted,
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
pub trait ChunkSnapshotStore: std::fmt::Debug + Send + Sync {
    async fn write_snapshot(
        &mut self,
        snapshot: ChunkSnapshotDto,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), ChunkSnapshotStoreError>;

    async fn read_snapshot(
        &self,
        coord: ChunkCoord,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryChunkSnapshotStore {
    snapshots: HashMap<(ChunkCoord, SnapshotCompatibility), ChunkSnapshotDto>,
}

#[async_trait]
impl ChunkSnapshotStore for InMemoryChunkSnapshotStore {
    async fn write_snapshot(
        &mut self,
        snapshot: ChunkSnapshotDto,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), ChunkSnapshotStoreError> {
        InMemoryChunkSnapshotStore::write_snapshot(self, snapshot, compatibility);
        Ok(())
    }

    async fn read_snapshot(
        &self,
        coord: ChunkCoord,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<ChunkSnapshotDto>, ChunkSnapshotStoreError> {
        Ok(InMemoryChunkSnapshotStore::read_snapshot(self, coord, compatibility).cloned())
    }
}

impl InMemoryChunkSnapshotStore {
    pub fn write_snapshot(
        &mut self,
        snapshot: ChunkSnapshotDto,
        compatibility: &SnapshotCompatibility,
    ) {
        self.snapshots.insert(
            (
                ChunkCoord {
                    x: snapshot.coord.x,
                    y: snapshot.coord.y,
                },
                compatibility.clone(),
            ),
            snapshot,
        );
    }

    pub fn read_snapshot(
        &self,
        coord: ChunkCoord,
        compatibility: &SnapshotCompatibility,
    ) -> Option<&ChunkSnapshotDto> {
        self.snapshots.get(&(coord, compatibility.clone()))
    }

    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    pub fn snapshot_coords(&self) -> Vec<ChunkCoord> {
        let mut coords: Vec<ChunkCoord> = self
            .snapshots
            .keys()
            .map(|(coord, _compatibility)| *coord)
            .collect();
        coords.sort_unstable();
        coords.dedup();
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
pub trait MobilitySnapshotStore: std::fmt::Debug + Send + Sync {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), MobilitySnapshotStoreError>;

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, MobilityPersistSnapshot)>, MobilitySnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryMobilitySnapshotStore {
    snapshots: HashMap<(String, SnapshotCompatibility), (u64, MobilityPersistSnapshot)>,
}

#[async_trait]
impl MobilitySnapshotStore for InMemoryMobilitySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &MobilityPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), MobilitySnapshotStoreError> {
        self.snapshots.insert(
            (world_id.to_string(), compatibility.clone()),
            (tick, snapshot.clone()),
        );
        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, MobilityPersistSnapshot)>, MobilitySnapshotStoreError> {
        Ok(self
            .snapshots
            .get(&(world_id.to_string(), compatibility.clone()))
            .cloned())
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct EconomySnapshotStoreError {
    message: String,
}

impl EconomySnapshotStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait EconomySnapshotStore: std::fmt::Debug + Send + Sync {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &EconomyPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), EconomySnapshotStoreError>;

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, EconomyPersistSnapshot)>, EconomySnapshotStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryEconomySnapshotStore {
    snapshots: HashMap<(String, SnapshotCompatibility), (u64, EconomyPersistSnapshot)>,
}

#[async_trait]
impl EconomySnapshotStore for InMemoryEconomySnapshotStore {
    async fn write(
        &mut self,
        world_id: &str,
        tick: u64,
        snapshot: &EconomyPersistSnapshot,
        compatibility: &SnapshotCompatibility,
    ) -> Result<(), EconomySnapshotStoreError> {
        self.snapshots.insert(
            (world_id.to_string(), compatibility.clone()),
            (tick, snapshot.clone()),
        );
        Ok(())
    }

    async fn read(
        &self,
        world_id: &str,
        compatibility: &SnapshotCompatibility,
    ) -> Result<Option<(u64, EconomyPersistSnapshot)>, EconomySnapshotStoreError> {
        Ok(self
            .snapshots
            .get(&(world_id.to_string(), compatibility.clone()))
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use crate::ids::ChunkCoord;
    use crate::scheduler::ChunkActivity;
    use crate::tile::{TileKind, TileRecord};

    #[test]
    fn build_chunk_snapshot_accepts_u16_max_tiles() {
        // 65535 tiles fit in u16: must not panic, tile_count round-trips.
        let tiles = vec![TileRecord::default(); usize::from(u16::MAX)];
        let dto = build_chunk_snapshot_from_parts(
            "abutopia",
            ChunkCoord { x: 0, y: 0 },
            &tiles,
            1,
            ChunkActivity::Active,
        );
        assert_eq!(dto.tile_count, u16::MAX);
    }

    #[test]
    #[should_panic(expected = "chunk tile count exceeds u16")]
    fn build_chunk_snapshot_panics_when_tile_count_exceeds_u16() {
        // 65536 tiles overflow u16: the writer must fail loudly, never silently
        // truncate the snapshot. (chunk_size is capped at 255 by Chunk::try_new,
        // so this is an unreachable internal invariant — but the public builder
        // must not corrupt data if that contract is ever broken.)
        let tiles = vec![TileRecord::default(); usize::from(u16::MAX) + 1];
        let _ = build_chunk_snapshot_from_parts(
            "abutopia",
            ChunkCoord { x: 0, y: 0 },
            &tiles,
            1,
            ChunkActivity::Active,
        );
    }

    #[test]
    fn snapshot_contains_initial_tiles_then_clears_dirty_state() {
        let mut chunk = Chunk::new(ChunkCoord { x: 1, y: 2 }, 32);
        chunk
            .set_tile_kind(3, TileKind::Water)
            .expect("tile exists");
        chunk.set_tile_kind(9, TileKind::Road).expect("tile exists");

        let snapshot = build_chunk_snapshot("abutopia", &chunk, ChunkActivity::Active);

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

        let snapshot = build_chunk_snapshot("abutopia", &chunk, ChunkActivity::Active);

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
        let compatibility = SnapshotCompatibility::new("abutopia", 1);

        let mut east = Chunk::new(ChunkCoord { x: 5, y: 4 }, 32);
        east.set_tile_kind(0, TileKind::Water).expect("tile exists");
        let mut visible = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        visible
            .set_tile_kind(0, TileKind::Road)
            .expect("tile exists");

        store.write_snapshot(
            build_chunk_snapshot("abutopia", &east, ChunkActivity::Warm),
            &compatibility,
        );
        store.write_snapshot(
            build_chunk_snapshot("abutopia", &visible, ChunkActivity::Active),
            &compatibility,
        );

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
        let snapshot = build_chunk_snapshot("abutopia", &chunk, ChunkActivity::Active);
        let compatibility = SnapshotCompatibility::new("abutopia", 1);

        ChunkSnapshotStore::write_snapshot(&mut store, snapshot.clone(), &compatibility)
            .await
            .unwrap();

        let stored =
            ChunkSnapshotStore::read_snapshot(&store, ChunkCoord { x: 4, y: 4 }, &compatibility)
                .await
                .unwrap()
                .expect("snapshot exists");
        assert_eq!(stored, snapshot);
    }

    #[tokio::test]
    async fn chunk_snapshot_store_filters_by_base_world_metadata() {
        let mut store = InMemoryChunkSnapshotStore::default();
        let mut chunk = Chunk::new(ChunkCoord { x: 4, y: 4 }, 32);
        chunk.set_tile_kind(0, TileKind::Road).unwrap();
        let snapshot = build_chunk_snapshot("abutopia", &chunk, ChunkActivity::Active);
        let current = SnapshotCompatibility::new("abutopia", 1);
        let stale = SnapshotCompatibility::new("abutopia", 0);

        ChunkSnapshotStore::write_snapshot(&mut store, snapshot, &stale)
            .await
            .unwrap();

        assert!(
            ChunkSnapshotStore::read_snapshot(&store, ChunkCoord { x: 4, y: 4 }, &current)
                .await
                .unwrap()
                .is_none(),
            "chunk snapshots from another base-world schema must not hydrate current chunks"
        );
    }

    #[tokio::test]
    async fn mobility_snapshot_store_writes_and_reads() {
        use crate::mobility::{extract_from_world, seed};

        let mut store = InMemoryMobilitySnapshotStore::default();
        let (world, _) = seed::test_seed_world();
        let snap = extract_from_world(&world);
        let compatibility = SnapshotCompatibility::new("abutopia", 1);

        MobilitySnapshotStore::write(&mut store, "abutopia", 42, &snap, &compatibility)
            .await
            .unwrap();

        let (tick, restored) = MobilitySnapshotStore::read(&store, "abutopia", &compatibility)
            .await
            .unwrap()
            .expect("snapshot exists");

        assert_eq!(tick, 42);
        assert_eq!(restored, snap);
    }

    #[tokio::test]
    async fn mobility_snapshot_store_filters_by_base_world_metadata() {
        use crate::mobility::{extract_from_world, seed};

        let mut store = InMemoryMobilitySnapshotStore::default();
        let (world, _) = seed::test_seed_world();
        let snap = extract_from_world(&world);
        let current = SnapshotCompatibility::new("abutopia", 1);
        let stale = SnapshotCompatibility::new("legacy-world", 1);

        MobilitySnapshotStore::write(&mut store, "abutopia", 42, &snap, &stale)
            .await
            .unwrap();

        assert!(
            MobilitySnapshotStore::read(&store, "abutopia", &current)
                .await
                .unwrap()
                .is_none(),
            "snapshots without current base-world metadata must be invisible to hydration"
        );
    }

    #[tokio::test]
    async fn mobility_snapshot_store_read_returns_none_for_unknown_world() {
        let store = InMemoryMobilitySnapshotStore::default();
        let result = MobilitySnapshotStore::read(
            &store,
            "missing-world",
            &SnapshotCompatibility::new("abutopia", 1),
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn in_memory_economy_store_round_trips() {
        use crate::economy::EconomyPersistSnapshot;
        let mut store = InMemoryEconomySnapshotStore::default();
        let compat = SnapshotCompatibility::new("abutopia", 1);
        let snap = EconomyPersistSnapshot {
            next_order_id: 99,
            ..Default::default()
        };

        store.write("w1", 7, &snap, &compat).await.unwrap();
        let got = store.read("w1", &compat).await.unwrap();
        assert_eq!(got, Some((7, snap.clone())));

        // Compatibility mismatch -> miss.
        let other = SnapshotCompatibility::new("abutopia", 2);
        assert_eq!(store.read("w1", &other).await.unwrap(), None);
    }
}
