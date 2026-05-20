mod records;
pub use records::*;

mod dto;
pub use dto::*;

pub mod api;
pub mod components;
pub mod lod;
pub mod persist_snapshot;
pub mod resources;
pub mod seed;
pub mod systems;

pub use api::install_mobility;
pub use persist_snapshot::{MobilityPersistSnapshot, apply_into_world, extract_from_world};

pub fn chunk_of(x: f32, y: f32, chunk_size: u16) -> crate::ids::ChunkCoord {
    let cs = chunk_size as f32;
    crate::ids::ChunkCoord {
        x: x.div_euclid(cs) as i32,
        y: y.div_euclid(cs) as i32,
    }
}

/// World coord for an agent given its mobility state. Returns `None` for
/// states where there is no unambiguous spawn-time coord (`InVehicle`,
/// `AtActivity`) or when the referenced link/stop is not registered.
///
/// Used by both `compute_world_coord_system` (per-tick) and
/// `spawn_agent_from_record` (one-shot at spawn time) so LOD systems see
/// the real position immediately on Tick 1 instead of the default `(0,0)`.
pub fn agent_world_coord(
    state: &AgentMobilityState,
    routes: &resources::Routes,
    stops: &resources::Stops,
    link_polylines: &resources::LinkPolylines,
) -> Option<(f32, f32)> {
    match state {
        AgentMobilityState::Walking { link_id, progress } => {
            let points = link_polylines.0.get(link_id)?;
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                points, *progress,
            ))
        }
        AgentMobilityState::WaitingAtStop { stop_id }
        | AgentMobilityState::Boarding { stop_id, .. }
        | AgentMobilityState::Alighting { stop_id, .. } => {
            let stop = stops.0.get(stop_id)?;
            let route = routes.0.get(&stop.route_id)?;
            let link_id = route.links.get(stop.link_index)?;
            let points = link_polylines.0.get(link_id)?;
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                points,
                stop.progress,
            ))
        }
        _ => None,
    }
}

