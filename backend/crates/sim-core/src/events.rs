use std::collections::HashSet;

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

    pub fn duplicate_command(command_id: impl Into<String>) -> Self {
        Self {
            code: "duplicate_command_id",
            message: format!("command_id already present: {}", command_id.into()),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[async_trait]
pub trait WorldEventStore: std::fmt::Debug + Send {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError>;

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError>;

    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError>;

    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError>;

    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryWorldEventStore {
    events: Vec<WorldEventDto>,
    command_keys: HashSet<(String, String)>,
}

impl InMemoryWorldEventStore {
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
        let metadata = WorldEventMetadata::from_event(&event);
        let key = (metadata.world_id.clone(), metadata.command_id.clone());
        if !self.command_keys.insert(key) {
            return Err(WorldEventStoreError::duplicate_command(metadata.command_id));
        }
        self.events.push(event);
        Ok(())
    }

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        Ok(self
            .events
            .iter()
            .find(|event| {
                let m = WorldEventMetadata::from_event(event);
                m.world_id == world_id && m.command_id == command_id
            })
            .cloned())
    }

    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        let mut matches: Vec<WorldEventDto> = self
            .events
            .iter()
            .filter(|event| {
                let m = WorldEventMetadata::from_event(event);
                if m.world_id != world_id {
                    return false;
                }
                match event {
                    WorldEventDto::TileKindSet(payload) => {
                        payload.coord.x == coord.x
                            && payload.coord.y == coord.y
                            && payload.version > after_chunk_version
                    }
                }
            })
            .cloned()
            .collect();
        matches.sort_by_key(|event| match event {
            WorldEventDto::TileKindSet(payload) => payload.version,
        });
        Ok(matches)
    }

    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Ok(self
            .events
            .iter()
            .filter(|event| WorldEventMetadata::from_event(event).world_id == world_id)
            .map(|event| WorldEventMetadata::from_event(event).tick)
            .max())
    }

    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Ok(self
            .events
            .iter()
            .filter(|event| WorldEventMetadata::from_event(event).world_id == world_id)
            .map(|event| WorldEventMetadata::from_event(event).version)
            .max())
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
    async fn find_event_by_command(
        &self,
        _world_id: &str,
        _command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn read_chunk_events_since(
        &self,
        _world_id: &str,
        _coord: abutown_protocol::ChunkCoordDto,
        _after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn max_tick(&self, _world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
    async fn max_version(&self, _world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
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

    #[tokio::test]
    async fn event_store_finds_event_by_command_id() {
        let mut store = InMemoryWorldEventStore::default();
        WorldEventStore::append(&mut store, tile_event("event:1", 1))
            .await
            .unwrap();
        WorldEventStore::append(&mut store, tile_event("event:2", 2))
            .await
            .unwrap();

        let found =
            WorldEventStore::find_event_by_command(&store, "abutown-main", "command:event:1")
                .await
                .unwrap();
        assert_eq!(found, Some(tile_event("event:1", 1)));

        let missing =
            WorldEventStore::find_event_by_command(&store, "abutown-main", "command:nope")
                .await
                .unwrap();
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn event_store_reads_chunk_events_since_version() {
        let mut store = InMemoryWorldEventStore::default();
        WorldEventStore::append(&mut store, tile_event("event:1", 1))
            .await
            .unwrap();
        WorldEventStore::append(&mut store, tile_event("event:2", 2))
            .await
            .unwrap();
        WorldEventStore::append(&mut store, tile_event("event:3", 3))
            .await
            .unwrap();

        let events = WorldEventStore::read_chunk_events_since(
            &store,
            "abutown-main",
            ChunkCoordDto { x: 4, y: 4 },
            1,
        )
        .await
        .unwrap();

        assert_eq!(
            events,
            vec![tile_event("event:2", 2), tile_event("event:3", 3)]
        );
    }

    #[tokio::test]
    async fn event_store_filters_chunk_events_by_coord_and_world() {
        let mut store = InMemoryWorldEventStore::default();
        // Same world, different coord.
        let other_coord = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:other".to_string(),
            command_id: "command:event:other".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            tick: 4,
            version: 4,
            coord: ChunkCoordDto { x: 9, y: 9 },
            local_index: 1,
            kind: TileKindDto::Road,
        });
        // Different world, target coord.
        let other_world = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:other-world".to_string(),
            command_id: "command:event:other-world".to_string(),
            world_id: WorldId("other-world".to_string()),
            tick: 5,
            version: 5,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 2,
            kind: TileKindDto::Road,
        });
        let target = tile_event("event:target", 6);

        WorldEventStore::append(&mut store, other_coord)
            .await
            .unwrap();
        WorldEventStore::append(&mut store, other_world)
            .await
            .unwrap();
        WorldEventStore::append(&mut store, target.clone())
            .await
            .unwrap();

        let events = WorldEventStore::read_chunk_events_since(
            &store,
            "abutown-main",
            ChunkCoordDto { x: 4, y: 4 },
            0,
        )
        .await
        .unwrap();

        assert_eq!(
            events,
            vec![target],
            "must exclude events from other coords and other worlds"
        );
    }

    #[tokio::test]
    async fn event_store_reports_max_tick_and_version() {
        let mut store = InMemoryWorldEventStore::default();
        assert_eq!(
            WorldEventStore::max_tick(&store, "abutown-main")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            WorldEventStore::max_version(&store, "abutown-main")
                .await
                .unwrap(),
            None
        );

        WorldEventStore::append(&mut store, tile_event("event:1", 5))
            .await
            .unwrap();
        WorldEventStore::append(&mut store, tile_event("event:2", 9))
            .await
            .unwrap();

        assert_eq!(
            WorldEventStore::max_tick(&store, "abutown-main")
                .await
                .unwrap(),
            Some(9)
        );
        assert_eq!(
            WorldEventStore::max_version(&store, "abutown-main")
                .await
                .unwrap(),
            Some(9)
        );
    }
}
