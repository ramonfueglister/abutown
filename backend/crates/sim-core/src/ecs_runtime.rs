use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::ids::{ChunkCoord, StableEntityId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializedKind {
    Player,
    Item,
    Machine,
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct StableIdComponent(pub StableEntityId);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkLocationComponent(pub ChunkCoord);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializedKindComponent(pub MaterializedKind);

#[derive(Default)]
pub struct MaterializedRuntime {
    world: World,
    by_stable_id: HashMap<StableEntityId, Entity>,
}

impl MaterializedRuntime {
    pub fn spawn_materialized(
        &mut self,
        stable_id: StableEntityId,
        chunk: ChunkCoord,
        kind: MaterializedKind,
    ) -> Entity {
        if let Some(entity) = self.by_stable_id.get(&stable_id) {
            return *entity;
        }

        let entity = self
            .world
            .spawn((
                StableIdComponent(stable_id.clone()),
                ChunkLocationComponent(chunk),
                MaterializedKindComponent(kind),
            ))
            .id();

        self.by_stable_id.insert(stable_id, entity);
        entity
    }

    pub fn lookup(&self, stable_id: &StableEntityId) -> Option<Entity> {
        self.by_stable_id.get(stable_id).copied()
    }

    pub fn materialized_count(&self) -> usize {
        self.by_stable_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ChunkCoord, StableEntityId};

    #[test]
    fn materialized_entities_keep_stable_ids_outside_ecs_indices() {
        let mut runtime = MaterializedRuntime::default();
        let stable_id = StableEntityId("item:bench:0001".to_string());

        let entity = runtime.spawn_materialized(
            stable_id.clone(),
            ChunkCoord { x: 0, y: 0 },
            MaterializedKind::Item,
        );

        assert_eq!(runtime.lookup(&stable_id), Some(entity));
        assert_eq!(runtime.materialized_count(), 1);
    }
}
