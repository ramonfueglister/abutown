use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, WorldId};
#[cfg(test)]
use sim_core::persistence::InMemoryChunkSnapshotStore;
use sim_core::{
    chunk::{Chunk, ChunkError},
    ids::ChunkCoord,
    persistence::build_chunk_snapshot,
    scheduler::ChunkActivity,
    tile::TileKind,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChunkMutationError {
    ChunkNotLoaded { coord: ChunkCoord },
    NoStateChange { coord: ChunkCoord, local_index: u16 },
    TileOutOfBounds { index: u16, tile_count: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SetTileKindPlan {
    pub(crate) coord: ChunkCoord,
    pub(crate) local_index: u16,
    pub(crate) kind: TileKind,
    pub(crate) version: u64,
}

#[derive(Debug)]
struct LoadedChunk {
    chunk: Chunk,
    activity: ChunkActivity,
    last_persisted_version: u64,
    last_snapshot_at: std::time::Instant,
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
        self.chunks.insert(
            chunk.coord(),
            LoadedChunk {
                chunk,
                activity,
                last_persisted_version: 0,
                last_snapshot_at: std::time::Instant::now(),
            },
        );
    }

    pub(crate) fn insert_hydrated(
        &mut self,
        chunk: Chunk,
        last_persisted_version: u64,
        activity: ChunkActivity,
    ) {
        assert_eq!(
            chunk.chunk_size(),
            self.chunk_size,
            "loaded chunk size must match registry chunk size"
        );
        self.chunks.insert(
            chunk.coord(),
            LoadedChunk {
                chunk,
                activity,
                last_persisted_version,
                last_snapshot_at: std::time::Instant::now(),
            },
        );
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

    #[cfg(test)]
    pub(crate) fn set_tile_kind(
        &mut self,
        coord: ChunkCoord,
        local_index: u16,
        kind: TileKind,
    ) -> Result<u64, ChunkMutationError> {
        let plan = self.plan_set_tile_kind(coord, local_index, kind)?;
        self.apply_set_tile_kind(plan)
    }

    pub(crate) fn plan_set_tile_kind(
        &self,
        coord: ChunkCoord,
        local_index: u16,
        kind: TileKind,
    ) -> Result<SetTileKindPlan, ChunkMutationError> {
        let loaded = self
            .chunks
            .get(&coord)
            .ok_or(ChunkMutationError::ChunkNotLoaded { coord })?;
        let existing_kind =
            loaded
                .chunk
                .kind_at(local_index)
                .ok_or(ChunkMutationError::TileOutOfBounds {
                    index: local_index,
                    tile_count: loaded.chunk.tile_count(),
                })?;

        if existing_kind == kind {
            return Err(ChunkMutationError::NoStateChange { coord, local_index });
        }

        Ok(SetTileKindPlan {
            coord,
            local_index,
            kind,
            version: loaded.chunk.version() + 1,
        })
    }

    pub(crate) fn apply_set_tile_kind(
        &mut self,
        plan: SetTileKindPlan,
    ) -> Result<u64, ChunkMutationError> {
        let loaded = self
            .chunks
            .get_mut(&plan.coord)
            .ok_or(ChunkMutationError::ChunkNotLoaded { coord: plan.coord })?;

        loaded
            .chunk
            .set_tile_kind(plan.local_index, plan.kind)
            .map_err(|error| match error {
                ChunkError::IndexOutOfBounds { index, tile_count } => {
                    ChunkMutationError::TileOutOfBounds { index, tile_count }
                }
                ChunkError::InvalidChunkSize { .. } => {
                    unreachable!("loaded chunks are already valid")
                }
            })?;

        debug_assert_eq!(loaded.chunk.version(), plan.version);
        Ok(loaded.chunk.version())
    }

    #[cfg(test)]
    pub(crate) fn write_snapshots(
        &mut self,
        world_id: &WorldId,
        store: &mut InMemoryChunkSnapshotStore,
    ) -> usize {
        let snapshots = self.collect_snapshots(world_id);
        let coords: Vec<ChunkCoord> = snapshots
            .iter()
            .map(|snapshot| ChunkCoord {
                x: snapshot.coord.x,
                y: snapshot.coord.y,
            })
            .collect();
        let written = snapshots.len();
        for snapshot in snapshots {
            store.write_snapshot(snapshot);
        }

        self.mark_snapshots_persisted(&coords);
        written
    }

    pub(crate) fn collect_snapshots(&self, world_id: &WorldId) -> Vec<ChunkSnapshotDto> {
        let ceiling = std::time::Duration::from_secs(30);
        let now = std::time::Instant::now();
        let mut coords: Vec<ChunkCoord> = self
            .chunks
            .iter()
            .filter(|(_, loaded)| {
                loaded.chunk.version() > loaded.last_persisted_version
                    || now.duration_since(loaded.last_snapshot_at) >= ceiling
            })
            .map(|(coord, _)| *coord)
            .collect();
        coords.sort_by_key(|coord| (coord.y, coord.x));
        coords
            .into_iter()
            .filter_map(|coord| self.chunk_snapshot(world_id, coord))
            .collect()
    }

    pub(crate) fn mark_snapshots_persisted(&mut self, coords: &[ChunkCoord]) {
        let now = std::time::Instant::now();
        for coord in coords {
            if let Some(loaded) = self.chunks.get_mut(coord) {
                loaded.last_persisted_version = loaded.chunk.version();
                loaded.last_snapshot_at = now;
                loaded.chunk.clear_dirty();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(snapshot.tiles.len(), 1);
        assert_eq!(snapshot.tiles[0].local_index, 17);
        assert_eq!(snapshot.tiles[0].kind, abutown_protocol::TileKindDto::Road);
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
    fn registry_sets_tile_kind_on_loaded_chunk() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let version = registry
            .set_tile_kind(ChunkCoord { x: 4, y: 4 }, 11, TileKind::Water)
            .expect("loaded tile can mutate");

        assert_eq!(version, 2);
        let snapshot = registry
            .chunk_snapshot(
                &WorldId("abutown-main".to_string()),
                ChunkCoord { x: 4, y: 4 },
            )
            .expect("chunk snapshot exists");
        assert_eq!(snapshot.tiles.len(), 2);
        assert_eq!(snapshot.tiles[1].local_index, 11);
        assert_eq!(snapshot.tiles[1].kind, abutown_protocol::TileKindDto::Water);
    }

    #[test]
    fn registry_plans_tile_kind_mutation_without_changing_chunk() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let plan = registry
            .plan_set_tile_kind(ChunkCoord { x: 4, y: 4 }, 11, TileKind::Water)
            .expect("loaded tile can be planned");

        assert_eq!(plan.coord, ChunkCoord { x: 4, y: 4 });
        assert_eq!(plan.local_index, 11);
        assert_eq!(plan.kind, TileKind::Water);
        assert_eq!(plan.version, 2);

        let snapshot = registry
            .chunk_snapshot(
                &WorldId("abutown-main".to_string()),
                ChunkCoord { x: 4, y: 4 },
            )
            .expect("chunk snapshot exists");
        assert!(!snapshot.tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }

    #[test]
    fn registry_applies_planned_tile_kind_mutation() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let plan = registry
            .plan_set_tile_kind(ChunkCoord { x: 4, y: 4 }, 11, TileKind::Water)
            .expect("loaded tile can be planned");

        registry
            .apply_set_tile_kind(plan)
            .expect("planned mutation applies");

        let snapshot = registry
            .chunk_snapshot(
                &WorldId("abutown-main".to_string()),
                ChunkCoord { x: 4, y: 4 },
            )
            .expect("chunk snapshot exists");
        assert!(snapshot.tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }

    #[test]
    fn registry_rejects_missing_chunk_mutation() {
        let mut registry = ChunkRegistry::new(32);

        assert!(matches!(
            registry.set_tile_kind(ChunkCoord { x: 9, y: 9 }, 0, TileKind::Road),
            Err(ChunkMutationError::ChunkNotLoaded { coord }) if coord == ChunkCoord { x: 9, y: 9 }
        ));
    }

    #[test]
    fn registry_rejects_out_of_bounds_tile_mutation() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        assert!(matches!(
            registry.set_tile_kind(ChunkCoord { x: 4, y: 4 }, 2000, TileKind::Water),
            Err(ChunkMutationError::TileOutOfBounds {
                index: 2000,
                tile_count: 1024
            })
        ));
    }

    #[test]
    fn registry_rejects_no_op_tile_mutation() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        assert!(matches!(
            registry.set_tile_kind(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            Err(ChunkMutationError::NoStateChange {
                coord,
                local_index: 0
            }) if coord == ChunkCoord { x: 4, y: 4 }
        ));
    }

    #[test]
    fn registry_writes_snapshots_and_clears_dirty_state() {
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
                .tiles
                .len(),
            1
        );

        // After persisting, a fresh write with no new events and within the
        // 30s ceiling must skip both chunks.
        assert_eq!(registry.write_snapshots(&world_id, &mut store), 0);
        // Previously-stored snapshot rows remain intact.
        assert_eq!(
            store
                .read_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("visible snapshot still exists")
                .tiles
                .len(),
            1
        );

        // A new event on one chunk re-arms only that chunk for the next
        // collect.
        registry
            .set_tile_kind(ChunkCoord { x: 4, y: 4 }, 9, TileKind::Water)
            .expect("loaded tile can mutate");
        assert_eq!(registry.write_snapshots(&world_id, &mut store), 1);
        assert_eq!(
            store
                .read_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("visible snapshot has the new tile")
                .tiles
                .len(),
            2
        );
    }

    #[test]
    fn registry_collects_snapshots_without_clearing_dirty_state() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 3, TileKind::Road),
            ChunkActivity::Active,
        );
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 5, y: 4 }, 7, TileKind::Water),
            ChunkActivity::Warm,
        );
        let world_id = WorldId("abutown-main".to_string());

        let snapshots = registry.collect_snapshots(&world_id);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].tiles.len(), 1);

        // collect_snapshots is non-destructive; a second call with no
        // persistence in between must still yield both chunks.
        let collected_again = registry.collect_snapshots(&world_id);
        assert_eq!(collected_again.len(), 2);
        assert_eq!(collected_again[0].tiles.len(), 1);

        registry.mark_snapshots_persisted(&[ChunkCoord { x: 4, y: 4 }]);

        // Only the unmarked chunk should remain in the next collect — the
        // marked chunk has no new events and is within the 30s ceiling.
        let after_partial_mark = registry.collect_snapshots(&world_id);
        assert_eq!(after_partial_mark.len(), 1);
        assert_eq!(after_partial_mark[0].coord.x, 5);
        assert_eq!(after_partial_mark[0].coord.y, 4);
        assert_eq!(after_partial_mark[0].tiles.len(), 1);
    }

    #[test]
    fn collect_snapshots_skips_chunks_with_no_new_events_within_snapshot_ceiling() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let world_id = WorldId("abutown-main".to_string());

        let first = registry.collect_snapshots(&world_id);
        assert_eq!(first.len(), 1, "first call must include the dirty chunk");
        let coords: Vec<ChunkCoord> = first
            .iter()
            .map(|s| ChunkCoord { x: s.coord.x, y: s.coord.y })
            .collect();
        registry.mark_snapshots_persisted(&coords);

        let second = registry.collect_snapshots(&world_id);
        assert!(
            second.is_empty(),
            "second call without new events and within 30s must produce no snapshots"
        );
    }

    #[test]
    fn collect_snapshots_emits_again_after_new_event() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let world_id = WorldId("abutown-main".to_string());
        let coords: Vec<ChunkCoord> = registry
            .collect_snapshots(&world_id)
            .iter()
            .map(|s| ChunkCoord { x: s.coord.x, y: s.coord.y })
            .collect();
        registry.mark_snapshots_persisted(&coords);

        registry
            .set_tile_kind(ChunkCoord { x: 4, y: 4 }, 5, TileKind::Water)
            .unwrap();

        let next = registry.collect_snapshots(&world_id);
        assert_eq!(next.len(), 1, "new event must produce a new snapshot candidate");
    }

    #[test]
    fn insert_hydrated_skips_redundant_snapshot_when_version_matches() {
        let mut registry = ChunkRegistry::new(32);
        let chunk = chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road);
        let restored_version = chunk.version();
        registry.insert_hydrated(chunk, restored_version, ChunkActivity::Active);

        let world_id = WorldId("abutown-main".to_string());
        let snapshots = registry.collect_snapshots(&world_id);
        assert!(
            snapshots.is_empty(),
            "hydrated chunk with version == last_persisted_version must not generate a redundant snapshot"
        );
    }
}
