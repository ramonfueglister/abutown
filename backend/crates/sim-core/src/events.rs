use abutown_protocol::WorldEventDto;
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldEventMetadata {
    pub event_id: String,
    pub world_id: String,
    pub command_id: String,
    pub event_type: &'static str,
    pub tick: u64,
    pub version: u64,
}

impl WorldEventMetadata {
    pub fn from_event(event: &WorldEventDto) -> Self {
        match event {
            WorldEventDto::TileKindSet(event) => Self {
                event_id: event.event_id.clone(),
                world_id: event.world_id.0.clone(),
                command_id: event.command_id.clone(),
                event_type: "tile_kind_set",
                tick: event.tick,
                version: event.version,
            },
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct WorldEventStoreError {
    code: &'static str,
    message: String,
}

impl WorldEventStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "event_store_unavailable",
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[async_trait]
pub trait WorldEventStore: std::fmt::Debug + Send {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError>;
}

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

#[async_trait]
impl WorldEventStore for InMemoryWorldEventStore {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        InMemoryWorldEventStore::append(self, event);
        Ok(())
    }
}

#[derive(Debug)]
pub struct FailingWorldEventStore {
    message: String,
}

impl FailingWorldEventStore {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn event_count(&self) -> usize {
        0
    }
}

#[async_trait]
impl WorldEventStore for FailingWorldEventStore {
    async fn append(&mut self, _event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
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

    #[tokio::test]
    async fn event_store_appends_events_in_order() {
        let mut store = InMemoryWorldEventStore::default();

        WorldEventStore::append(&mut store, tile_event("event:1", 1))
            .await
            .unwrap();
        WorldEventStore::append(&mut store, tile_event("event:2", 2))
            .await
            .unwrap();

        assert_eq!(store.event_count(), 2);
        assert_eq!(store.events()[0], tile_event("event:1", 1));
        assert_eq!(store.events()[1], tile_event("event:2", 2));
    }

    #[tokio::test]
    async fn failing_event_store_returns_typed_error_without_appending() {
        let mut store = FailingWorldEventStore::new("database offline");

        let error = WorldEventStore::append(&mut store, tile_event("event:1", 1))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "event_store_unavailable");
        assert!(error.to_string().contains("database offline"));
        assert_eq!(store.event_count(), 0);
    }

    #[test]
    fn event_metadata_extracts_queryable_columns() {
        let metadata = WorldEventMetadata::from_event(&tile_event("event:7", 7));

        assert_eq!(metadata.event_id, "event:7");
        assert_eq!(metadata.world_id, "abutown-main");
        assert_eq!(metadata.command_id, "command:event:7");
        assert_eq!(metadata.event_type, "tile_kind_set");
        assert_eq!(metadata.tick, 7);
        assert_eq!(metadata.version, 7);
    }
}
