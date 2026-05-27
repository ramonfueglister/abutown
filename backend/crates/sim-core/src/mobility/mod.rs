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
pub mod snapshot_provider;
pub mod systems;

pub use api::install_mobility;
pub use persist_snapshot::{MobilityPersistSnapshot, apply_into_world, extract_from_world};

use crate::world::schedule::SimPlugin;

pub struct MobilityPlugin;

impl SimPlugin for MobilityPlugin {
    fn name(&self) -> &'static str {
        "mobility"
    }
    fn install(
        &self,
        world: &mut bevy_ecs::world::World,
        schedule: &mut bevy_ecs::schedule::Schedule,
    ) {
        crate::mobility::api::install_mobility(world, schedule);
    }
}

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
    graph: &crate::routing::Graph,
    _transit_lines: &crate::routing::TransitLines,
) -> Option<(f32, f32)> {
    match state {
        AgentMobilityState::Walking { link_id, progress } => {
            let edge_id = crate::mobility::api::edge_by_canonical_key(graph, link_id)?;
            let edge = graph.edge(edge_id);
            Some(crate::mobility_geometry::world_coord_at_progress_slice(
                &edge.polyline,
                *progress,
            ))
        }
        AgentMobilityState::WaitingAtStop { stop_id }
        | AgentMobilityState::Boarding { stop_id, .. }
        | AgentMobilityState::Alighting { stop_id, .. } => {
            let node_id = graph.node_by_legacy(stop_id)?;
            Some(graph.node(node_id).position)
        }
        _ => None,
    }
}

