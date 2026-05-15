use abutown_protocol::WorldEventDto;

#[derive(Debug, Default)]
pub struct InMemoryWorldEventStore {
    events: Vec<WorldEventDto>,
}

impl InMemoryWorldEventStore {
    pub fn append(&mut self, event: WorldEventDto) {
        self.events.push(event);
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn events(&self) -> &[WorldEventDto] {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abutown_protocol::{
        ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId,
    };

    fn tile_event(event_id: &str, version: u64) -> WorldEventDto {
        WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: event_id.to_string(),
            command_id: format!("command:{event_id}"),
            world_id: WorldId("abutown-main".to_string()),
            tick: version,
            version,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 3,
            kind: TileKindDto::Road,
        })
    }

    #[test]
    fn event_store_appends_events_in_order() {
        let mut store = InMemoryWorldEventStore::default();

        store.append(tile_event("event:1", 1));
        store.append(tile_event("event:2", 2));

        assert_eq!(store.event_count(), 2);
        assert_eq!(store.events()[0], tile_event("event:1", 1));
        assert_eq!(store.events()[1], tile_event("event:2", 2));
    }
}
