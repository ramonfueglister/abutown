use bevy_ecs::prelude::*;

use crate::ids::ChunkCoord;
use crate::tile::TileKind;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ChunkLod { Asleep, Warm, Active, Hot }

#[derive(Event, Debug)]
pub struct ChunkLoaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub initial_version: u64,
}

#[derive(Event, Debug)]
pub struct ChunkUnloaded {
    pub entity: Entity,
    pub coord: ChunkCoord,
}

#[derive(Event, Debug)]
pub struct TileChanged {
    pub chunk: Entity,
    pub coord: ChunkCoord,
    pub local_index: u16,
    pub old_kind: TileKind,
    pub new_kind: TileKind,
    pub new_version: u64,
    pub tick: u64,
}

#[derive(Event, Debug)]
pub struct ChunkLodChanged {
    pub entity: Entity,
    pub coord: ChunkCoord,
    pub from: ChunkLod,
    pub to: ChunkLod,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;

    #[test]
    fn events_can_be_written_and_read() {
        // In bevy_ecs 0.18, #[derive(Event)] uses the observer pattern.
        // Trigger an event and verify the observer fires with the correct payload.
        let mut world = World::new();
        let entity = world.spawn_empty().id();

        let received_coord = std::sync::Arc::new(std::sync::Mutex::new(None::<ChunkCoord>));
        let received_coord_clone = received_coord.clone();

        world.add_observer(move |ev: On<ChunkLoaded>| {
            *received_coord_clone.lock().unwrap() = Some(ev.coord);
        });
        world.flush();

        world.trigger(ChunkLoaded {
            entity,
            coord: ChunkCoord { x: 1, y: 2 },
            initial_version: 0,
        });

        let coord = received_coord.lock().unwrap();
        assert_eq!(*coord, Some(ChunkCoord { x: 1, y: 2 }));
    }
}
