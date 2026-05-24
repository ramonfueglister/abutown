use bevy_ecs::prelude::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct SnapshotKey {
    pub world_id: String,
    pub kind: &'static str,
    pub identifier: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotItem {
    pub key: SnapshotKey,
    pub schema_version: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("no migration registered from version {from} to {to} for kind {kind}")]
    NoMigration {
        kind: &'static str,
        from: u32,
        to: u32,
    },
    #[error("migration failure: {0}")]
    Other(String),
}

pub trait SnapshotProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema_version(&self) -> u32;
    fn collect(&self, world: &World) -> Vec<SnapshotItem>;
    fn migrate(&self, raw: SnapshotItem, from_version: u32)
    -> Result<SnapshotItem, MigrationError>;
}

#[derive(Resource, Default)]
pub struct SnapshotProviders(pub Vec<Box<dyn SnapshotProvider>>);

#[derive(Resource, Default)]
pub struct MigrationRegistry {
    by_kind: HashMap<&'static str, Vec<(u32, u32)>>,
}

impl MigrationRegistry {
    pub fn register(&mut self, kind: &'static str, from: u32, to: u32) {
        self.by_kind.entry(kind).or_default().push((from, to));
    }

    pub fn registered_for(&self, kind: &'static str) -> &[(u32, u32)] {
        self.by_kind.get(kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyProvider;
    impl SnapshotProvider for DummyProvider {
        fn name(&self) -> &'static str {
            "dummy"
        }
        fn schema_version(&self) -> u32 {
            1
        }
        fn collect(&self, _w: &World) -> Vec<SnapshotItem> {
            vec![]
        }
        fn migrate(&self, raw: SnapshotItem, _from: u32) -> Result<SnapshotItem, MigrationError> {
            Ok(raw)
        }
    }

    #[test]
    fn snapshot_providers_can_register_and_iterate() {
        let mut reg = SnapshotProviders::default();
        reg.0.push(Box::new(DummyProvider));
        assert_eq!(reg.0.len(), 1);
        assert_eq!(reg.0[0].name(), "dummy");
    }

    #[test]
    fn migration_registry_remembers_pairs() {
        let mut reg = MigrationRegistry::default();
        reg.register("chunk", 1, 2);
        reg.register("chunk", 2, 3);
        assert_eq!(reg.registered_for("chunk").len(), 2);
        assert_eq!(reg.registered_for("agent").len(), 0);
    }
}
