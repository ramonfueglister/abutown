use bevy_ecs::schedule::Schedule;
use bevy_ecs::world::World;

use super::*;
use crate::city_network::CityNetwork;
use crate::ids::{AgentId, VehicleId};

/// Pedestrian walks generated from the authored base-world network.
pub fn seeded_walks_from_network(network: &CityNetwork) -> Vec<crate::routing::SeededWalk> {
    use crate::city_network::NetworkCoord;
    let mut out: Vec<crate::routing::SeededWalk> = Vec::new();

    for (index, corridor) in network.pedestrian_corridors.iter().enumerate() {
        let polyline: Vec<(f32, f32)> = corridor
            .iter()
            .map(|NetworkCoord { x, y }| (*x as f32, *y as f32))
            .collect();
        if polyline.len() < 2 {
            continue;
        }
        out.push(crate::routing::SeededWalk {
            legacy_link_id: format!("link:walk:corridor:{index}"),
            polyline,
        });
    }

    out
}

#[cfg(test)]
fn test_seeded_walks(network: &CityNetwork) -> Vec<crate::routing::SeededWalk> {
    let mut out = seeded_walks_from_network(network);
    let c44 = (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0);
    let c54 = (5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0);
    out.push(crate::routing::SeededWalk {
        legacy_link_id: "link:walk:default".into(),
        polyline: vec![c44, c54],
    });

    out
}

#[cfg(test)]
fn test_seeded_stops() -> Vec<crate::routing::SeededStop> {
    // Tile-space centre coords for the seeded chunks:
    //   c44 = (4 * 32 + 16, 4 * 32 + 16) = (144.0, 144.0)
    //   c54 = (5 * 32 + 16, 4 * 32 + 16) = (176.0, 144.0)
    //   c45 = (4 * 32 + 16, 5 * 32 + 16) = (144.0, 176.0)
    //
    // horizontal route: c44 → c54  (progress 0.0 → 1.0)
    // vertical route:   c44 → c45  (progress 0.0 → 1.0)
    vec![
        crate::routing::SeededStop {
            legacy_stop_id: "stop:horizontal:pickup".into(),
            coord: (144.0, 144.0),
            legacy_route_id: "route:horizontal".into(),
        },
        crate::routing::SeededStop {
            legacy_stop_id: "stop:horizontal:dropoff".into(),
            coord: (176.0, 144.0),
            legacy_route_id: "route:horizontal".into(),
        },
        crate::routing::SeededStop {
            legacy_stop_id: "stop:vertical:pickup".into(),
            coord: (144.0, 144.0),
            legacy_route_id: "route:vertical".into(),
        },
        crate::routing::SeededStop {
            legacy_stop_id: "stop:vertical:dropoff".into(),
            coord: (144.0, 176.0),
            legacy_route_id: "route:vertical".into(),
        },
    ]
}

#[cfg(test)]
fn tiny_city_network() -> CityNetwork {
    use crate::city_network::{NetworkCoord, WorldTiles};
    let c44 = NetworkCoord { x: 144, y: 144 };
    let c54 = NetworkCoord { x: 176, y: 144 };
    let c45 = NetworkCoord { x: 144, y: 176 };
    CityNetwork {
        version: 1,
        world_id: "abutown-tiny".into(),
        chunk_size: 32,
        world_tiles: WorldTiles {
            width: 256,
            height: 256,
        },
        arterial_paths: vec![vec![c44, c54], vec![c44, c45]],
        pedestrian_corridors: Vec::new(),
    }
}

fn empty_world_and_schedule_for_network(network: &CityNetwork) -> (World, Schedule) {
    let mut world = World::new();
    let mut schedule = Schedule::default();
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::time::TimePlugin.install(&mut world, &mut schedule);
    world.insert_resource(network.clone());
    crate::routing::RoutingPlugin {
        seeded_stops: Vec::new(),
        seeded_walks: seeded_walks_from_network(network),
    }
    .install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    (world, schedule)
}

#[cfg(test)]
fn test_world_and_schedule_for_network(network: &CityNetwork) -> (World, Schedule) {
    let mut world = World::new();
    let mut schedule = Schedule::default();
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    CorePlugin::default().install(&mut world, &mut schedule);
    crate::time::TimePlugin.install(&mut world, &mut schedule);
    world.insert_resource(network.clone());
    crate::routing::RoutingPlugin {
        seeded_stops: test_seeded_stops(),
        seeded_walks: test_seeded_walks(network),
    }
    .install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);
    (world, schedule)
}

