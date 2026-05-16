use std::collections::{HashMap, VecDeque};

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

    let mut routes = HashMap::new();
    routes.insert(
        horizontal_route.clone(),
        RouteRecord {
            id: horizontal_route.clone(),
            links: vec![horizontal_link.clone()],
        },
    );
    routes.insert(
        vertical_route.clone(),
        RouteRecord {
            id: vertical_route.clone(),
            links: vec![vertical_link.clone()],
        },
    );

    let mut stops = HashMap::new();
    for (stop_id, route_id, progress) in [
        (&horizontal_pickup, &horizontal_route, 0.0_f32),
        (&horizontal_dropoff, &horizontal_route, 1.0_f32),
        (&vertical_pickup, &vertical_route, 0.0_f32),
        (&vertical_dropoff, &vertical_route, 1.0_f32),
    ] {
        stops.insert(
            stop_id.clone(),
            StopRecord {
                id: stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress,
                waiting_agents: VecDeque::new(),
            },
        );
    }

    let mut vehicles = HashMap::new();
    for offset in 0..4u32 {
        let route_id = if offset % 2 == 0 {
            horizontal_route.clone()
        } else {
            vertical_route.clone()
        };
        let vehicle_id = VehicleId(format!("vehicle:seed:{offset}"));
        vehicles.insert(
            vehicle_id.clone(),
            VehicleRecord {
                id: vehicle_id,
                kind: VehicleKind::Tram,
                route_id,
                link_index: 0,
                progress: (offset as f32) * 0.25,
                speed_per_tick: 0.1,
                capacity: 4,
                occupants: Vec::new(),
                dwell_ticks_remaining: 0,
            },
        );
    }

    let mut agents = HashMap::new();
    for offset in 0..20u32 {
        let agent_id = AgentId(format!("agent:seed:{offset}"));
        let (pickup, dropoff, route_id) = if offset % 2 == 0 {
            (&horizontal_pickup, &horizontal_dropoff, &horizontal_route)
        } else {
            (&vertical_pickup, &vertical_dropoff, &vertical_route)
        };

        agents.insert(
            agent_id.clone(),
            AgentRecord {
                id: agent_id,
                state: AgentMobilityState::Walking {
                    link_id: walk_link.clone(),
                    progress: (offset as f32) * 0.05,
                },
                plan: vec![
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
                plan_cursor: 0,
                walk_speed_per_tick: 0.5,
            },
        );
    }

    MobilityWorld {
        tick: 0,
        agents,
        vehicles,
        stops,
        routes,
        link_polylines: HashMap::new(),
    }
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

    let mut world = MobilityWorld::default();
    let mut routes: HashMap<RouteId, RouteRecord> = HashMap::new();
    let mut links: Vec<(LinkId, Vec<(f32, f32)>)> = Vec::new();

    // Register pedestrian corridors as walking links.
    for (index, corridor) in network.pedestrian_corridors.iter().enumerate() {
        let link_id = LinkId(format!("link:walk:corridor:{index}"));
        let points: Vec<(f32, f32)> = corridor
            .iter()
            .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
            .collect();
        links.push((link_id, points));
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
            world.agents.insert(
                agent_id.clone(),
                AgentRecord {
                    id: agent_id,
                    state: AgentMobilityState::Walking { link_id, progress },
                    plan: vec![PlanStage::Activity {
                        activity_id: format!("activity:wander:{corridor_index}"),
                    }],
                    plan_cursor: 0,
                    walk_speed_per_tick: 0.05,
                },
            );
        }
    }

    // Register arterial paths as routes for cars (one route per arterial, one link).
    for (index, arterial) in network.arterial_paths.iter().enumerate() {
        let route_id = RouteId(format!("route:arterial:{index}"));
        let link_id = LinkId(format!("link:arterial:{index}"));
        let points: Vec<(f32, f32)> = arterial
            .iter()
            .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
            .collect();
        links.push((link_id.clone(), points));
        routes.insert(
            route_id.clone(),
            RouteRecord {
                id: route_id.clone(),
                links: vec![link_id],
            },
        );
    }

    // Spawn cars + drivers.
    let mut driver_index: u32 = 0;
    for (arterial_index, _arterial) in network.arterial_paths.iter().enumerate() {
        for n in 0..density.cars_per_arterial {
            let vehicle_id = VehicleId(format!("vehicle:car:{arterial_index}:{n}"));
            let route_id = RouteId(format!("route:arterial:{arterial_index}"));
            let driver_id = AgentId(format!("agent:driver:{driver_index}"));
            driver_index += 1;
            world.vehicles.insert(
                vehicle_id.clone(),
                VehicleRecord {
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
                },
            );
            world.agents.insert(
                driver_id.clone(),
                AgentRecord {
                    id: driver_id,
                    state: AgentMobilityState::InVehicle {
                        vehicle_id,
                        seat_index: 0,
                    },
                    plan: vec![PlanStage::Activity {
                        activity_id: format!("activity:drive:{arterial_index}"),
                    }],
                    plan_cursor: 0,
                    walk_speed_per_tick: 0.05,
                },
            );
        }
    }

    // Trams: reuse the existing tiny_world tram vehicles, routes, and stops.
    // We do NOT copy the tiny_world pedestrian agents — they belong to
    // the tiny seeded world only; this world seeds its own walkers above.
    if density.trams_total > 0 {
        let tram_seed = tiny_world();
        for vehicle in tram_seed.vehicles.values() {
            world.vehicles.insert(vehicle.id.clone(), vehicle.clone());
        }
        for (id, record) in &tram_seed.routes {
            routes.insert(id.clone(), record.clone());
        }
        for stop in tram_seed.stops.values() {
            world.stops.insert(stop.id.clone(), stop.clone());
        }
    }

    world.routes = routes;
    world.link_polylines = links.into_iter().collect();
    world
}
