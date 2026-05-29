use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NetworkPoint {
    pub x: f32,
    pub y: f32,
}

impl From<NetworkCoord> for NetworkPoint {
    fn from(coord: NetworkCoord) -> Self {
        Self {
            x: coord.x as f32,
            y: coord.y as f32,
        }
    }
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
    pub arterial_paths: Vec<Vec<NetworkPoint>>,
    pub pedestrian_corridors: Vec<Vec<NetworkPoint>>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "version": 1,
        "world_id": "abutopia",
        "chunk_size": 32,
        "world_tiles": { "width": 16, "height": 8 },
        "arterial_paths": [],
        "pedestrian_corridors": [
            [{"x": 2, "y": 3}, {"x": 13, "y": 3}]
        ]
    }"#;

    #[test]
    fn parses_fixture_with_paths_and_corridors() {
        let network = CityNetwork::from_json(FIXTURE).expect("parses");
        assert_eq!(network.world_id, "abutopia");
        assert_eq!(network.chunk_size, 32);
        assert_eq!(network.arterial_paths.len(), 0);
        assert_eq!(network.pedestrian_corridors.len(), 1);
        assert_eq!(
            network.pedestrian_corridors[0][0],
            NetworkPoint { x: 2.0, y: 3.0 }
        );
    }

    #[test]
    fn parses_fractional_transport_points() {
        let fixture = r#"{
            "version": 1,
            "world_id": "abutopia",
            "chunk_size": 32,
            "world_tiles": { "width": 16, "height": 8 },
            "arterial_paths": [
                [{"x": 3, "y": 3}, {"x": 12, "y": 3}]
            ],
            "pedestrian_corridors": [
                [{"x": 2, "y": 2.49}, {"x": 13, "y": 2.49}],
                [{"x": 2, "y": 3.51}, {"x": 13, "y": 3.51}]
            ]
        }"#;

        let network = CityNetwork::from_json(fixture).expect("parses fractional points");

        assert_eq!(
            network.arterial_paths[0][0],
            NetworkPoint { x: 3.0, y: 3.0 }
        );
        assert_eq!(
            network.pedestrian_corridors[0][0],
            NetworkPoint { x: 2.0, y: 2.49 },
        );
        assert_eq!(
            network.pedestrian_corridors[1][1],
            NetworkPoint { x: 13.0, y: 3.51 },
        );
    }

    #[test]
    fn rejects_payload_without_required_fields() {
        let bad = r#"{"version": 1}"#;
        assert!(CityNetwork::from_json(bad).is_err());
    }
}
