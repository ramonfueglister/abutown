use std::collections::VecDeque;

use super::*;
use crate::city_network::CityNetwork;
use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};

/// Backward-compatible wrapper — delegates to [`tiny_world`].
pub fn initial_world() -> MobilityWorld {
    tiny_world()
}

/// Build a deterministic populated mobility world for fresh server starts.
///
/// Two routes traverse the seeded chunk neighbourhood; 4 vehicles and
/// 20 agents are spawned with cyclic plans. Calling this function twice
/// returns equal worlds.
pub fn tiny_world() -> MobilityWorld {
    let horizontal_route = RouteId("route:horizontal".to_string());
    let vertical_route = RouteId("route:vertical".to_string());
    let horizontal_link = LinkId("link:horizontal:main".to_string());
    let vertical_link = LinkId("link:vertical:main".to_string());

    let horizontal_pickup = StopId("stop:horizontal:pickup".to_string());
    let horizontal_dropoff = StopId("stop:horizontal:dropoff".to_string());
    let vertical_pickup = StopId("stop:vertical:pickup".to_string());
    let vertical_dropoff = StopId("stop:vertical:dropoff".to_string());

    let walk_link = LinkId("link:walk:default".to_string());
    let work_activity = "activity:work".to_string();

    let mut world = MobilityWorld::empty();

    // Register the three seeded polylines so spawn-time / runtime coord
    // resolution can look them up directly without a hardcoded fallback.
    // Coords use chunk-centre points for chunks (4,4), (5,4), (4,5) where
    // chunk_size = 32 → centre = chunk * 32 + 16.
    let c44 = (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0);
    let c54 = (5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0);
    let c45 = (4.0 * 32.0 + 16.0, 5.0 * 32.0 + 16.0);
    world.set_link_polyline(horizontal_link.clone(), vec![c44, c54]);
    world.set_link_polyline(vertical_link.clone(), vec![c44, c45]);
    world.set_link_polyline(walk_link.clone(), vec![c44, c54]);

    world.add_route(RouteRecord {
        id: horizontal_route.clone(),
        links: vec![horizontal_link.clone()],
    });
    world.add_route(RouteRecord {
        id: vertical_route.clone(),
        links: vec![vertical_link.clone()],
    });

    for (stop_id, route_id, progress) in [
        (&horizontal_pickup, &horizontal_route, 0.0_f32),
        (&horizontal_dropoff, &horizontal_route, 1.0_f32),
        (&vertical_pickup, &vertical_route, 0.0_f32),
        (&vertical_dropoff, &vertical_route, 1.0_f32),
    ] {
        world.add_stop(StopRecord {
            id: stop_id.clone(),
            route_id: route_id.clone(),
            link_index: 0,
            progress,
            waiting_agents: VecDeque::new(),
        });
    }

    for offset in 0..4u32 {
        let route_id = if offset % 2 == 0 {
            horizontal_route.clone()
        } else {
            vertical_route.clone()
        };
        let vehicle_id = VehicleId(format!("vehicle:seed:{offset}"));
        world.spawn_vehicle_from_record(VehicleRecord {
            id: vehicle_id,
            kind: VehicleKind::Tram,
            route_id,
            link_index: 0,
            progress: (offset as f32) * 0.25,
            speed_per_tick: 0.1,
            capacity: 4,
            occupants: Vec::new(),
            dwell_ticks_remaining: 0,
        });
    }

    for offset in 0..20u32 {
        let agent_id = AgentId(format!("agent:seed:{offset}"));
        let (pickup, dropoff, route_id) = if offset % 2 == 0 {
            (&horizontal_pickup, &horizontal_dropoff, &horizontal_route)
        } else {
            (&vertical_pickup, &vertical_dropoff, &vertical_route)
        };

        world.spawn_agent_from_record(AgentRecord::new(
            agent_id,
            AgentMobilityState::Walking {
                link_id: walk_link.clone(),
                progress: (offset as f32) * 0.05,
            },
            vec![
                PlanStage::WalkToStop {
                    link_id: walk_link.clone(),
                    stop_id: pickup.clone(),
                },
                PlanStage::RideToStop {
                    route_id: route_id.clone(),
                    stop_id: dropoff.clone(),
                },
                PlanStage::WalkToActivity {
                    link_id: walk_link.clone(),
                    activity_id: work_activity.clone(),
                },
                PlanStage::Activity {
                    activity_id: work_activity.clone(),
                },
            ],
            0.5,
        ));
    }

    world
}

