use bevy_ecs::prelude::*;

use crate::ids::ChunkCoord;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ChunkLod {
    Asleep,
    Warm,
    Active,
    Hot,
}

#[derive(Message, Debug)]
pub struct ChunkLoaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub initial_version: u64,
}

#[derive(Message, Debug)]
pub struct ChunkUnloaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
}

#[derive(Message, Debug)]
pub struct ChunkLodChanged {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub from: ChunkLod,
    pub to: ChunkLod,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construct a dummy entity for tests — no World needed since Messages<T>
    /// is a standalone resource and we never look up the entity in a World.
    fn dummy_entity() -> Entity {
        Entity::PLACEHOLDER
    }

    #[test]
    fn chunk_loaded_can_be_written_and_read_via_buffered_messages() {
        let mut messages = Messages::<ChunkLoaded>::default();

        messages.write(ChunkLoaded {
            entity: dummy_entity(),
            coord: ChunkCoord { x: 3, y: 7 },
            initial_version: 42,
        });

        let mut cursor = messages.get_cursor();
        let read: Vec<_> = cursor.read(&messages).collect();

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].coord, ChunkCoord { x: 3, y: 7 });
        assert_eq!(read[0].initial_version, 42);
    }

    #[test]
    fn multiple_event_types_are_independent() {
        let mut loaded = Messages::<ChunkLoaded>::default();
        let mut unloaded = Messages::<ChunkUnloaded>::default();

        loaded.write(ChunkLoaded {
            entity: dummy_entity(),
            coord: ChunkCoord { x: 0, y: 0 },
            initial_version: 1,
        });
        unloaded.write(ChunkUnloaded {
            entity: dummy_entity(),
            coord: ChunkCoord { x: 5, y: 5 },
        });

        let mut c1 = loaded.get_cursor();
        let mut c2 = unloaded.get_cursor();

        let r1: Vec<_> = c1.read(&loaded).collect();
        let r2: Vec<_> = c2.read(&unloaded).collect();

        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        assert_eq!(r1[0].coord, ChunkCoord { x: 0, y: 0 });
        assert_eq!(r2[0].coord, ChunkCoord { x: 5, y: 5 });
    }

    #[test]
    fn cursor_does_not_re_read_consumed_events() {
        let mut messages = Messages::<ChunkLodChanged>::default();

        messages.write(ChunkLodChanged {
            entity: dummy_entity(),
            coord: ChunkCoord { x: 1, y: 1 },
            from: ChunkLod::Asleep,
            to: ChunkLod::Active,
        });

        let mut cursor = messages.get_cursor();
        let first_count = cursor.read(&messages).count();
        let second_count = cursor.read(&messages).count();

        assert_eq!(first_count, 1);
        assert_eq!(
            second_count, 0,
            "cursor must not re-read already-consumed events"
        );
    }
}