/// Build a deterministic populated mobility world for unit tests.
///
/// Two routes traverse the seeded chunk neighbourhood; 4 vehicles and
/// 20 agents are spawned with cyclic plans. Calling this function twice
/// returns equal worlds (by `extract_from_world`).
#[cfg(test)]
pub fn test_seed_world() -> (World, Schedule) {
    let horizontal_route = "route:arterial:0".to_string();
    let vertical_route = "route:arterial:1".to_string();

    let horizontal_pickup = "stop:horizontal:pickup".to_string();
    let horizontal_dropoff = "stop:horizontal:dropoff".to_string();
    let vertical_pickup = "stop:vertical:pickup".to_string();
    let vertical_dropoff = "stop:vertical:dropoff".to_string();

    let walk_link = "link:walk:default".to_string();
    let work_activity = "activity:work".to_string();

    let network = tiny_city_network();
    let (mut world, schedule) = test_world_and_schedule_for_network(&network);

    for offset in 0..4u32 {
        let route_id = if offset % 2 == 0 {
            horizontal_route.clone()
        } else {
            vertical_route.clone()
        };
        let vehicle_id = VehicleId(format!("vehicle:seed:{offset}"));
        api::spawn_vehicle_from_record(
            &mut world,
            VehicleRecord {
                id: vehicle_id,
                kind: VehicleKind::Car,
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

    for offset in 0..20u32 {
        let agent_id = AgentId(format!("agent:seed:{offset}"));
        let (pickup, dropoff, route_id) = if offset % 2 == 0 {
            (&horizontal_pickup, &horizontal_dropoff, &horizontal_route)
        } else {
            (&vertical_pickup, &vertical_dropoff, &vertical_route)
        };

        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
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
            ),
        );
    }

    (world, schedule)
}

#[derive(Debug, Clone, Copy)]
pub struct SeedDensity {
    pub pedestrians_per_corridor: u32,
    pub cars_per_arterial: u32,
}

impl Default for SeedDensity {
    fn default() -> Self {
        Self {
            pedestrians_per_corridor: 6,
            cars_per_arterial: 4,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("base world pedestrian group {group_id} references missing corridor {corridor_id}")]
    MissingPedestrianCorridor {
        group_id: String,
        corridor_id: String,
    },
    #[error("base world car group {group_id} references missing arterial path {arterial_id}")]
    MissingArterialPath {
        group_id: String,
        arterial_id: String,
    },
}

pub fn from_network(network: &CityNetwork, density: SeedDensity) -> (World, Schedule) {
    let (mut world, schedule) = empty_world_and_schedule_for_network(network);

    // Spawn walking agents distributed across corridors.
    if !network.pedestrian_corridors.is_empty() {
        let pedestrian_count =
            network.pedestrian_corridors.len() as u32 * density.pedestrians_per_corridor;
        for n in 0..pedestrian_count {
            let corridor_index = (n as usize) % network.pedestrian_corridors.len();
            let agent_id = AgentId(format!("agent:walk:{n}"));
            let link_id = format!("link:walk:corridor:{corridor_index}");
            let progress = ((n as f32) / (density.pedestrians_per_corridor as f32)).fract();
            api::spawn_agent_from_record(
                &mut world,
                AgentRecord::new(
                    agent_id,
                    AgentMobilityState::Walking { link_id, progress },
                    vec![PlanStage::Activity {
                        activity_id: format!("activity:wander:{corridor_index}"),
                    }],
                    0.05,
                ),
            );
        }
    }

    // Spawn cars + drivers.
    let mut driver_index: u32 = 0;
    for (arterial_index, _arterial) in network.arterial_paths.iter().enumerate() {
        for n in 0..density.cars_per_arterial {
            let vehicle_id = VehicleId(format!("vehicle:car:{arterial_index}:{n}"));
            let route_id = format!("route:arterial:{arterial_index}");
            let driver_id = AgentId(format!("agent:driver:{driver_index}"));
            driver_index += 1;
            api::spawn_vehicle_from_record(
                &mut world,
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
            api::spawn_agent_from_record(
                &mut world,
                AgentRecord::new(
                    driver_id,
                    AgentMobilityState::InVehicle {
                        vehicle_id,
                        seat_index: 0,
                    },
                    vec![PlanStage::Activity {
                        activity_id: format!("activity:drive:{arterial_index}"),
                    }],
                    0.05,
                ),
            );
        }
    }

    (world, schedule)
}

pub fn from_base_world_bundle(
    bundle: &crate::base_world::BaseWorldBundle,
) -> Result<(World, Schedule), SeedError> {
    let network = bundle.to_city_network();
    let mut world = World::new();
    let mut schedule = Schedule::default();
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    CorePlugin::default().install(&mut world, &mut schedule);
    world.insert_resource(network);
    crate::routing::RoutingPlugin {
        seeded_stops: Vec::new(),
        seeded_walks: seeded_walks_from_network(&bundle.to_city_network()),
    }
    .install(&mut world, &mut schedule);
    crate::mobility::MobilityPlugin.install(&mut world, &mut schedule);

    seed_pedestrians_from_bundle(&mut world, bundle)?;
    seed_cars_from_bundle(&mut world, bundle)?;

    Ok((world, schedule))
}

fn seed_pedestrians_from_bundle(
    world: &mut World,
    bundle: &crate::base_world::BaseWorldBundle,
) -> Result<(), SeedError> {
    let mut agent_index = 0u32;
    for group in &bundle.spawns.pedestrian_groups {
        let Some(corridor_index) = bundle
            .transport
            .pedestrian_corridors
            .iter()
            .position(|path| path.id == group.corridor_id)
        else {
            return Err(SeedError::MissingPedestrianCorridor {
                group_id: group.id.clone(),
                corridor_id: group.corridor_id.clone(),
            });
        };
        for n in 0..group.agents_per_corridor {
            let agent_id = AgentId(format!("agent:walk:{agent_index}"));
            agent_index += 1;
            let link_id = format!("link:walk:corridor:{corridor_index}");
            let progress = if group.agents_per_corridor > 0 {
                (n as f32) / (group.agents_per_corridor as f32)
            } else {
                0.0
            };
            api::spawn_agent_from_record(
                world,
                AgentRecord::new(
                    agent_id,
                    AgentMobilityState::Walking { link_id, progress },
                    vec![PlanStage::Activity {
                        activity_id: format!("activity:wander:{corridor_index}"),
                    }],
                    0.05,
                ),
            );
        }
    }
    Ok(())
}

fn seed_cars_from_bundle(
    world: &mut World,
    bundle: &crate::base_world::BaseWorldBundle,
) -> Result<(), SeedError> {
    let mut driver_index = 0u32;
    for group in &bundle.spawns.car_groups {
        let Some(arterial_index) = bundle
            .transport
            .arterial_paths
            .iter()
            .position(|path| path.id == group.arterial_id)
        else {
            return Err(SeedError::MissingArterialPath {
                group_id: group.id.clone(),
                arterial_id: group.arterial_id.clone(),
            });
        };
        for n in 0..group.cars_per_arterial {
            let vehicle_id = VehicleId(format!("vehicle:car:{arterial_index}:{n}"));
            let route_id = format!("route:arterial:{arterial_index}");
            let driver_id = AgentId(format!("agent:driver:{driver_index}"));
            driver_index += 1;
            api::spawn_vehicle_from_record(
                world,
                VehicleRecord {
                    id: vehicle_id.clone(),
                    kind: VehicleKind::Car,
                    route_id,
                    link_index: 0,
                    progress: if group.cars_per_arterial > 0 {
                        (n as f32) / (group.cars_per_arterial as f32)
                    } else {
                        0.0
                    },
                    speed_per_tick: 0.02,
                    capacity: 1,
                    occupants: vec![driver_id.clone()],
                    dwell_ticks_remaining: 0,
                },
            );
            api::spawn_agent_from_record(
                world,
                AgentRecord::new(
                    driver_id,
                    AgentMobilityState::InVehicle {
                        vehicle_id,
                        seat_index: 0,
                    },
                    vec![PlanStage::Activity {
                        activity_id: format!("activity:drive:{arterial_index}"),
                    }],
                    0.05,
                ),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("sim-core crate lives under backend/crates")
            .to_path_buf()
    }

    #[test]
    fn from_base_world_bundle_seeds_no_trams() {
        let bundle = crate::base_world::BaseWorldBundle::load_from_dir(
            workspace_root().join("data/worlds/abutopia"),
        )
        .expect("base world bundle should load");

        let (world, _) = from_base_world_bundle(&bundle).expect("base world should seed");
        let agents = crate::mobility::api::agents(&world);
        let vehicles = crate::mobility::api::vehicles(&world);

        assert_eq!(agents.len(), 1);
        assert!(vehicles.is_empty());
        let tram_prefix = ["vehicle:", "tram:"].concat();
        assert!(
            vehicles
                .iter()
                .all(|vehicle| !vehicle.id.0.starts_with(&tram_prefix))
        );
    }
}
