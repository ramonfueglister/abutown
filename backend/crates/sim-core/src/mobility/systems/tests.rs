use super::*;
use std::collections::HashSet;
/// Returns a `SimulatedChunks` resource pre-populated so that every
/// chunk in a generous range around the origin counts as simulated.
/// Tests that exercise the LOD-filtered Advance/Output systems use
/// this so the filter doesn't skip their fixtures.
fn all_active() -> SimulatedChunks {
    let mut a = SimulatedChunks::default();
    for x in -10..=20 {
        for y in -10..=20 {
            a.0.insert(crate::ids::ChunkCoord { x, y });
        }
    }
    a
}
fn insert_test_routing(world: &mut World) -> crate::routing::TrafficRouteId {
    use crate::routing::{
        Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind, TrafficRoute, TrafficRouteId,
        TrafficRoutes,
    };
    let nodes = vec![
        Node {
            id: NodeId(0),
            position: (0.0, 0.0),
            kind: NodeKind::TransitStop,
            legacy_id: Some("s:1".into()),
        },
        Node {
            id: NodeId(1),
            position: (10.0, 0.0),
            kind: NodeKind::TransitStop,
            legacy_id: Some("s:end".into()),
        },
        Node {
            id: NodeId(2),
            position: (0.0, 10.0),
            kind: NodeKind::Intersection,
            legacy_id: None,
        },
        Node {
            id: NodeId(3),
            position: (40.0, 5.0),
            kind: NodeKind::Intersection,
            legacy_id: None,
        },
    ];
    let edges = vec![
        Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (10.0, 0.0)],
            length: 10.0,
            kind: EdgeKind::Road,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:vehicle".into()),
        },
        Edge {
            id: EdgeId(1),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (10.0, 0.0)],
            length: 10.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:1".into()),
        },
        Edge {
            id: EdgeId(2),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (10.0, 0.0)],
            length: 10.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("link:test".into()),
        },
        Edge {
            id: EdgeId(3),
            from: NodeId(0),
            to: NodeId(2),
            polyline: vec![(0.0, 0.0), (0.0, 10.0)],
            length: 10.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:b".into()),
        },
        Edge {
            id: EdgeId(4),
            from: NodeId(1),
            to: NodeId(2),
            polyline: vec![(10.0, 0.0), (0.0, 10.0)],
            length: 20.0_f32.sqrt() * 10.0_f32.sqrt(),
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:2".into()),
        },
        Edge {
            id: EdgeId(5),
            from: NodeId(0),
            to: NodeId(3),
            polyline: vec![(5.0, 5.0), (40.0, 5.0)],
            length: 35.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:end".into()),
        },
        Edge {
            id: EdgeId(6),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(10.0, 10.0), (20.0, 10.0)],
            length: 10.0,
            kind: EdgeKind::Footway,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:0".into()),
        },
        Edge {
            id: EdgeId(7),
            from: NodeId(0),
            to: NodeId(2),
            polyline: vec![(0.0, 0.0), (0.0, 10.0)],
            length: 10.0,
            kind: EdgeKind::Road,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some("l:b".into()),
        },
    ];
    let mut graph = Graph::new(nodes, edges);
    graph.add_legacy_node_alias("stop:old-town".into(), NodeId(0));
    graph.add_legacy_node_alias("stop:station".into(), NodeId(1));
    let route_id = TrafficRouteId(0);
    let routes = TrafficRoutes::new(vec![TrafficRoute {
        id: route_id,
        name: "r:1".into(),
        edges: vec![EdgeId(0), EdgeId(7)],
        legacy_route_id: "r:1".into(),
    }]);
    world.insert_resource(graph);
    world.insert_resource(routes);
    if !world.contains_resource::<crate::routing::WaitingAgents>() {
        world.insert_resource(crate::routing::WaitingAgents::default());
    }
    route_id
}
#[test]
fn tick_increment_system_advances_tick_by_one_per_schedule_run() {
    // Use the full plugin install path — the LOD set now depends on
    // foundation-owned `Messages<ChunkLodChanged>` and a configured
    // `CoreSet::LodReclassify`, so building the world by hand is brittle.
    let (mut world, mut schedule) = crate::mobility::api::empty_world_and_schedule();
    // Replace the freshly-installed (empty) SimulatedChunks with one
    // primed for the wide tile range any future seed could land in —
    // we're only asserting on Tick(), but the install order keeps the
    // gating systems happy.
    *world.resource_mut::<SimulatedChunks>() = all_active();
    schedule.run(&mut world);
    assert_eq!(world.resource::<Tick>().0, 1);
    schedule.run(&mut world);
    assert_eq!(world.resource::<Tick>().0, 2);
}
#[test]
fn stop_arrival_transitions_walking_agent_to_waiting_at_stop() {
    use crate::ids::AgentId;
    use crate::mobility::records::{AgentMobilityState, PlanStage};
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:1".into(),
                progress: 1.0,
            }),
            WalkPlan {
                stages: vec![PlanStage::WalkToStop {
                    link_id: "l:1".into(),
                    stop_id: "s:1".into(),
                }],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.1),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            NearStop,
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(stop_arrival_system);
    schedule.run(&mut world);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    match &state.0 {
        AgentMobilityState::WaitingAtStop { stop_id } => {
            assert_eq!(stop_id.as_str(), "s:1");
        }
        other => panic!("expected WaitingAtStop, got {other:?}"),
    }
    let plan = world.get::<WalkPlan>(entity).unwrap();
    assert_eq!(plan.cursor, 1);
    let node_id = world
        .resource::<crate::routing::Graph>()
        .node_by_legacy("s:1")
        .unwrap();
    let waiting = world.resource::<crate::routing::WaitingAgents>();
    assert_eq!(
        waiting.queue(node_id).and_then(|queue| queue.front()),
        Some(&AgentId("a:1".into()))
    );
    assert!(world.resource::<DirtyAgents>().0.contains(&entity));
}
#[test]
fn walk_advance_advances_progress_by_walk_speed() {
    use crate::ids::AgentId;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(Tick(0));
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "link:test".into(),
                progress: 0.2,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.1),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(walk_advance_system);
    schedule.run(&mut world);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    match &state.0 {
        AgentMobilityState::Walking { progress, .. } => {
            assert!(
                (progress - 0.3).abs() < 1e-6,
                "progress should be 0.3, got {progress}"
            );
        }
        other => panic!("expected Walking, got {other:?}"),
    }
    assert!(world.resource::<DirtyAgents>().0.contains(&entity));
}
#[test]
fn walk_advance_clamps_at_one_and_marks_dirty() {
    use crate::ids::AgentId;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:near".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "link:test".into(),
                progress: 0.95,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.1),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(walk_advance_system);
    schedule.run(&mut world);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    match &state.0 {
        AgentMobilityState::Walking { progress, .. } => {
            assert!(
                (progress - 1.0).abs() < 1e-6,
                "progress clamped to 1.0, got {progress}"
            );
        }
        _ => panic!(),
    }
    assert!(world.resource::<DirtyAgents>().0.contains(&entity));
}
#[test]
fn vehicle_advance_decrements_dwell_when_positive() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(DirtyVehicles::default());
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let route_id = crate::routing::TrafficRouteId(0);
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 0.5,
                speed: 0.1,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(3),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(vehicle_advance_system);
    schedule.run(&mut world);
    let dwell = world.get::<DwellTicksRemaining>(entity).unwrap();
    assert_eq!(dwell.0, 2);
    let pos = world.get::<RoutePosition>(entity).unwrap();
    assert!(
        (pos.progress - 0.5).abs() < 1e-6,
        "progress unchanged during dwell"
    );
    assert!(world.resource::<DirtyVehicles>().0.contains(&entity));
}
#[test]
fn vehicle_advance_progresses_when_not_dwelling() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    world.insert_resource(DirtyVehicles::default());
    let route_id = crate::routing::TrafficRouteId(0);
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 0.4,
                speed: 0.1,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(vehicle_advance_system);
    schedule.run(&mut world);
    let pos = world.get::<RoutePosition>(entity).unwrap();
    assert!((pos.progress - 0.5).abs() < 1e-6);
    assert!(world.resource::<DirtyVehicles>().0.contains(&entity));
}
#[test]
fn vehicle_advance_loops_car_over_traffic_route_edges() {
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    let route_id = insert_test_routing(&mut world);
    world.insert_resource(SimulatedChunks(
        std::iter::once(crate::ids::ChunkCoord { x: 0, y: 0 }).collect(),
    ));
    world.insert_resource(DirtyVehicles::default());
    let entity = world
        .spawn((
            VehicleMarker,
            VehicleKindComponent(VehicleKind::Car),
            Position { x: 10.0, y: 0.0 },
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 1.0,
                speed: 0.1,
            },
            DwellTicksRemaining(0),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(vehicle_advance_system);
    schedule.run(&mut world);
    let pos = world.get::<RoutePosition>(entity).unwrap();
    assert_eq!(pos.edge_index, 1);
    assert_eq!(pos.progress, 0.0);
}
#[test]
fn vehicle_advance_requires_traffic_routes() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    world.insert_resource(DirtyVehicles::default());
    world.insert_resource(crate::routing::TrafficRoutes::default());
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:legacy".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id: crate::routing::TrafficRouteId(0),
                edge_index: 0,
                progress: 0.4,
                speed: 0.1,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(vehicle_advance_system);
    schedule.run(&mut world);
    let pos = world.get::<RoutePosition>(entity).unwrap();
    assert!(
        (pos.progress - 0.4).abs() < 1e-6,
        "vehicles must not advance without traffic routes"
    );
    assert!(!world.resource::<DirtyVehicles>().0.contains(&entity));
}
#[test]
fn vehicle_advance_skips_invalid_route_edge_index_before_state_mutation() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    world.insert_resource(DirtyVehicles::default());
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:invalid-edge".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id: crate::routing::TrafficRouteId(0),
                edge_index: 99,
                progress: 0.4,
                speed: 0.1,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(2),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(vehicle_advance_system);
    schedule.run(&mut world);
    let pos = world.get::<RoutePosition>(entity).unwrap();
    assert_eq!(pos.edge_index, 99);
    assert!(
        (pos.progress - 0.4).abs() < 1e-6,
        "invalid route edge index must not advance progress"
    );
    assert_eq!(
        world.get::<DwellTicksRemaining>(entity).unwrap().0,
        2,
        "invalid route edge index must not decrement dwell"
    );
    assert!(!world.resource::<DirtyVehicles>().0.contains(&entity));
}
#[test]
fn compute_world_coord_system_writes_position_for_walking_agent() {
    use crate::ids::AgentId;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:1".into(),
                progress: 0.5,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.0),
            Position { x: 99.0, y: 99.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(compute_world_coord_system);
    schedule.run(&mut world);
    let pos = world.get::<Position>(entity).unwrap();
    assert!(
        (pos.x - 5.0).abs() < 1e-3,
        "x at midpoint of 0..10 = 5.0, got {}",
        pos.x
    );
    assert!(pos.y.abs() < 1e-3);
}
#[test]
fn compute_direction_system_writes_direction_for_walking_agent() {
    use crate::ids::AgentId;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:1".into(),
                progress: 0.5,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.0),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(compute_direction_system);
    schedule.run(&mut world);
    let dir = world.get::<Direction>(entity).unwrap();
    // East-pointing polyline → DirectionDto::E
    assert_eq!(dir.0, abutown_protocol::DirectionDto::E);
}
#[test]
fn track_chunk_populations_sums_agents_vehicles_and_flow_cells() {
    use crate::ids::*;
    use crate::mobility::lod::FlowCell;
    use crate::mobility::records::{AgentMobilityState, VehicleKind};
    let mut world = World::new();
    insert_test_routing(&mut world);
    let mut flow_cells = FlowCells::default();
    flow_cells.0.insert(
        ChunkCoord { x: 0, y: 0 },
        FlowCell {
            population: 3.7,
            outflow: std::collections::HashMap::new(),
            attractiveness: 1.0,
            last_tick: 0,
        },
    );
    world.insert_resource(flow_cells);
    world.insert_resource(ChunkPopulations::default());
    world.insert_resource(AgentsByChunk::default());
    world.insert_resource(VehiclesByChunk::default());
    world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
    world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());
    for n in 0..2 {
        world.spawn((
            AgentMarker,
            StableAgentId(AgentId(format!("a:{n}"))),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l".into(),
                progress: 0.0,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.0),
            Position { x: 40.0, y: 16.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ));
    }
    world.spawn((
        VehicleMarker,
        StableVehicleId(VehicleId("v:1".into())),
        VehicleKindComponent(VehicleKind::Car),
        RoutePosition {
            route_id: crate::routing::TrafficRouteId(0),
            edge_index: 0,
            progress: 0.0,
            speed: 0.0,
        },
        Capacity(1),
        Occupants(vec![]),
        DwellTicksRemaining(0),
        Position { x: 80.0, y: 16.0 },
        Direction(abutown_protocol::DirectionDto::S),
        SpriteKey(String::new()),
    ));
    let mut schedule = Schedule::default();
    schedule.add_systems(track_chunk_populations_system);
    schedule.run(&mut world);
    let pops = world.resource::<ChunkPopulations>();
    assert_eq!(pops.0.get(&ChunkCoord { x: 1, y: 0 }), Some(&2)); // two agents
    assert_eq!(pops.0.get(&ChunkCoord { x: 2, y: 0 }), Some(&1)); // one vehicle
    assert_eq!(pops.0.get(&ChunkCoord { x: 0, y: 0 }), Some(&3)); // floor(3.7) flow cell
}
#[test]
fn compute_world_coord_writes_position_for_vehicle() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let route_id = crate::routing::TrafficRouteId(0);
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 0.25,
                speed: 0.0,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 99.0, y: 99.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(compute_world_coord_system);
    schedule.run(&mut world);
    let pos = world.get::<Position>(entity).unwrap();
    assert!(
        (pos.x - 2.5).abs() < 1e-3,
        "0.25 of the graph edge 0..10 = 2.5, got {}",
        pos.x
    );
    assert!(pos.y.abs() < 1e-3);
}
#[test]
fn compute_world_coord_ignores_stale_vehicle_polyline_cache() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    use std::sync::Arc;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    let route_id = crate::routing::TrafficRouteId(0);
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:stale-cache".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 1,
                progress: 0.5,
                speed: 0.0,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 99.0, y: 99.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            CurrentLinkPolyline {
                link_id: "l:vehicle".into(),
                points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
            },
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(compute_world_coord_system);
    schedule.run(&mut world);
    let pos = world.get::<Position>(entity).unwrap();
    assert!(pos.x.abs() < 1e-3);
    assert!(
        (pos.y - 5.0).abs() < 1e-3,
        "edge 1 should resolve through TrafficRoutes instead of stale cached edge 0"
    );
}
#[test]
fn compute_direction_ignores_stale_vehicle_polyline_cache() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    use std::sync::Arc;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    let route_id = crate::routing::TrafficRouteId(0);
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:stale-direction-cache".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 1,
                progress: 0.5,
                speed: 0.0,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 0.0, y: 5.0 },
            Direction(abutown_protocol::DirectionDto::E),
            SpriteKey(String::new()),
            CurrentLinkPolyline {
                link_id: "l:vehicle".into(),
                points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
            },
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(compute_direction_system);
    schedule.run(&mut world);
    let dir = world.get::<Direction>(entity).unwrap();
    assert_eq!(
        dir.0,
        abutown_protocol::DirectionDto::S,
        "edge 1 direction should resolve through TrafficRoutes instead of stale cached edge 0"
    );
}
#[test]
fn subscribe_drives_chunk_active_via_entity_classifier() {
    // End-to-end: apply_subscription_diff -> reclassify_chunk_lod_system
    // -> ChunkLodChanged -> consume_chunk_lod_transitions_system. Activity
    // for the subscribed chunk reaches Active after one schedule tick.
    use crate::ids::ChunkCoord;
    use crate::mobility::lod::MobilityActivity;
    let (mut world, mut schedule) = crate::mobility::api::empty_world_and_schedule();
    let chunk = ChunkCoord { x: 4, y: 4 };
    crate::mobility::api::apply_subscription_diff(&mut world, &[chunk], std::iter::empty());
    schedule.run(&mut world);
    assert_eq!(
        crate::mobility::api::activity_for_chunk(&world, chunk),
        Some(MobilityActivity::Active),
        "single subscriber → Active on first tick",
    );
}
#[test]
fn consume_chunk_lod_transitions_publishes_event_to_scratchpad() {
    // Manually write a `ChunkLodChanged` event and assert it lands in
    // `ChunkLodTransitions` after running the consumer system.
    use crate::ids::ChunkCoord;
    let (mut world, _schedule) = crate::mobility::api::empty_world_and_schedule();
    let chunk = ChunkCoord { x: 1, y: 2 };
    world
        .resource_mut::<Messages<ChunkLodChanged>>()
        .write(ChunkLodChanged {
            entity: Entity::PLACEHOLDER,
            coord: chunk,
            from: ChunkLod::Asleep,
            to: ChunkLod::Active,
        });
    let mut sched = Schedule::default();
    sched.add_systems(consume_chunk_lod_transitions_system);
    sched.run(&mut world);
    let scratch = world.resource::<ChunkLodTransitions>();
    assert_eq!(scratch.0.len(), 1);
    assert_eq!(scratch.0[0].0, chunk);
    assert_eq!(scratch.0[0].1, ChunkLod::Asleep);
    assert_eq!(scratch.0[0].2, ChunkLod::Active);
}
#[test]
fn promote_warm_spawns_floor_population_agents() {
    use crate::ids::*;
    use crate::mobility::lod::FlowCell;
    let mut world = World::new();
    insert_test_routing(&mut world);
    let chunk = ChunkCoord { x: 0, y: 0 };
    let mut flow = FlowCells::default();
    flow.0.insert(
        chunk,
        FlowCell {
            population: 3.7,
            outflow: std::collections::HashMap::new(),
            attractiveness: 1.0,
            last_tick: 0,
        },
    );
    world.insert_resource(flow);
    let mut transitions = ChunkLodTransitions::default();
    transitions
        .0
        .push((chunk, ChunkLod::Warm, ChunkLod::Active));
    world.insert_resource(transitions);
    world.insert_resource(Tick(100));
    let mut schedule = Schedule::default();
    schedule.add_systems(promote_warm_to_active_system);
    schedule.run(&mut world);
    let mut query = world.query_filtered::<Entity, With<AgentMarker>>();
    let spawned: Vec<Entity> = query.iter(&world).collect();
    assert_eq!(spawned.len(), 3);
    let cell = world.resource::<FlowCells>().0.get(&chunk).unwrap();
    assert!((cell.population - 0.7).abs() < 1e-6);
}
#[test]
fn demote_active_to_warm_collapses_agents_into_flow_cell() {
    use crate::ids::*;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    let chunk = ChunkCoord { x: 0, y: 0 };
    world.insert_resource(FlowCells::default());
    world.insert_resource(ChunkPopulations::default());
    world.insert_resource(AgentsByChunk::default());
    world.insert_resource(VehiclesByChunk::default());
    world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
    world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());
    let mut transitions = ChunkLodTransitions::default();
    transitions
        .0
        .push((chunk, ChunkLod::Active, ChunkLod::Warm));
    world.insert_resource(transitions);
    for n in 0..3 {
        world.spawn((
            AgentMarker,
            StableAgentId(AgentId(format!("a:{n}"))),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:end".into(),
                progress: 0.1,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.05),
            Position {
                x: 5.0 + n as f32,
                y: 5.0,
            },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ));
    }
    let mut schedule = Schedule::default();
    schedule.add_systems((
        track_chunk_populations_system,
        demote_active_to_warm_system.after(track_chunk_populations_system),
    ));
    schedule.run(&mut world);
    let cell = world
        .resource::<FlowCells>()
        .0
        .get(&chunk)
        .expect("flow cell created");
    assert!((cell.population - 3.0).abs() < 1e-6);
    let dest = ChunkCoord { x: 1, y: 0 };
    assert!(
        cell.outflow.contains_key(&dest),
        "outflow should target end-of-link chunk"
    );
    let remaining: u32 = {
        let mut q = world.query_filtered::<Entity, With<AgentMarker>>();
        q.iter(&world).count() as u32
    };
    assert_eq!(remaining, 0, "agents despawned");
}
#[test]
fn demote_active_to_warm_keeps_vehicles_concrete() {
    use crate::ids::*;
    use crate::mobility::records::VehicleKind;
    let mut world = World::new();
    let route_id = insert_test_routing(&mut world);
    let chunk = ChunkCoord { x: 0, y: 0 };
    world.insert_resource(FlowCells::default());
    world.insert_resource(ChunkPopulations::default());
    world.insert_resource(AgentsByChunk::default());
    world.insert_resource(VehiclesByChunk::default());
    world.insert_resource(crate::mobility::resources::PreviousChunkByEntity::default());
    world.insert_resource(crate::mobility::resources::PreviousFlowCellContrib::default());
    let vehicle = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:street-car".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 0.1,
                speed: 0.02,
            },
            Capacity(4),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 5.0, y: 5.0 },
            Direction(abutown_protocol::DirectionDto::E),
            SpriteKey(String::new()),
        ))
        .id();
    let mut transitions = ChunkLodTransitions::default();
    transitions
        .0
        .push((chunk, ChunkLod::Active, ChunkLod::Warm));
    world.insert_resource(transitions);
    let mut schedule = Schedule::default();
    schedule.add_systems((
        track_chunk_populations_system,
        demote_active_to_warm_system.after(track_chunk_populations_system),
    ));
    schedule.run(&mut world);
    assert!(
        world.get::<VehicleMarker>(vehicle).is_some(),
        "street vehicles stay as concrete entities instead of being folded into flow cells"
    );
    assert!(
        !world.resource::<FlowCells>().0.contains_key(&chunk),
        "vehicle-only demotion must not create anonymous population"
    );
}
#[test]
fn warm_chunk_flow_transfers_population_between_chunks() {
    use crate::ids::ChunkCoord;
    use crate::mobility::lod::FlowCell;
    let mut world = World::new();
    world.insert_resource(Tick(10));
    let mut warm = WarmChunkCoords::default();
    warm.0.insert(ChunkCoord { x: 0, y: 0 });
    world.insert_resource(warm);
    let mut flow = FlowCells::default();
    flow.0.insert(
        ChunkCoord { x: 0, y: 0 },
        FlowCell {
            population: 10.0,
            outflow: std::collections::HashMap::from([(ChunkCoord { x: 1, y: 0 }, 0.5)]),
            attractiveness: 1.0,
            last_tick: 0,
        },
    );
    world.insert_resource(flow);
    let mut schedule = Schedule::default();
    schedule.add_systems(warm_chunk_flow_system);
    schedule.run(&mut world);
    let cells = world.resource::<FlowCells>();
    let src = cells.0.get(&ChunkCoord { x: 0, y: 0 }).unwrap();
    let dst = cells.0.get(&ChunkCoord { x: 1, y: 0 }).unwrap();
    assert!((src.population - 5.0).abs() < 1e-3);
    assert!((dst.population - 5.0).abs() < 1e-3);
}
#[test]
fn warm_chunk_flow_skips_non_multiple_of_10_ticks() {
    use crate::ids::ChunkCoord;
    use crate::mobility::lod::FlowCell;
    let mut world = World::new();
    world.insert_resource(Tick(5));
    let mut warm = WarmChunkCoords::default();
    warm.0.insert(ChunkCoord { x: 0, y: 0 });
    world.insert_resource(warm);
    let mut flow = FlowCells::default();
    flow.0.insert(
        ChunkCoord { x: 0, y: 0 },
        FlowCell {
            population: 10.0,
            outflow: std::collections::HashMap::from([(ChunkCoord { x: 1, y: 0 }, 0.5)]),
            attractiveness: 1.0,
            last_tick: 0,
        },
    );
    world.insert_resource(flow);
    let mut schedule = Schedule::default();
    schedule.add_systems(warm_chunk_flow_system);
    schedule.run(&mut world);
    let cells = world.resource::<FlowCells>();
    let src = cells.0.get(&ChunkCoord { x: 0, y: 0 }).unwrap();
    assert!(
        (src.population - 10.0).abs() < 1e-3,
        "skipped on non-multiple-of-10 tick"
    );
}
#[test]
fn promote_warm_is_deterministic_across_runs() {
    use crate::ids::*;
    use crate::mobility::lod::FlowCell;
    fn run_promote() -> Vec<String> {
        let mut world = World::new();
        insert_test_routing(&mut world);
        let chunk = ChunkCoord { x: 0, y: 0 };
        let mut flow = FlowCells::default();
        flow.0.insert(
            chunk,
            FlowCell {
                population: 5.0,
                outflow: std::collections::HashMap::new(),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        world.insert_resource(flow);
        let mut transitions = ChunkLodTransitions::default();
        transitions
            .0
            .push((chunk, ChunkLod::Warm, ChunkLod::Active));
        world.insert_resource(transitions);
        world.insert_resource(Tick(42));
        let mut schedule = Schedule::default();
        schedule.add_systems(promote_warm_to_active_system);
        schedule.run(&mut world);
        let mut query = world.query::<&StableAgentId>();
        let mut ids: Vec<String> = query.iter(&world).map(|s| s.0.0.clone()).collect();
        ids.sort();
        ids
    }
    let a = run_promote();
    let b = run_promote();
    assert_eq!(
        a, b,
        "promote must be deterministic across runs (same chunk + tick → same ids)"
    );
}
#[test]
fn walk_advance_skips_agents_in_asleep_chunks() {
    use crate::ids::*;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(SimulatedChunks::default()); // empty = none simulated
    world.insert_resource(DirtyAgents::default());
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:0".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:0".into(),
                progress: 0.5,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.1),
            Position { x: 100.0, y: 100.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(walk_advance_system);
    schedule.run(&mut world);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    match &state.0 {
        AgentMobilityState::Walking { progress, .. } => {
            assert!(
                (progress - 0.5).abs() < 1e-6,
                "progress unchanged in Asleep chunk"
            );
        }
        _ => panic!(),
    }
    assert!(
        !world.resource::<DirtyAgents>().0.contains(&entity),
        "Asleep-chunk agent must not be marked dirty"
    );
}
#[test]
fn walk_advance_advances_agents_in_active_chunks() {
    use crate::ids::*;
    use crate::mobility::records::AgentMobilityState;
    let mut world = World::new();
    insert_test_routing(&mut world);
    let mut simulated = SimulatedChunks::default();
    // Position (100, 100) → chunk (3, 3) for chunk_size = 32.
    simulated.0.insert(ChunkCoord { x: 3, y: 3 });
    world.insert_resource(simulated);
    world.insert_resource(DirtyAgents::default());
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:0".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:0".into(),
                progress: 0.5,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.1),
            Position { x: 100.0, y: 100.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(walk_advance_system);
    schedule.run(&mut world);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    match &state.0 {
        AgentMobilityState::Walking { progress, .. } => {
            assert!((progress - 0.6).abs() < 1e-6);
        }
        _ => panic!(),
    }
    assert!(world.resource::<DirtyAgents>().0.contains(&entity));
}
#[test]
fn walk_advance_inserts_near_stop_marker_when_progress_saturates() {
    use crate::ids::AgentId;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:1".into(),
                progress: 0.99,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.05),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(walk_advance_system);
    schedule.run(&mut world);
    assert!(
        world.get::<NearStop>(entity).is_some(),
        "walk_advance should add NearStop when progress saturates to 1.0"
    );
}
#[test]
fn stop_arrival_removes_near_stop_marker_after_transition() {
    use crate::ids::AgentId;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(DirtyAgents::default());
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:1".into(),
                progress: 1.0,
            }),
            WalkPlan {
                stages: vec![PlanStage::WalkToStop {
                    link_id: "l:1".into(),
                    stop_id: "s:1".into(),
                }],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.05),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            NearStop,
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(stop_arrival_system);
    schedule.run(&mut world);
    assert!(
        world.get::<NearStop>(entity).is_none(),
        "stop_arrival should remove NearStop after state transition"
    );
}
#[test]
fn current_link_polyline_invalidates_on_walker_link_change() {
    use crate::ids::AgentId;
    use std::sync::Arc;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let entity = world
        .spawn((
            AgentMarker,
            StableAgentId(AgentId("a:1".into())),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l:a".into(),
                progress: 0.0,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.05),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            CurrentLinkPolyline {
                link_id: "l:a".into(),
                points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
            },
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(update_link_polyline_cache_system);
    // Tick 1: cache already matches → no change.
    schedule.run(&mut world);
    assert_eq!(
        world
            .get::<CurrentLinkPolyline>(entity)
            .unwrap()
            .link_id
            .as_str(),
        "l:a"
    );
    // Mutate the agent to a different link.
    if let Some(mut s) = world.get_mut::<AgentMobilityStateComponent>(entity) {
        s.0 = AgentMobilityState::Walking {
            link_id: "l:b".into(),
            progress: 0.0,
        };
    }
    schedule.run(&mut world);
    assert_eq!(
        world
            .get::<CurrentLinkPolyline>(entity)
            .unwrap()
            .link_id
            .as_str(),
        "l:b"
    );
    let cached = world.get::<CurrentLinkPolyline>(entity).unwrap();
    assert_eq!(cached.points.as_ref(), &vec![(0.0, 0.0), (0.0, 10.0)]);
}
#[test]
fn current_link_polyline_invalidates_on_vehicle_link_change() {
    use crate::ids::VehicleId;
    use crate::mobility::records::VehicleKind;
    use std::sync::Arc;
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(all_active());
    insert_test_routing(&mut world);
    let route_id = crate::routing::TrafficRouteId(0);
    let entity = world
        .spawn((
            VehicleMarker,
            StableVehicleId(VehicleId("v:1".into())),
            VehicleKindComponent(VehicleKind::Car),
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 0.0,
                speed: 0.1,
            },
            Capacity(1),
            Occupants(vec![]),
            DwellTicksRemaining(0),
            Position { x: 0.0, y: 0.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
            CurrentLinkPolyline {
                link_id: "l:a".into(),
                points: Arc::new(vec![(0.0, 0.0), (10.0, 0.0)]),
            },
        ))
        .id();
    let mut schedule = Schedule::default();
    schedule.add_systems(update_link_polyline_cache_system);
    if let Some(mut rp) = world.get_mut::<RoutePosition>(entity) {
        rp.edge_index = 1;
    }
    schedule.run(&mut world);
    assert_eq!(
        world
            .get::<CurrentLinkPolyline>(entity)
            .unwrap()
            .link_id
            .as_str(),
        "l:b"
    );
}
#[test]
fn incremental_chunk_populations_matches_full_rebuild() {
    use crate::ids::AgentId;
    use crate::mobility::resources::{PreviousChunkByEntity, PreviousFlowCellContrib};
    let mut world = World::new();
    insert_test_routing(&mut world);
    world.insert_resource(FlowCells::default());
    world.insert_resource(ChunkPopulations::default());
    world.insert_resource(AgentsByChunk::default());
    world.insert_resource(VehiclesByChunk::default());
    world.insert_resource(PreviousChunkByEntity::default());
    world.insert_resource(PreviousFlowCellContrib::default());
    // Spawn 200 agents scattered across multiple chunks.
    for i in 0..200 {
        let x = (i % 10) as f32 * 35.0; // chunks at 0, 35, 70, ...
        let y = (i / 10) as f32 * 35.0;
        world.spawn((
            AgentMarker,
            StableAgentId(AgentId(format!("a:{i}"))),
            AgentMobilityStateComponent(AgentMobilityState::Walking {
                link_id: "l".into(),
                progress: 0.0,
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.05),
            Position { x, y },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey(String::new()),
        ));
    }
    let mut schedule = Schedule::default();
    schedule.add_systems(track_chunk_populations_system);
    // Tick 1: full rebuild path (all positions are "new").
    schedule.run(&mut world);
    // Mutate one agent's position to a new chunk.
    let mut q = world.query::<(Entity, &mut Position)>();
    let moved_entity = q
        .iter_mut(&mut world)
        .next()
        .map(|(e, mut p)| {
            p.x = 999.0;
            p.y = 999.0;
            e
        })
        .unwrap();
    // Tick 2: incremental path.
    schedule.run(&mut world);
    let after2_incremental: std::collections::HashMap<crate::ids::ChunkCoord, HashSet<Entity>> =
        world
            .resource::<AgentsByChunk>()
            .0
            .iter()
            .map(|(c, e)| (*c, e.clone()))
            .collect();
    // Compare against a fresh full rebuild from query state.
    let mut reference: std::collections::HashMap<crate::ids::ChunkCoord, HashSet<Entity>> =
        std::collections::HashMap::new();
    let mut q2 = world.query::<(Entity, &Position, &AgentMarker)>();
    for (entity, pos, _) in q2.iter(&world) {
        let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
        reference.entry(chunk).or_default().insert(entity);
    }
    assert_eq!(after2_incremental, reference);
    // Ensure the moved entity actually moved buckets.
    assert!(
        after2_incremental
            .values()
            .any(|v| v.contains(&moved_entity))
    );
}
#[test]
fn spawned_agent_carries_sex_and_parent() {
    use crate::mobility::components::Sex;
    let (mut world, _s) = crate::mobility::api::empty_world_and_schedule();
    let mut rec = crate::mobility::records::AgentRecord::new_born_at(
        crate::ids::AgentId("agent:f".into()),
        crate::mobility::records::AgentMobilityState::AtActivity {
            activity_id: "a".into(),
        },
        vec![crate::mobility::records::PlanStage::Activity {
            activity_id: "a".into(),
        }],
        0.05,
        0,
    );
    rec.sex = Sex::Female;
    rec.parent_id = Some(crate::ids::AgentId("agent:mum".into()));
    let e = crate::mobility::api::spawn_agent_from_record_at_position(&mut world, rec, (0.0, 0.0));
    assert_eq!(*world.get::<Sex>(e).unwrap(), Sex::Female);
}
#[test]
fn cyclic_plan_cursor_wraps_to_zero_at_end() {
    use crate::mobility::components::WalkPlan;
    use crate::mobility::records::PlanStage;
    let stages = vec![
        PlanStage::Activity {
            activity_id: "a".into(),
        },
        PlanStage::Activity {
            activity_id: "b".into(),
        },
    ];
    let mut p = WalkPlan {
        stages: stages.clone(),
        cursor: 1,
        cyclic: true,
    };
    crate::mobility::systems::advance_cursor(&mut p);
    assert_eq!(p.cursor, 0);
    let mut q = WalkPlan {
        stages,
        cursor: 1,
        cyclic: false,
    };
    crate::mobility::systems::advance_cursor(&mut q);
    assert_eq!(q.cursor, 2);
}
#[test]
fn spawned_agent_carries_birth_tick_and_ages() {
    use crate::mobility::components::BirthTick;
    use crate::time::SimClock;
    let (mut world, _schedule) = crate::mobility::api::empty_world_and_schedule();

    let rec = crate::mobility::records::AgentRecord::new_born_at(
        crate::ids::AgentId("agent:test".into()),
        crate::mobility::records::AgentMobilityState::AtActivity {
            activity_id: "a".into(),
        },
        vec![crate::mobility::records::PlanStage::Activity {
            activity_id: "a".into(),
        }],
        0.05,
        100,
    );
    let entity =
        crate::mobility::api::spawn_agent_from_record_at_position(&mut world, rec, (0.0, 0.0));
    assert_eq!(world.get::<BirthTick>(entity).unwrap().0, 100);

    let clock = SimClock {
        sim_seconds_per_tick: 200,
    };
    assert!((clock.age_years(100 + 157_680, 100) - 1.0).abs() < 1e-3);
}

#[test]
fn trader_agent_world_coord_reads_position_verbatim() {
    use crate::ids::AgentId;
    use crate::mobility::api::world_coord_for_agent;
    use crate::mobility::components::{
        AgentMobilityStateComponent, BirthTick, Direction, Position, SpriteKey, StableAgentId,
        TraderAgent, WalkPlan, WalkSpeed,
    };
    use crate::mobility::records::AgentMobilityState;
    use crate::mobility::resources::AgentIdIndex;

    let mut world = bevy_ecs::world::World::new();
    world.insert_resource(AgentIdIndex::default());
    let id = AgentId("trader:1".to_string());
    let entity = world
        .spawn((
            TraderAgent,
            StableAgentId(id.clone()),
            AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                activity_id: "trader".to_string(),
            }),
            WalkPlan {
                stages: vec![],
                cursor: 0,
                cyclic: false,
            },
            WalkSpeed(0.0),
            BirthTick(0),
            Position { x: 12.5, y: 34.0 },
            Direction(abutown_protocol::DirectionDto::S),
            SpriteKey("trader:3".to_string()),
        ))
        .id();
    world
        .resource_mut::<AgentIdIndex>()
        .0
        .insert(id.clone(), entity);

    assert_eq!(world_coord_for_agent(&world, &id), Some((12.5, 34.0)));
}
