use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldTiles {
    pub width: u32,
    pub height: u32,
}

#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityNetwork {
    pub version: u32,
    pub world_id: String,
    pub chunk_size: u16,
    pub world_tiles: WorldTiles,
    pub arterial_paths: Vec<Vec<NetworkCoord>>,
    pub pedestrian_corridors: Vec<Vec<NetworkCoord>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CityNetworkError {
    #[error("failed to read city network: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse city network: {0}")]
    Parse(#[from] serde_json::Error),
}

impl CityNetwork {
    pub fn from_json(json: &str) -> Result<Self, CityNetworkError> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, CityNetworkError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_json(&contents)
    }

    /// Alias for [`from_path`] — preferred name for callers that distinguish
    /// "load" (disk I/O) from constructors.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, CityNetworkError> {
        Self::from_path(path)
    }

    /// Return an empty network for the given world — used as a fallback when
    /// the city-network JSON cannot be loaded at startup.
    pub fn empty_for_world(world_id: &str) -> Self {
        Self {
            version: 1,
            world_id: world_id.to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles { width: 256, height: 256 },
            arterial_paths: Vec::new(),
            pedestrian_corridors: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "version": 1,
        "world_id": "zurich-river-city-v1",
        "chunk_size": 32,
        "world_tiles": { "width": 256, "height": 256 },
        "arterial_paths": [
            [{"x": 10, "y": 20}, {"x": 14, "y": 20}, {"x": 14, "y": 24}]
        ],
        "pedestrian_corridors": [
            [{"x": 11, "y": 30}, {"x": 15, "y": 30}]
        ]
    }"#;

    #[test]
    fn parses_fixture_with_paths_and_corridors() {
        let network = CityNetwork::from_json(FIXTURE).expect("parses");
        assert_eq!(network.world_id, "zurich-river-city-v1");
        assert_eq!(network.chunk_size, 32);
        assert_eq!(network.arterial_paths.len(), 1);
        assert_eq!(network.arterial_paths[0].len(), 3);
        assert_eq!(network.arterial_paths[0][0], NetworkCoord { x: 10, y: 20 });
        assert_eq!(network.pedestrian_corridors.len(), 1);
    }

    #[test]
    fn rejects_payload_without_required_fields() {
        let bad = r#"{"version": 1}"#;
        assert!(CityNetwork::from_json(bad).is_err());
    }
}