/// World coord for a vehicle given its route position. Returns `None` if
/// the route or link is not registered, or its polyline missing.
pub fn vehicle_world_coord(
    route_position: &components::RoutePosition,
    routes: &resources::Routes,
    link_polylines: &resources::LinkPolylines,
) -> Option<(f32, f32)> {
    let route = routes.0.get(&route_position.route_id)?;
    let link_id = route.links.get(route_position.link_index)?;
    let points = link_polylines.0.get(link_id)?;
    Some(crate::mobility_geometry::world_coord_at_progress_slice(
        points,
        route_position.progress,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, ChunkCoord, LinkId, RouteId, StopId, VehicleId};
    use crate::mobility::resources::{AgentIdIndex, VehicleIdIndex};
    use abutown_protocol::WorldId;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::world::World;
    use std::collections::HashMap;
    use std::collections::VecDeque;

    fn empty_world() -> (World, Schedule) {
        api::empty_world_and_schedule()
    }

    #[test]
    fn initial_world_seeds_expected_population() {
        let (world, _) = seed::initial_world();

        assert_eq!(api::tick(&world), 0);
        assert_eq!(api::routes(&world).len(), 2, "expected 2 routes");

        let snapshot = api::snapshot(&world);
        assert_eq!(snapshot.stops.len(), 4, "expected 4 stops");
        assert_eq!(snapshot.vehicles.len(), 4, "expected 4 vehicles");
        assert_eq!(snapshot.agents.len(), 20, "expected 20 agents");

        for agent in &snapshot.agents {
            assert!(
                !agent.plan.is_empty(),
                "every agent must have at least one plan stage"
            );
        }
        for vehicle in &snapshot.vehicles {
            assert!(vehicle.capacity > 0, "vehicle capacity must be positive");
        }
    }

    #[test]
    fn initial_world_is_deterministic() {
        let (a_world, _) = seed::initial_world();
        let (b_world, _) = seed::initial_world();
        let a = extract_from_world(&a_world);
        let b = extract_from_world(&b_world);
        assert_eq!(a, b, "initial_world() must be deterministic across calls");
    }

    #[test]
    fn sample_world_starts_with_agent_walking_to_pickup_stop() {
        let (world, _) = sample_world();
        let agent = api::agent(&world, &AgentId("agent:pedestrian:0".to_string()))
            .expect("sample agent exists");
        let vehicle = api::vehicle(&world, &VehicleId("vehicle:shuttle:0".to_string()))
            .expect("sample vehicle exists");
        let stop = api::stop(&world, &StopId("stop:old-town".to_string()))
            .expect("sample stop exists");

        assert_eq!(agent.plan_cursor, 0);
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:home-to-old-town-stop".to_string()),
                progress: 0.0
            }
        );
        assert_eq!(vehicle.route_id, RouteId("route:old-town-loop".to_string()));
        assert_eq!(vehicle.capacity, 4);
        assert_eq!(stop.route_id, RouteId("route:old-town-loop".to_string()));
    }

    #[test]
    fn walking_agent_reaches_pickup_stop_and_waits() {
        let (mut world, mut schedule) = sample_world();
        api::force_all_chunks_active_for_test(&mut world);
        let agent_id = AgentId("agent:pedestrian:0".to_string());

        let first_map = api::tick_mobility(&mut world, &mut schedule);
        let agent = api::agent(&world, &agent_id).expect("agent exists");
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:home-to-old-town-stop".to_string()),
                progress: 0.5
            }
        );
        assert_eq!(
            first_map
                .values()
                .flat_map(|d| d.changed_agents.iter())
                .count(),
            1
        );

        let second_map = api::tick_mobility(&mut world, &mut schedule);
        let agent = api::agent(&world, &agent_id).expect("agent exists");
        let stop = api::stop(&world, &StopId("stop:old-town".to_string()))
            .expect("pickup stop exists");

        assert_eq!(
            agent.state,
            AgentMobilityState::WaitingAtStop {
                stop_id: StopId("stop:old-town".to_string())
            }
        );
        assert_eq!(agent.plan_cursor, 1);
        assert_eq!(
            stop.waiting_agents.iter().cloned().collect::<Vec<_>>(),
            vec![agent_id]
        );
        assert_eq!(
            second_map
                .values()
                .flat_map(|d| d.changed_agents.iter())
                .count(),
            1
        );
    }

    #[test]
    fn vehicle_respects_initial_dwell_then_moves_on_route() {
        let (mut world, mut schedule) = sample_world();
        api::force_all_chunks_active_for_test(&mut world);
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        let first_map = api::tick_mobility(&mut world, &mut schedule);
        let vehicle = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.progress, 0.0);
        assert_eq!(vehicle.dwell_ticks_remaining, 1);
        assert_eq!(
            first_map
                .values()
                .flat_map(|d| d.changed_vehicles.iter())
                .count(),
            1
        );

        let second_map = api::tick_mobility(&mut world, &mut schedule);
        let vehicle = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.progress, 0.0);
        assert_eq!(vehicle.dwell_ticks_remaining, 0);
        assert_eq!(
            second_map
                .values()
                .flat_map(|d| d.changed_vehicles.iter())
                .count(),
            1
        );

        let third_map = api::tick_mobility(&mut world, &mut schedule);
        let vehicle = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.progress, 0.5);
        assert_eq!(vehicle.dwell_ticks_remaining, 0);
        assert_eq!(
            third_map
                .values()
                .flat_map(|d| d.changed_vehicles.iter())
                .count(),
            1
        );
    }

    #[test]
    fn agent_boards_rides_alights_and_walks_to_activity() {
        let (mut world, mut schedule) = sample_world();
        api::force_all_chunks_active_for_test(&mut world);
        let agent_id = AgentId("agent:pedestrian:0".to_string());
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        api::tick_mobility(&mut world, &mut schedule);
        api::tick_mobility(&mut world, &mut schedule);

        let waiting = api::agent(&world, &agent_id).expect("agent exists");
        assert_eq!(
            waiting.state,
            AgentMobilityState::WaitingAtStop {
                stop_id: StopId("stop:old-town".to_string())
            }
        );

        api::tick_mobility(&mut world, &mut schedule);
        let boarded = api::agent(&world, &agent_id).expect("agent exists");
        let vehicle = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(
            boarded.state,
            AgentMobilityState::InVehicle {
                vehicle_id: vehicle_id.clone(),
                seat_index: 0
            }
        );
        assert_eq!(vehicle.occupants, vec![agent_id.clone()]);

        api::tick_mobility(&mut world, &mut schedule);
        let riding = api::agent(&world, &agent_id).expect("agent exists");
        assert!(matches!(riding.state, AgentMobilityState::InVehicle { .. }));

        api::tick_mobility(&mut world, &mut schedule);
        let alighted = api::agent(&world, &agent_id).expect("agent exists");
        let vehicle = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.occupants, Vec::<AgentId>::new());
        assert_eq!(
            alighted.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:station-to-work".to_string()),
                progress: 0.0
            }
        );
        assert_eq!(alighted.plan_cursor, 2);

        api::tick_mobility(&mut world, &mut schedule);
        api::tick_mobility(&mut world, &mut schedule);
        let arrived = api::agent(&world, &agent_id).expect("agent exists");
        assert_eq!(
            arrived.state,
            AgentMobilityState::AtActivity {
                activity_id: "activity:work".to_string()
            }
        );
        assert_eq!(arrived.plan_cursor, 3);
    }

    #[test]
    fn mobility_world_serde_round_trip_preserves_state() {
        let (world, _) = sample_world();
        let snap = extract_from_world(&world);
        let json = serde_json::to_value(&snap).expect("serialize");
        let back: MobilityPersistSnapshot =
            serde_json::from_value(json.clone()).expect("deserialize");
        let rejson = serde_json::to_value(&back).expect("re-serialize");
        assert_eq!(json, rejson, "round-trip should preserve state");
    }

    fn sample_world() -> (World, Schedule) {
        let route_id = RouteId("route:old-town-loop".to_string());
        let pickup_stop_id = StopId("stop:old-town".to_string());
        let dropoff_stop_id = StopId("stop:station".to_string());
        let walk_to_pickup = LinkId("link:home-to-old-town-stop".to_string());
        let vehicle_link = LinkId("link:old-town-to-station".to_string());
        let walk_to_activity = LinkId("link:station-to-work".to_string());
        let agent_id = AgentId("agent:pedestrian:0".to_string());
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        let mut routes = HashMap::new();
        routes.insert(
            route_id.clone(),
            RouteRecord {
                id: route_id.clone(),
                links: vec![vehicle_link],
            },
        );

        let mut stops = HashMap::new();
        stops.insert(
            pickup_stop_id.clone(),
            StopRecord {
                id: pickup_stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 0.0,
                waiting_agents: VecDeque::new(),
            },
        );
        stops.insert(
            dropoff_stop_id.clone(),
            StopRecord {
                id: dropoff_stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 1.0,
                waiting_agents: VecDeque::new(),
            },
        );

        let mut agents = HashMap::new();
        agents.insert(
            agent_id.clone(),
            AgentRecord::new(
                agent_id,
                AgentMobilityState::Walking {
                    link_id: walk_to_pickup.clone(),
                    progress: 0.0,
                },
                vec![
                    PlanStage::WalkToStop {
                        link_id: walk_to_pickup,
                        stop_id: pickup_stop_id,
                    },
                    PlanStage::RideToStop {
                        route_id: route_id.clone(),
                        stop_id: dropoff_stop_id,
                    },
                    PlanStage::WalkToActivity {
                        link_id: walk_to_activity,
                        activity_id: "activity:work".to_string(),
                    },
                    PlanStage::Activity {
                        activity_id: "activity:work".to_string(),
                    },
                ],
                0.5,
            ),
        );

        let mut vehicles = HashMap::new();
        vehicles.insert(
            vehicle_id.clone(),
            VehicleRecord {
                id: vehicle_id,
                kind: VehicleKind::Tram,
                route_id,
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.5,
                capacity: 4,
                occupants: Vec::new(),
                dwell_ticks_remaining: 2,
            },
        );

        let (mut world, schedule) = empty_world();
        api::set_link_polyline(
            &mut world,
            LinkId("link:home-to-old-town-stop".to_string()),
            vec![(0.0, 0.0), (10.0, 0.0)],
        );
        api::set_link_polyline(
            &mut world,
            LinkId("link:old-town-to-station".to_string()),
            vec![(10.0, 0.0), (20.0, 0.0)],
        );
        api::set_link_polyline(
            &mut world,
            LinkId("link:station-to-work".to_string()),
            vec![(20.0, 0.0), (30.0, 0.0)],
        );
        for (_, route) in routes {
            api::add_route(&mut world, route);
        }
        for (_, stop) in stops {
            api::add_stop(&mut world, stop);
        }
        for (_, agent) in agents {
            api::spawn_agent_from_record(&mut world, agent);
        }
        for (_, vehicle) in vehicles {
            api::spawn_vehicle_from_record(&mut world, vehicle);
        }
        (world, schedule)
    }

    #[test]
    fn world_coord_for_walking_agent_interpolates_link() {
        let (world, _) = seed::initial_world();
        let agent_id = AgentId("agent:seed:0".to_string());
        let expected = (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0);
        let coord = api::world_coord_for_agent(&world, &agent_id)
            .expect("agent resolves to coord");
        assert!((coord.0 - expected.0).abs() < 0.01);
        assert!((coord.1 - expected.1).abs() < 0.01);
    }

    #[test]
    fn world_coord_for_agent_waiting_at_stop_uses_stop_coord() {
        let (mut world, _) = empty_world();
        let stop_id = StopId("stop:horizontal:pickup".to_string());
        let route_id = RouteId("route:horizontal".to_string());
        let link_id = LinkId("link:horizontal:main".to_string());
        let start = (10.0, 20.0);
        let end = (30.0, 20.0);
        api::set_link_polyline(&mut world, link_id.clone(), vec![start, end]);
        api::add_route(
            &mut world,
            RouteRecord {
                id: route_id.clone(),
                links: vec![link_id],
            },
        );
        api::add_stop(
            &mut world,
            StopRecord {
                id: stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 0.0,
                waiting_agents: VecDeque::new(),
            },
        );
        let agent_id = AgentId("agent:waiter".to_string());
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                agent_id.clone(),
                AgentMobilityState::WaitingAtStop {
                    stop_id: stop_id.clone(),
                },
                vec![PlanStage::RideToStop { route_id, stop_id }],
                0.5,
            ),
        );
        let coord = api::world_coord_for_agent(&world, &agent_id)
            .expect("agent coord resolves");
        assert!((coord.0 - start.0).abs() < 0.01);
        assert!((coord.1 - start.1).abs() < 0.01);
    }

    #[test]
    fn world_coord_for_transit_vehicle_interpolates_route() {
        let (mut world, _) = empty_world();
        let route_id = RouteId("route:horizontal".to_string());
        let link_id = LinkId("link:horizontal:main".to_string());
        let start = (0.0, 0.0);
        let end = (100.0, 0.0);
        api::set_link_polyline(&mut world, link_id.clone(), vec![start, end]);
        api::add_route(
            &mut world,
            RouteRecord {
                id: route_id.clone(),
                links: vec![link_id],
            },
        );
        let vehicle_id = VehicleId("vehicle:test".to_string());
        api::spawn_vehicle_from_record(
            &mut world,
            VehicleRecord {
                id: vehicle_id.clone(),
                kind: VehicleKind::Tram,
                route_id,
                link_index: 0,
                progress: 0.5,
                speed_per_tick: 0.1,
                capacity: 4,
                occupants: Vec::new(),
                dwell_ticks_remaining: 0,
            },
        );
        let coord = api::world_coord_for_vehicle(&world, &vehicle_id)
            .expect("vehicle coord resolves");
        assert!((coord.0 - 50.0).abs() < 0.01);
        assert!((coord.1 - 0.0).abs() < 0.01);
    }

    #[test]
    fn sprite_key_for_agent_is_deterministic_by_id_hash() {
        let (world, _) = seed::initial_world();
        let a = api::sprite_key_for_agent(&world, &AgentId("agent:seed:0".to_string())).unwrap();
        let b = api::sprite_key_for_agent(&world, &AgentId("agent:seed:0".to_string())).unwrap();
        assert_eq!(
            a, b,
            "sprite key must be deterministic across calls for the same id"
        );
        assert!(a.starts_with("pedestrian:"));
    }

    #[test]
    fn agent_dto_built_through_world_includes_world_coord_direction_and_sprite_key() {
        let (world, _) = seed::initial_world();
        let agent_id = AgentId("agent:seed:0".to_string());
        let dto = api::agent_dto_for(&world, &agent_id).expect("agent exists");
        assert!(dto.sprite_key.starts_with("pedestrian:"));
        assert!(dto.world_coord.x.is_finite());
    }

    #[test]
    fn seeded_world_vehicles_default_to_tram_kind() {
        let (world, _) = seed::initial_world();
        for vehicle in api::vehicles(&world) {
            assert_eq!(vehicle.kind, VehicleKind::Tram);
        }
    }

    #[test]
    fn from_network_produces_expected_population_counts() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};

        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![
                vec![NetworkCoord { x: 10, y: 20 }, NetworkCoord { x: 30, y: 20 }],
                vec![NetworkCoord { x: 40, y: 60 }, NetworkCoord { x: 60, y: 60 }],
            ],
            pedestrian_corridors: vec![
                vec![NetworkCoord { x: 11, y: 30 }, NetworkCoord { x: 31, y: 30 }],
                vec![NetworkCoord { x: 41, y: 70 }, NetworkCoord { x: 61, y: 70 }],
                vec![NetworkCoord { x: 71, y: 80 }, NetworkCoord { x: 91, y: 80 }],
            ],
        };

        let density = seed::SeedDensity {
            pedestrians_per_corridor: 6,
            cars_per_arterial: 4,
            trams_total: 4,
        };
        let (world, _) = seed::from_network(&network, density);

        let walking_agents = api::agents(&world)
            .into_iter()
            .filter(|a| matches!(a.state, AgentMobilityState::Walking { .. }))
            .count();
        let driving_agents = api::agents(&world)
            .into_iter()
            .filter(|a| matches!(a.state, AgentMobilityState::InVehicle { .. }))
            .count();
        let cars = api::vehicles(&world)
            .into_iter()
            .filter(|v| v.kind == VehicleKind::Car)
            .count();
        let trams = api::vehicles(&world)
            .into_iter()
            .filter(|v| v.kind == VehicleKind::Tram)
            .count();

        assert_eq!(walking_agents, 18, "3 corridors x 6 = 18 walkers");
        assert_eq!(cars, 8, "2 arterials x 4 = 8 cars");
        assert_eq!(driving_agents, 8, "one driver per car");
        assert_eq!(trams, 4);
    }

    #[test]
    fn from_network_is_deterministic() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![vec![
                NetworkCoord { x: 0, y: 5 },
                NetworkCoord { x: 10, y: 5 },
            ]],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 3,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let (a_world, _) = seed::from_network(&network, density);
        let (b_world, _) = seed::from_network(&network, density);
        let a = extract_from_world(&a_world);
        let b = extract_from_world(&b_world);
        assert_eq!(a, b);
    }

    #[test]
    fn from_network_assigns_drivers_to_cars() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 0,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let (world, _) = seed::from_network(&network, density);

        let vehicles = api::vehicles(&world);
        assert_eq!(vehicles.len(), 2);
        for vehicle in &vehicles {
            assert_eq!(vehicle.kind, VehicleKind::Car);
            assert_eq!(vehicle.capacity, 1);
            assert_eq!(vehicle.occupants.len(), 1, "each car has its driver");
            let driver_id = &vehicle.occupants[0];
            let driver = api::agent(&world, driver_id).expect("driver agent exists");
            match &driver.state {
                AgentMobilityState::InVehicle { vehicle_id, .. } => {
                    assert_eq!(vehicle_id, &vehicle.id);
                }
                other => panic!("driver state expected InVehicle, got {other:?}"),
            }
        }
    }

    #[test]
    fn chunk_of_truncates_to_chunk_grid() {
        assert_eq!(chunk_of(0.0, 0.0, 32), ChunkCoord { x: 0, y: 0 });
        assert_eq!(chunk_of(31.9, 31.9, 32), ChunkCoord { x: 0, y: 0 });
        assert_eq!(chunk_of(32.0, 0.0, 32), ChunkCoord { x: 1, y: 0 });
        assert_eq!(chunk_of(150.5, 95.0, 32), ChunkCoord { x: 4, y: 2 });
    }

    #[test]
    fn chunk_of_handles_negative_coords() {
        assert_eq!(chunk_of(-0.1, -0.1, 32), ChunkCoord { x: -1, y: -1 });
    }

    #[test]
    fn snapshot_dto_includes_all_agents_even_in_vehicle() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 0,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let (world, _) = seed::from_network(&network, density);
        let world_id = WorldId("test".to_string());
        let snap = build_mobility_snapshot_dto(&world_id, api::tick(&world), &world);
        assert_eq!(
            snap.agents.len(),
            2,
            "snapshot must include in_vehicle drivers so clients can hydrate state"
        );
    }

    #[test]
    fn tick_mobility_indexes_lod_spawned_agents() {
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let (mut world, mut schedule) = empty_world();
        api::set_link_polyline(
            &mut world,
            LinkId("l:0".into()),
            vec![(10.0, 10.0), (20.0, 10.0)],
        );

        let chunk = ChunkCoord { x: 0, y: 0 };

        api::seed_flow_cell(
            &mut world,
            chunk,
            FlowCell {
                population: 2.0,
                outflow: std::collections::HashMap::new(),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        api::seed_chunk_activity(&mut world, chunk, MobilityActivity::Warm);
        api::seed_chunk_subscriber_count(&mut world, chunk, 1);

        api::tick_mobility(&mut world, &mut schedule);

        let lod_agents: Vec<_> = world
            .resource::<AgentIdIndex>()
            .0
            .keys()
            .filter(|id| id.0.starts_with("agent:lod:"))
            .cloned()
            .collect();
        assert_eq!(
            lod_agents.len(),
            2,
            "AgentIdIndex contains 2 LOD-spawned agents"
        );
    }

    #[test]
    fn snapshot_with_flow_cells_and_activities_round_trips() {
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let (mut world, _) = empty_world();
        let chunk = ChunkCoord { x: 1, y: 1 };
        api::seed_flow_cell(
            &mut world,
            chunk,
            FlowCell {
                population: 4.2,
                outflow: std::collections::HashMap::from([(ChunkCoord { x: 2, y: 1 }, 0.3)]),
                attractiveness: 1.5,
                last_tick: 100,
            },
        );
        api::seed_chunk_activity(&mut world, chunk, MobilityActivity::Warm);
        let snap = extract_from_world(&world);
        let json = serde_json::to_value(&snap).unwrap();
        let back: MobilityPersistSnapshot = serde_json::from_value(json.clone()).unwrap();
        let rejson = serde_json::to_value(&back).unwrap();
        assert_eq!(json, rejson);
    }

    #[test]
    fn tick_mobility_returns_per_chunk_deltas_with_changed_and_left() {
        let (mut world, mut schedule) = empty_world();
        api::force_all_chunks_active_for_test(&mut world);
        api::set_link_polyline(
            &mut world,
            LinkId("l".into()),
            vec![(4.0, 10.0), (60.0, 10.0)],
        );

        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("walker".into()),
                AgentMobilityState::Walking {
                    link_id: LinkId("l".into()),
                    progress: 0.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "act".into(),
                }],
                0.1,
            ),
        );

        let map1 = api::tick_mobility(&mut world, &mut schedule);
        assert!(
            map1.contains_key(&ChunkCoord { x: 0, y: 0 }),
            "tick 1: agent should be in chunk(0,0); map keys: {:?}",
            map1.keys().collect::<Vec<_>>()
        );
        let delta1 = &map1[&ChunkCoord { x: 0, y: 0 }];
        assert!(!delta1.changed_agents.is_empty());
        assert!(
            delta1.left_agents.is_empty(),
            "first tick: no previous chunk to leave"
        );

        let mut crossed = false;
        for _ in 0..20 {
            let map = api::tick_mobility(&mut world, &mut schedule);
            if let Some(delta) = map.get(&ChunkCoord { x: 0, y: 0 })
                && !delta.left_agents.is_empty()
            {
                assert!(
                    delta.left_agents.iter().any(|id| id.0 == "walker"),
                    "walker shows up in chunk(0,0).left_agents when it crosses out"
                );
                assert!(
                    map.get(&ChunkCoord { x: 1, y: 0 })
                        .map(|d| d.changed_agents.iter().any(|r| r.id.0 == "walker"))
                        .unwrap_or(false),
                    "walker shows up in chunk(1,0).changed_agents when it crosses in"
                );
                crossed = true;
                break;
            }
        }
        assert!(crossed, "agent must cross chunk boundary within 20 ticks");
    }

    #[test]
    fn tick_mobility_omits_unchanged_chunks() {
        let (mut world, mut schedule) = empty_world();
        api::force_all_chunks_active_for_test(&mut world);
        api::set_link_polyline(
            &mut world,
            LinkId("l".into()),
            vec![(10.0, 10.0), (20.0, 10.0)],
        );

        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("stationary".into()),
                AgentMobilityState::Walking {
                    link_id: LinkId("l".into()),
                    progress: 0.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "act".into(),
                }],
                0.0,
            ),
        );

        let _ = api::tick_mobility(&mut world, &mut schedule);
        let map = api::tick_mobility(&mut world, &mut schedule);
        assert!(
            map.get(&ChunkCoord { x: 0, y: 0 })
                .map(|d| d.changed_agents.is_empty() && d.left_agents.is_empty())
                .unwrap_or(true),
            "chunk with no changes should either be absent or have empty changed/left lists"
        );
    }

    #[test]
    fn build_chunk_snapshot_returns_only_entities_in_that_chunk() {
        let (mut world, mut schedule) = empty_world();

        api::set_link_polyline(
            &mut world,
            LinkId("l:a".into()),
            vec![(10.0, 10.0), (20.0, 10.0)],
        );
        api::set_link_polyline(
            &mut world,
            LinkId("l:b".into()),
            vec![(40.0, 10.0), (50.0, 10.0)],
        );

        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent-a".into()),
                AgentMobilityState::Walking {
                    link_id: LinkId("l:a".into()),
                    progress: 0.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "act".into(),
                }],
                0.0,
            ),
        );
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent-b".into()),
                AgentMobilityState::Walking {
                    link_id: LinkId("l:b".into()),
                    progress: 0.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "act".into(),
                }],
                0.0,
            ),
        );

        api::force_all_chunks_active_for_test(&mut world);
        api::tick_mobility(&mut world, &mut schedule);

        let snapshot = api::build_chunk_snapshot(&world, ChunkCoord { x: 0, y: 0 });
        let agent_ids: Vec<String> = snapshot.agents.iter().map(|a| a.id.0.clone()).collect();
        assert_eq!(
            agent_ids,
            vec!["agent-a"],
            "snapshot returns only chunk(0,0) agents"
        );
        assert!(snapshot.vehicles.is_empty());
        assert_eq!(snapshot.chunk, ChunkCoord { x: 0, y: 0 });
    }

    #[test]
    fn spawn_agent_from_record_initializes_position_from_link_polyline() {
        use crate::mobility::components::Position;

        let (mut world, _) = empty_world();
        api::set_link_polyline(
            &mut world,
            LinkId("l".into()),
            vec![(10.0, 20.0), (30.0, 40.0)],
        );
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("a".into()),
                AgentMobilityState::Walking {
                    link_id: LinkId("l".into()),
                    progress: 0.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "act".into(),
                }],
                0.0,
            ),
        );

        let entity = *world
            .resource::<AgentIdIndex>()
            .0
            .get(&AgentId("a".into()))
            .unwrap();
        let pos = world.entity(entity).get::<Position>().unwrap();
        assert_eq!((pos.x, pos.y), (10.0, 20.0));
    }

    #[test]
    fn spawn_vehicle_from_record_initializes_position_from_route() {
        use crate::mobility::components::Position;

        let (mut world, _) = empty_world();
        api::set_link_polyline(
            &mut world,
            LinkId("v".into()),
            vec![(100.0, 200.0), (300.0, 400.0)],
        );
        world
            .resource_mut::<crate::mobility::resources::Routes>()
            .0
            .insert(
                RouteId("r".into()),
                RouteRecord {
                    id: RouteId("r".into()),
                    links: vec![LinkId("v".into())],
                },
            );
        api::spawn_vehicle_from_record(
            &mut world,
            VehicleRecord {
                id: VehicleId("v1".into()),
                kind: VehicleKind::Tram,
                route_id: RouteId("r".into()),
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.0,
                capacity: 0,
                occupants: vec![],
                dwell_ticks_remaining: 0,
            },
        );

        let entity = *world
            .resource::<VehicleIdIndex>()
            .0
            .get(&VehicleId("v1".into()))
            .unwrap();
        let pos = world.entity(entity).get::<Position>().unwrap();
        assert_eq!((pos.x, pos.y), (100.0, 200.0));
    }

    #[test]
    fn agent_id_index_resource_matches_spawn_index() {
        let (mut world, _) = empty_world();
        let id_a = AgentId("a:1".into());
        let id_b = AgentId("a:2".into());
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                id_a.clone(),
                AgentMobilityState::AtActivity {
                    activity_id: "act".into(),
                },
                vec![],
                0.05,
            ),
        );
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                id_b.clone(),
                AgentMobilityState::AtActivity {
                    activity_id: "act".into(),
                },
                vec![],
                0.05,
            ),
        );

        let index = world.resource::<AgentIdIndex>();
        assert_eq!(index.0.len(), 2);
        assert!(index.0.contains_key(&id_a));
        assert!(index.0.contains_key(&id_b));
    }

    #[test]
    fn vehicle_id_index_resource_matches_spawn_index() {
        let (mut world, _) = empty_world();
        let id_v = VehicleId("v:1".into());
        api::spawn_vehicle_from_record(
            &mut world,
            VehicleRecord {
                id: id_v.clone(),
                kind: VehicleKind::Car,
                route_id: RouteId("r:1".into()),
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.1,
                capacity: 1,
                occupants: vec![],
                dwell_ticks_remaining: 0,
            },
        );

        let index = world.resource::<VehicleIdIndex>();
        assert_eq!(index.0.len(), 1);
        assert!(index.0.contains_key(&id_v));
    }
}