/// World coord for a vehicle given its route position. Returns `None` if
/// the line or edge is not registered.
///
/// Phase 8b T10 (fixed): the graph is the single source of truth.
/// `RoutePosition` is `LineId`+`edge_index`-keyed; we look up the edge
/// directly via `TransitLines::line(line).edges[edge_index]` and read its
/// geometry from `Graph`.
pub fn vehicle_world_coord(
    route_position: &components::RoutePosition,
    transit_lines: &crate::routing::TransitLines,
    graph: &crate::routing::Graph,
) -> Option<(f32, f32)> {
    if (route_position.line_id.0 as usize) >= transit_lines.count() {
        return None;
    }
    let line = transit_lines.line(route_position.line_id);
    let edge_id = *line.edges.get(route_position.edge_index)?;
    let edge = graph.edge(edge_id);
    Some(crate::mobility_geometry::world_coord_at_progress_slice(
        &edge.polyline,
        route_position.progress,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, ChunkCoord, VehicleId};
    use crate::mobility::resources::{AgentIdIndex, VehicleIdIndex};
    use abutown_protocol::WorldId;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::world::World;
    use std::collections::HashMap;

    fn install_test_routing(world: &mut World) {
        use crate::routing::{Edge, EdgeId, EdgeKind, Graph, LineId, Node, NodeId, NodeKind};
        use crate::routing::{TransitLine, TransitLines};

        let nodes = vec![
            Node {
                id: NodeId(0),
                position: (10.0, 0.0),
                kind: NodeKind::TransitStop,
                legacy_id: Some("stop:old-town".into()),
            },
            Node {
                id: NodeId(1),
                position: (20.0, 0.0),
                kind: NodeKind::TransitStop,
                legacy_id: Some("stop:station".into()),
            },
            Node {
                id: NodeId(2),
                position: (10.0, 20.0),
                kind: NodeKind::TransitStop,
                legacy_id: Some("stop:horizontal:pickup".into()),
            },
            Node {
                id: NodeId(3),
                position: (30.0, 20.0),
                kind: NodeKind::TransitStop,
                legacy_id: Some("stop:horizontal:dropoff".into()),
            },
            Node {
                id: NodeId(4),
                position: (100.0, 200.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(5),
                position: (300.0, 400.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(6),
                position: (10.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(7),
                position: (20.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(8),
                position: (40.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(9),
                position: (50.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(10),
                position: (60.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(11),
                position: (80.0, 10.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ];
        let edges = vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(10.0, 0.0), (20.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::TramTrack,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("link:old-town-to-station".into()),
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(4),
                to: NodeId(5),
                polyline: vec![(100.0, 200.0), (300.0, 400.0)],
                length: 282.8427,
                kind: EdgeKind::TramTrack,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("v".into()),
            },
            Edge {
                id: EdgeId(2),
                from: NodeId(2),
                to: NodeId(3),
                polyline: vec![(10.0, 20.0), (30.0, 20.0)],
                length: 20.0,
                kind: EdgeKind::TramTrack,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("link:horizontal:main".into()),
            },
            Edge {
                id: EdgeId(3),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (10.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("link:home-to-old-town-stop".into()),
            },
            Edge {
                id: EdgeId(4),
                from: NodeId(1),
                to: NodeId(3),
                polyline: vec![(20.0, 0.0), (30.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("link:station-to-work".into()),
            },
            Edge {
                id: EdgeId(5),
                from: NodeId(2),
                to: NodeId(3),
                polyline: vec![(10.0, 20.0), (30.0, 40.0)],
                length: 28.28427,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l".into()),
            },
            Edge {
                id: EdgeId(6),
                from: NodeId(6),
                to: NodeId(7),
                polyline: vec![(10.0, 10.0), (20.0, 10.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:a".into()),
            },
            Edge {
                id: EdgeId(7),
                from: NodeId(8),
                to: NodeId(9),
                polyline: vec![(40.0, 10.0), (50.0, 10.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:b".into()),
            },
            Edge {
                id: EdgeId(8),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(4.0, 10.0), (60.0, 10.0)],
                length: 56.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("l:cross".into()),
            },
            Edge {
                id: EdgeId(9),
                from: NodeId(10),
                to: NodeId(11),
                polyline: vec![(60.0, 10.0), (80.0, 10.0)],
                length: 20.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: None,
            },
        ];
        let graph = Graph::new(nodes, edges);
        let mut lines = TransitLines::new(vec![
            TransitLine {
                id: LineId(0),
                name: "old-town-loop".into(),
                edges: vec![EdgeId(0)],
                stops: vec![NodeId(0), NodeId(1)],
                legacy_route_id: Some("route:old-town-loop".into()),
            },
            TransitLine {
                id: LineId(1),
                name: "r".into(),
                edges: vec![EdgeId(1)],
                stops: Vec::new(),
                legacy_route_id: Some("r".into()),
            },
            TransitLine {
                id: LineId(2),
                name: "horizontal".into(),
                edges: vec![EdgeId(2)],
                stops: vec![NodeId(2), NodeId(3)],
                legacy_route_id: Some("route:horizontal".into()),
            },
        ]);
        lines.add_legacy_route_alias("r:1".into(), LineId(1));
        world.insert_resource(graph);
        world.insert_resource(lines);
        if !world.contains_resource::<crate::routing::WaitingAgents>() {
            world.insert_resource(crate::routing::WaitingAgents::default());
        }
    }

    fn empty_world() -> (World, Schedule) {
        let (mut world, schedule) = api::empty_world_and_schedule();
        install_test_routing(&mut world);
        (world, schedule)
    }

    #[test]
    fn initial_world_seeds_expected_population() {
        let (world, _) = seed::initial_world();

        assert_eq!(api::tick(&world), 0);
        assert_eq!(
            world.resource::<crate::routing::TransitLines>().count(),
            2,
            "expected 2 lines"
        );

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
        let stop = api::stop(&world, "stop:old-town").expect("sample stop exists");

        assert_eq!(agent.plan_cursor, 0);
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: "link:home-to-old-town-stop".to_string(),
                progress: 0.0
            }
        );
        assert_eq!(vehicle.route_id, "route:old-town-loop".to_string());
        assert_eq!(vehicle.capacity, 4);
        assert_eq!(stop.route_id, "route:old-town-loop".to_string());
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
                link_id: "link:home-to-old-town-stop".to_string(),
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
        let stop = api::stop(&world, "stop:old-town").expect("pickup stop exists");

        assert_eq!(
            agent.state,
            AgentMobilityState::WaitingAtStop {
                stop_id: "stop:old-town".to_string()
            }
        );
        assert_eq!(agent.plan_cursor, 1);
        assert_eq!(stop.waiting_agents.to_vec(), vec![agent_id]);
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
    fn vehicle_restarts_route_after_reaching_link_end() {
        let (mut world, mut schedule) = sample_world();
        api::force_all_chunks_active_for_test(&mut world);
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        for _ in 0..4 {
            api::tick_mobility(&mut world, &mut schedule);
        }

        let at_end = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(at_end.progress, 1.0);

        api::tick_mobility(&mut world, &mut schedule);

        let restarted = api::vehicle(&world, &vehicle_id).expect("vehicle exists");
        assert_eq!(restarted.link_index, 0);
        assert_eq!(restarted.progress, 0.0);
    }

    #[test]
    fn activity_only_walker_restarts_after_reaching_link_end() {
        let (mut world, mut schedule) = empty_world();
        api::force_all_chunks_active_for_test(&mut world);
        let agent_id = AgentId("agent:ambient".to_string());
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                agent_id.clone(),
                AgentMobilityState::Walking {
                    link_id: "l".to_string(),
                    progress: 1.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "activity:wander".to_string(),
                }],
                0.05,
            ),
        );

        api::tick_mobility(&mut world, &mut schedule);

        let agent = api::agent(&world, &agent_id).expect("agent exists");
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: "l".to_string(),
                progress: 0.05
            }
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
                stop_id: "stop:old-town".to_string()
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
                link_id: "link:station-to-work".to_string(),
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
        let route_id = "route:old-town-loop".to_string();
        let pickup_stop_id = "stop:old-town".to_string();
        let dropoff_stop_id = "stop:station".to_string();
        let walk_to_pickup = "link:home-to-old-town-stop".to_string();
        let walk_to_activity = "link:station-to-work".to_string();
        let agent_id = AgentId("agent:pedestrian:0".to_string());
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

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
        let coord = api::world_coord_for_agent(&world, &agent_id).expect("agent resolves to coord");
        assert!((coord.0 - expected.0).abs() < 0.01);
        assert!((coord.1 - expected.1).abs() < 0.01);
    }

    #[test]
    fn world_coord_for_walking_agent_accepts_graph_native_edge_key() {
        let (mut world, _) = empty_world();
        let agent_id = AgentId("agent:graph-native".to_string());
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                agent_id.clone(),
                AgentMobilityState::Walking {
                    link_id: "edge:9".to_string(),
                    progress: 0.5,
                },
                Vec::new(),
                0.0,
            ),
        );

        let coord = api::world_coord_for_agent(&world, &agent_id).expect("agent coord resolves");
        assert!((coord.0 - 70.0).abs() < 0.01);
        assert!((coord.1 - 10.0).abs() < 0.01);
    }

    #[test]
    fn extract_snapshot_includes_graph_native_walking_polyline() {
        let (mut world, _) = empty_world();
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent:graph-native".to_string()),
                AgentMobilityState::Walking {
                    link_id: "edge:9".to_string(),
                    progress: 0.5,
                },
                Vec::new(),
                0.0,
            ),
        );

        let snapshot = extract_from_world(&world);

        assert_eq!(
            snapshot.link_polylines.get("edge:9"),
            Some(&vec![(60.0, 10.0), (80.0, 10.0)])
        );
    }

    #[test]
    fn world_coord_for_agent_waiting_at_stop_uses_stop_coord() {
        let (mut world, _) = empty_world();
        let stop_id = "stop:horizontal:pickup".to_string();
        let route_id = "route:horizontal".to_string();
        let start = (10.0, 20.0);
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
        let coord = api::world_coord_for_agent(&world, &agent_id).expect("agent coord resolves");
        assert!((coord.0 - start.0).abs() < 0.01);
        assert!((coord.1 - start.1).abs() < 0.01);
    }

    #[test]
    fn world_coord_for_transit_vehicle_interpolates_route() {
        let (mut world, _) = empty_world();
        let route_id = "route:horizontal".to_string();
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
        let coord =
            api::world_coord_for_vehicle(&world, &vehicle_id).expect("vehicle coord resolves");
        assert!((coord.0 - 20.0).abs() < 0.01);
        assert!((coord.1 - 20.0).abs() < 0.01);
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
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("walker".into()),
                AgentMobilityState::Walking {
                    link_id: "l:cross".into(),
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
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("stationary".into()),
                AgentMobilityState::Walking {
                    link_id: "l".into(),
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

        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent-a".into()),
                AgentMobilityState::Walking {
                    link_id: "l:a".into(),
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
                    link_id: "l:b".into(),
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

        let snapshot = api::build_mobility_chunk_snapshot(&world, ChunkCoord { x: 0, y: 0 });
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
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("a".into()),
                AgentMobilityState::Walking {
                    link_id: "l".into(),
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
        api::spawn_vehicle_from_record(
            &mut world,
            VehicleRecord {
                id: VehicleId("v1".into()),
                kind: VehicleKind::Tram,
                route_id: "r".into(),
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
                route_id: "r:1".into(),
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