#[derive(Debug, Clone, Copy)]
pub struct SeedDensity {
    pub pedestrians_per_corridor: u32,
    pub cars_per_arterial: u32,
    pub trams_total: u32,
}

impl Default for SeedDensity {
    fn default() -> Self {
        Self {
            pedestrians_per_corridor: 6,
            cars_per_arterial: 4,
            trams_total: 4,
        }
    }
}

pub fn from_network(network: &CityNetwork, density: SeedDensity) -> MobilityWorld {
    use crate::city_network::NetworkCoord;

    let mut world = MobilityWorld::empty();

    // Register pedestrian corridors as walking links BEFORE spawning any
    // agents on them — spawn-time position resolution requires the polyline
    // to be in `LinkPolylines` already.
    for (index, corridor) in network.pedestrian_corridors.iter().enumerate() {
        let link_id = LinkId(format!("link:walk:corridor:{index}"));
        let points: Vec<(f32, f32)> = corridor
            .iter()
            .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
            .collect();
        world.set_link_polyline(link_id, points);
    }

    // Register arterial paths as routes + polylines BEFORE spawning cars.
    for (index, arterial) in network.arterial_paths.iter().enumerate() {
        let route_id = RouteId(format!("route:arterial:{index}"));
        let link_id = LinkId(format!("link:arterial:{index}"));
        let points: Vec<(f32, f32)> = arterial
            .iter()
            .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
            .collect();
        world.set_link_polyline(link_id.clone(), points);
        world.add_route(RouteRecord {
            id: route_id.clone(),
            links: vec![link_id],
        });
    }

    // Trams: reuse the existing tiny_world tram polylines, routes, vehicles,
    // and stops. Polylines copied first so subsequent tram-vehicle spawns can
    // resolve their position. We do NOT copy tiny_world's pedestrian agents —
    // those belong to the tiny seed only; this world seeds its own walkers below.
    if density.trams_total > 0 {
        let tram_seed = tiny_world();
        for (id, points) in tram_seed.link_polylines_iter() {
            world.set_link_polyline(id.clone(), points.clone());
        }
        for (id, record) in tram_seed.routes() {
            world.add_route(RouteRecord {
                id: id.clone(),
                links: record.links.clone(),
            });
        }
        for stop in tram_seed.stops() {
            world.add_stop(stop);
        }
        for vehicle in tram_seed.vehicles() {
            world.spawn_vehicle_from_record(vehicle);
        }
    }

    // Spawn walking agents distributed across corridors.
    if !network.pedestrian_corridors.is_empty() {
        let pedestrian_count =
            network.pedestrian_corridors.len() as u32 * density.pedestrians_per_corridor;
        for n in 0..pedestrian_count {
            let corridor_index = (n as usize) % network.pedestrian_corridors.len();
            let agent_id = AgentId(format!("agent:walk:{n}"));
            let link_id = LinkId(format!("link:walk:corridor:{corridor_index}"));
            let progress = ((n as f32) / (density.pedestrians_per_corridor as f32)).fract();
            world.spawn_agent_from_record(AgentRecord::new(
                agent_id,
                AgentMobilityState::Walking { link_id, progress },
                vec![PlanStage::Activity {
                    activity_id: format!("activity:wander:{corridor_index}"),
                }],
                0.05,
            ));
        }
    }

    // Spawn cars + drivers.
    let mut driver_index: u32 = 0;
    for (arterial_index, _arterial) in network.arterial_paths.iter().enumerate() {
        for n in 0..density.cars_per_arterial {
            let vehicle_id = VehicleId(format!("vehicle:car:{arterial_index}:{n}"));
            let route_id = RouteId(format!("route:arterial:{arterial_index}"));
            let driver_id = AgentId(format!("agent:driver:{driver_index}"));
            driver_index += 1;
            world.spawn_vehicle_from_record(VehicleRecord {
                id: vehicle_id.clone(),
                kind: VehicleKind::Car,
                route_id,
                link_index: 0,
                progress: if density.cars_per_arterial > 0 {
                    (n as f32) / (density.cars_per_arterial as f32)
                } else {
                    0.0
                },
                speed_per_tick: 0.02,
                capacity: 1,
                occupants: vec![driver_id.clone()],
                dwell_ticks_remaining: 0,
            });
            world.spawn_agent_from_record(AgentRecord::new(
                driver_id,
                AgentMobilityState::InVehicle {
                    vehicle_id,
                    seat_index: 0,
                },
                vec![PlanStage::Activity {
                    activity_id: format!("activity:drive:{arterial_index}"),
                }],
                0.05,
            ));
        }
    }

    world
}
