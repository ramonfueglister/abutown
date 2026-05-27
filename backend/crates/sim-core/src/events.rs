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
pub trait WorldEventStore: std::fmt::Debug + Send + Sync {
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
        match event {}
        #[allow(unreachable_code)]
        Ok(())
    }

    async fn find_event_by_command(
        &self,
        world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, WorldEventStoreError> {
        let _ = (world_id, command_id);
        Ok(None)
    }

    async fn read_chunk_events_since(
        &self,
        world_id: &str,
        coord: abutown_protocol::ChunkCoordDto,
        after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, WorldEventStoreError> {
        let _ = (world_id, coord, after_chunk_version);
        Ok(Vec::new())
    }

    async fn max_tick(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        let _ = world_id;
        Ok(None)
    }

    async fn max_version(&self, world_id: &str) -> Result<Option<u64>, WorldEventStoreError> {
        let _ = world_id;
        Ok(None)
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

    #[tokio::test]
    async fn event_store_is_empty_transition_dependency() {
        let store = InMemoryWorldEventStore::default();

        let found =
            WorldEventStore::find_event_by_command(&store, "abutown-main", "command:event:1")
                .await
                .unwrap();
        assert_eq!(found, None);

        let events = WorldEventStore::read_chunk_events_since(
            &store,
            "abutown-main",
            abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
            0,
        )
        .await
        .unwrap();
        assert!(events.is_empty());
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
        assert_eq!(store.event_count(), 0);
        assert!(store.events().is_empty());
    }

    #[tokio::test]
    async fn failing_event_store_returns_typed_error_for_reads() {
        let store = FailingWorldEventStore::new("database offline");
        let error = WorldEventStore::find_event_by_command(&store, "abutown-main", "command:1")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "event_store_unavailable");
        assert!(error.to_string().contains("database offline"));
        assert_eq!(
            WorldEventStore::max_tick(&store, "abutown-main")
                .await
                .unwrap_err()
                .code(),
            "event_store_unavailable"
        );
    }
}
