use bevy_ecs::prelude::*;

use crate::world::persistence::{
    MigrationError, SnapshotItem, SnapshotKey, SnapshotProvider,
};

/// A `SnapshotProvider` that emits the mobility persist snapshot as a
/// single item. Today this returns the full mobility world serialized
/// to JSON; the persist loop dispatches by `key.kind == "mobility"` to
/// the mobility-snapshot store. Schema is byte-identical to the
/// pre-Phase-8a mobility_snapshots JSONB column.
pub struct MobilitySnapshotProvider {
    pub world_id: String,
}

impl SnapshotProvider for MobilitySnapshotProvider {
    fn name(&self) -> &'static str { "mobility" }
    fn schema_version(&self) -> u32 { 1 }

    fn collect(&self, world: &World) -> Vec<SnapshotItem> {
        // Reuse the existing MobilityPersistSnapshot extraction.
        let snapshot = crate::mobility::persist_snapshot::extract_from_world(world);
        let payload = serde_json::to_vec(&snapshot)
            .expect("serde encodes MobilityPersistSnapshot");
        vec![SnapshotItem {
            key: SnapshotKey {
                world_id: self.world_id.clone(),
                kind: "mobility",
                identifier: "full".to_string(),
            },
            schema_version: 1,
            payload,
        }]
    }

    fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::schedule::SimPlugin;

    #[test]
    fn name_and_schema_version() {
        let p = MobilitySnapshotProvider { world_id: "test".to_string() };
        assert_eq!(p.name(), "mobility");
        assert_eq!(p.schema_version(), 1);
    }

    #[test]
    fn collect_returns_single_item_with_full_identifier() {
        // Build a minimal world via mobility's install path
        let mut world = bevy_ecs::world::World::new();
        let mut schedule = bevy_ecs::schedule::Schedule::default();
        crate::world::plugin::CorePlugin::default().install(&mut world, &mut schedule);
        crate::mobility::api::install_mobility(&mut world, &mut schedule);

        let provider = MobilitySnapshotProvider { world_id: "test".to_string() };
        let items = provider.collect(&world);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key.kind, "mobility");
        assert_eq!(items[0].key.identifier, "full");
        assert_eq!(items[0].schema_version, 1);
        // Payload is non-empty JSON
        assert!(!items[0].payload.is_empty());
    }
}
