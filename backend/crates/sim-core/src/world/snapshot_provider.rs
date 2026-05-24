use bevy_ecs::prelude::*;

use crate::persistence::build_chunk_snapshot_from_parts;
use crate::scheduler::ChunkActivity;
use crate::world::components::{
    ActiveChunk, ChunkVersion, HotChunk, LastPersistedVersion, Tiles, WarmChunk,
};
use crate::world::persistence::{MigrationError, SnapshotItem, SnapshotKey, SnapshotProvider};
use crate::world::resources::ChunksByCoord;

/// A `SnapshotProvider` that emits per-chunk snapshots compatible with the
/// existing Postgres `chunk_snapshots` JSONB schema. The serialised payload is
/// byte-identical to `build_chunk_snapshot_from_parts` output, so the
/// migration is wire-stable for persistence.
pub struct ChunkSnapshotProvider {
    pub world_id: String,
}

impl SnapshotProvider for ChunkSnapshotProvider {
    fn name(&self) -> &'static str {
        "chunk"
    }

    fn schema_version(&self) -> u32 {
        1
    }

    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        let by_coord = world.resource::<ChunksByCoord>();

        // Snapshot (coord, entity) pairs while holding the resource ref, then
        // release it before the per-entity component reads.
        let entries: Vec<_> = by_coord.0.iter().map(|(c, e)| (*c, *e)).collect();

        let mut items = Vec::new();
        for (coord, entity) in entries {
            let version = match world.get::<ChunkVersion>(entity) {
                Some(v) => v.0,
                None => continue,
            };
            let last_persisted = match world.get::<LastPersistedVersion>(entity) {
                Some(lp) => lp.0,
                None => continue,
            };
            // Only emit chunks that have unsaved changes.
            if version <= last_persisted {
                continue;
            }
            let tiles = match world.get::<Tiles>(entity) {
                Some(t) => &t.0,
                None => continue,
            };
            let activity = if world.get::<HotChunk>(entity).is_some() {
                ChunkActivity::Hot
            } else if world.get::<ActiveChunk>(entity).is_some() {
                ChunkActivity::Active
            } else if world.get::<WarmChunk>(entity).is_some() {
                ChunkActivity::Warm
            } else {
                // AsleepChunk or no marker — treat as asleep.
                ChunkActivity::Asleep
            };

            let dto =
                build_chunk_snapshot_from_parts(&self.world_id, coord, tiles, version, activity);
            let payload = serde_json::to_vec(&dto).expect("serde always encodes ChunkSnapshotDto");

            items.push(SnapshotItem {
                key: SnapshotKey {
                    world_id: self.world_id.clone(),
                    kind: "chunk",
                    identifier: format!("{}:{}", coord.x, coord.y),
                },
                schema_version: 1,
                payload,
            });
        }
        items
    }

    fn migrate(
        &self,
        raw: SnapshotItem,
        _from_version: u32,
    ) -> Result<SnapshotItem, MigrationError> {
        // Schema version 1 is the only version; nothing to migrate.
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::{TileKind, TileRecord};
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use crate::world::systems::spawn_chunk_entity;
    use bevy_ecs::schedule::Schedule;

    fn make_world_with_dirty_chunk() -> World {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);

        let coord = ChunkCoord { x: 1, y: 2 };
        let mut tiles = vec![TileRecord::default(); 4];
        tiles[0].kind = TileKind::Road;
        tiles[0].version = 1;

        let entity = spawn_chunk_entity(&mut world, coord, 2, tiles, 0, ChunkActivity::Active);

        // Simulate a dirty chunk: version > last_persisted.
        world
            .entity_mut(entity)
            .get_mut::<ChunkVersion>()
            .unwrap()
            .0 = 1;
        // LastPersistedVersion stays at 0 (from spawn), so version (1) > last_persisted (0).

        world
    }

    #[test]
    fn collect_emits_snapshot_for_dirty_chunk() {
        let world = make_world_with_dirty_chunk();
        let provider = ChunkSnapshotProvider {
            world_id: "test-world".to_string(),
        };

        let items = provider.collect(&world);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key.kind, "chunk");
        assert_eq!(items[0].key.identifier, "1:2");
        assert_eq!(items[0].schema_version, 1);
        assert!(!items[0].payload.is_empty());
    }

    #[test]
    fn collect_skips_clean_chunks() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);

        let coord = ChunkCoord { x: 3, y: 4 };
        let tiles = vec![TileRecord::default(); 4];
        let entity = spawn_chunk_entity(&mut world, coord, 2, tiles, 5, ChunkActivity::Warm);

        // Ensure last_persisted == version (clean).
        world
            .entity_mut(entity)
            .get_mut::<LastPersistedVersion>()
            .unwrap()
            .0 = 5;

        let provider = ChunkSnapshotProvider {
            world_id: "test-world".to_string(),
        };
        let items = provider.collect(&world);
        assert_eq!(items.len(), 0, "clean chunk must not be emitted");
    }

    #[test]
    fn migrate_is_identity_for_v1() {
        let provider = ChunkSnapshotProvider {
            world_id: "test-world".to_string(),
        };
        let item = SnapshotItem {
            key: SnapshotKey {
                world_id: "test-world".to_string(),
                kind: "chunk",
                identifier: "1:2".to_string(),
            },
            schema_version: 1,
            payload: b"data".to_vec(),
        };
        let result = provider.migrate(item.clone(), 1).unwrap();
        assert_eq!(result.payload, item.payload);
    }

    #[test]
    fn provider_name_and_schema_version() {
        let provider = ChunkSnapshotProvider {
            world_id: "x".to_string(),
        };
        assert_eq!(provider.name(), "chunk");
        assert_eq!(provider.schema_version(), 1);
    }
}
