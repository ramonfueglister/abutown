use std::collections::HashMap;

use abutown_protocol::{DirectionDto, RoadVehicleDto, WorldCoordDto, WorldId};
use serde::{Deserialize, Serialize};

use crate::ids::{RoadVehicleId, TileCoord};
use crate::mobility_geometry::direction_from_delta;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleRecord {
    pub id: RoadVehicleId,
    pub path: Vec<TileCoord>,
    pub offset: f32,
    pub speed: f32,
    pub sprite_key: String,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadVehicleWorld {
    pub tick: u64,
    pub vehicles: HashMap<RoadVehicleId, RoadVehicleRecord>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct RoadVehicleDelta {
    pub changed: Vec<RoadVehicleId>,
}

impl RoadVehicleWorld {
    pub fn insert(&mut self, vehicle: RoadVehicleRecord) {
        self.vehicles.insert(vehicle.id.clone(), vehicle);
    }

    pub fn get(&self, id: &RoadVehicleId) -> Option<&RoadVehicleRecord> {
        self.vehicles.get(id)
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn tick_road_vehicles(&mut self) -> RoadVehicleDelta {
        self.tick = self.tick.wrapping_add(1);
        let mut changed = Vec::with_capacity(self.vehicles.len());
        for vehicle in self.vehicles.values_mut() {
            if vehicle.path.len() < 2 {
                continue;
            }
            let len = vehicle.path.len() as f32;
            vehicle.offset = (vehicle.offset + vehicle.speed).rem_euclid(len);
            changed.push(vehicle.id.clone());
        }
        // Deterministic order so deltas are stable across runs.
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        RoadVehicleDelta { changed }
    }

    pub fn world_coord(&self, id: &RoadVehicleId) -> Option<(f32, f32)> {
        let vehicle = self.vehicles.get(id)?;
        let (a, b, t) = interpolate_path(vehicle)?;
        Some((
            a.x as f32 + (b.x - a.x) as f32 * t,
            a.y as f32 + (b.y - a.y) as f32 * t,
        ))
    }

    pub fn direction(&self, id: &RoadVehicleId) -> Option<DirectionDto> {
        let vehicle = self.vehicles.get(id)?;
        let (a, b, _t) = interpolate_path(vehicle)?;
        Some(direction_from_delta((b.x - a.x) as f32, (b.y - a.y) as f32))
    }
}

fn interpolate_path(vehicle: &RoadVehicleRecord) -> Option<(TileCoord, TileCoord, f32)> {
    if vehicle.path.len() < 2 {
        return None;
    }
    let len = vehicle.path.len();
    let base = vehicle.offset.floor() as usize % len;
    let next = (base + 1) % len;
    let t = vehicle.offset - vehicle.offset.floor();
    Some((vehicle.path[base], vehicle.path[next], t))
}

pub fn build_road_vehicle_dto(world: &RoadVehicleWorld, id: &RoadVehicleId) -> Option<RoadVehicleDto> {
    let vehicle = world.vehicles.get(id)?;
    let coord = world.world_coord(id).unwrap_or((0.0, 0.0));
    let direction = world.direction(id).unwrap_or(DirectionDto::S);
    Some(RoadVehicleDto {
        id: vehicle.id.0.clone(),
        world_coord: WorldCoordDto { x: coord.0, y: coord.1 },
        direction,
        sprite_key: vehicle.sprite_key.clone(),
    })
}

pub fn build_road_vehicle_snapshot_dto(
    world_id: &WorldId,
    world: &RoadVehicleWorld,
) -> abutown_protocol::RoadVehicleSnapshotDto {
    let mut ids: Vec<RoadVehicleId> = world.vehicles.keys().cloned().collect();
    ids.sort_by(|a, b| a.0.cmp(&b.0));
    let vehicles = ids
        .iter()
        .filter_map(|id| build_road_vehicle_dto(world, id))
        .collect();
    abutown_protocol::RoadVehicleSnapshotDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick: world.tick,
        vehicles,
    }
}

pub fn build_road_vehicle_delta_dto(
    world_id: &WorldId,
    world: &RoadVehicleWorld,
    delta: &RoadVehicleDelta,
) -> abutown_protocol::RoadVehicleDeltaDto {
    let changed = delta
        .changed
        .iter()
        .filter_map(|id| build_road_vehicle_dto(world, id))
        .collect();
    abutown_protocol::RoadVehicleDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick: world.tick,
        changed,
    }
}

pub mod seed {
    use super::*;

    pub fn initial_road_vehicles() -> RoadVehicleWorld {
        let mut world = RoadVehicleWorld::default();
        // Four hardcoded corridors around the seeded chunks (4,4), (5,4), (4,5).
        // Each chunk is 32 tiles wide, so chunk (cx,cy) spans tiles cx*32..(cx+1)*32.
        let corridors: [Vec<TileCoord>; 4] = [
            // Horizontal across chunks (4,4) and (5,4) along their centers (y=144).
            vec![
                TileCoord { x: 4 * 32 + 4, y: 4 * 32 + 16 },
                TileCoord { x: 5 * 32 + 28, y: 4 * 32 + 16 },
            ],
            // Horizontal returning the other way.
            vec![
                TileCoord { x: 5 * 32 + 28, y: 4 * 32 + 16 },
                TileCoord { x: 4 * 32 + 4, y: 4 * 32 + 16 },
            ],
            // Vertical across chunks (4,4) and (4,5).
            vec![
                TileCoord { x: 4 * 32 + 16, y: 4 * 32 + 4 },
                TileCoord { x: 4 * 32 + 16, y: 5 * 32 + 28 },
            ],
            // Vertical returning.
            vec![
                TileCoord { x: 4 * 32 + 16, y: 5 * 32 + 28 },
                TileCoord { x: 4 * 32 + 16, y: 4 * 32 + 4 },
            ],
        ];

        for index in 0..80u32 {
            let corridor = corridors[(index as usize) % corridors.len()].clone();
            let id = RoadVehicleId(format!("road_vehicle:seed:{index}"));
            world.vehicles.insert(
                id.clone(),
                RoadVehicleRecord {
                    id,
                    offset: (index as f32) * 0.25,
                    speed: 0.05 + (index % 5) as f32 * 0.01,
                    sprite_key: format!("vehicle:{}", index % 8),
                    path: corridor,
                },
            );
        }
        world
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{RoadVehicleId, TileCoord};

    #[test]
    fn tick_advances_offset_by_speed_and_wraps_at_path_end() {
        let mut world = RoadVehicleWorld::default();
        let id = RoadVehicleId("road_vehicle:test:0".to_string());
        world.insert(RoadVehicleRecord {
            id: id.clone(),
            path: vec![
                TileCoord { x: 0, y: 0 },
                TileCoord { x: 4, y: 0 },
                TileCoord { x: 4, y: 4 },
                TileCoord { x: 0, y: 4 },
            ],
            offset: 3.5,
            speed: 1.0,
            sprite_key: "vehicle:0".to_string(),
        });

        world.tick_road_vehicles();
        let stored = world.get(&id).unwrap();
        assert!((stored.offset - 0.5).abs() < 1e-5, "offset wraps past path length");
    }

    #[test]
    fn world_coord_interpolates_between_path_segments() {
        let mut world = RoadVehicleWorld::default();
        let id = RoadVehicleId("road_vehicle:test:0".to_string());
        world.insert(RoadVehicleRecord {
            id: id.clone(),
            path: vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 4, y: 0 }],
            offset: 0.5,
            speed: 1.0,
            sprite_key: "vehicle:0".to_string(),
        });
        let coord = world.world_coord(&id).expect("coord exists");
        assert!((coord.0 - 2.0).abs() < 1e-5);
        assert!((coord.1 - 0.0).abs() < 1e-5);
    }

    #[test]
    fn direction_matches_path_orientation() {
        use abutown_protocol::DirectionDto;
        let mut world = RoadVehicleWorld::default();
        let id = RoadVehicleId("road_vehicle:test:0".to_string());
        world.insert(RoadVehicleRecord {
            id: id.clone(),
            path: vec![TileCoord { x: 0, y: 0 }, TileCoord { x: 0, y: -4 }],
            offset: 0.0,
            speed: 1.0,
            sprite_key: "vehicle:0".to_string(),
        });
        assert_eq!(world.direction(&id).unwrap(), DirectionDto::N);
    }

    #[test]
    fn initial_road_vehicles_seeds_a_useful_population() {
        let world = seed::initial_road_vehicles();
        assert!(world.vehicles.len() >= 80, "seed must populate at least 80 road vehicles");
        for vehicle in world.vehicles.values() {
            assert!(vehicle.path.len() >= 2, "every road vehicle path needs two points");
            assert!(vehicle.speed > 0.0);
            assert!(!vehicle.sprite_key.is_empty());
        }
    }

    #[test]
    fn seed_is_deterministic() {
        let a = seed::initial_road_vehicles();
        let b = seed::initial_road_vehicles();
        assert_eq!(a, b);
    }
}
