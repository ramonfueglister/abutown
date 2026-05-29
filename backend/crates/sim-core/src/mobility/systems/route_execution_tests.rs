use super::*;
use crate::ids::AgentId;
use crate::mobility::records::{AgentMobilityState, AgentRecord, PlanStage};
use crate::routing::{
    Edge, EdgeId, EdgeKind, FlowFieldCache, Graph, HpaConfig, HpaIndex, Node, NodeId, NodeKind,
    NodeSpatialIndex,
};
fn route_graph(activity_legacy_id: Option<&str>) -> Graph {
    route_graph_with_edge_legacy(activity_legacy_id, true)
}
fn graph_native_route_graph(activity_legacy_id: Option<&str>) -> Graph {
    route_graph_with_edge_legacy(activity_legacy_id, false)
}
fn route_graph_with_edge_legacy(activity_legacy_id: Option<&str>, edge_legacy: bool) -> Graph {
    Graph::new(
        vec![
            Node {
                id: NodeId(0),
                position: (0.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (1.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (2.0, 0.0),
                kind: NodeKind::ActivityLocation,
                legacy_id: activity_legacy_id.map(str::to_string),
            },
        ],
        vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (1.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: edge_legacy.then(|| "walk:a".into()),
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(1),
                to: NodeId(2),
                polyline: vec![(1.0, 0.0), (2.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: edge_legacy.then(|| "walk:b".into()),
            },
        ],
    )
}
fn intermediate_cluster_route_graph() -> Graph {
    Graph::new(
        vec![
            Node {
                id: NodeId(0),
                position: (0.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (12.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (25.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(3),
                position: (35.0, 0.0),
                kind: NodeKind::ActivityLocation,
                legacy_id: Some("activity:far".into()),
            },
        ],
        vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (12.0, 0.0)],
                length: 12.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:0".into()),
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(1),
                to: NodeId(2),
                polyline: vec![(12.0, 0.0), (25.0, 0.0)],
                length: 13.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:1".into()),
            },
            Edge {
                id: EdgeId(2),
                from: NodeId(2),
                to: NodeId(3),
                polyline: vec![(25.0, 0.0), (35.0, 0.0)],
                length: 10.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:2".into()),
            },
        ],
    )
}
fn outgoing_from_destination_graph() -> Graph {
    Graph::new(
        vec![
            Node {
                id: NodeId(0),
                position: (0.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (1.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (2.0, 0.0),
                kind: NodeKind::ActivityLocation,
                legacy_id: Some("activity:work".into()),
            },
            Node {
                id: NodeId(3),
                position: (3.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ],
        vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (1.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:a".into()),
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(1),
                to: NodeId(2),
                polyline: vec![(1.0, 0.0), (2.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:b".into()),
            },
            Edge {
                id: EdgeId(2),
                from: NodeId(2),
                to: NodeId(3),
                polyline: vec![(2.0, 0.0), (3.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: Some("walk:out".into()),
            },
        ],
    )
}
fn world_schedule_and_agent() -> (World, Schedule, Entity) {
    world_schedule_and_agent_with_activity_legacy(Some("activity:work"))
}
fn world_schedule_and_agent_without_activity_legacy() -> (World, Schedule, Entity) {
    world_schedule_and_agent_with_activity_legacy(None)
}
fn world_schedule_and_agent_with_activity_legacy(
    activity_legacy_id: Option<&str>,
) -> (World, Schedule, Entity) {
    world_schedule_and_agent_with_graph(
        route_graph(activity_legacy_id),
        "walk:a",
        "activity:work",
        1.0,
    )
}
fn world_schedule_and_agent_with_graph(
    graph: Graph,
    initial_link_id: &str,
    activity_id: &str,
    walk_speed: f32,
) -> (World, Schedule, Entity) {
    let (mut world, schedule) = crate::mobility::api::empty_world_and_schedule();
    let hpa = HpaIndex::build(&graph, HpaConfig::default()).expect("HPA should build");
    let spatial = NodeSpatialIndex::from_nodes(graph.nodes());
    world.insert_resource(graph);
    world.insert_resource(hpa);
    world.insert_resource(spatial);
    world.insert_resource(FlowFieldCache::default());
    let active_coord = crate::ids::ChunkCoord { x: 0, y: 0 };
    let chunk_entity = world
        .spawn((
            ChunkCoordComp(active_coord),
            ActiveChunk,
            crate::world::components::ChunkSubscriberCount(1),
            crate::world::components::LodCooldown(0),
        ))
        .id();
    world
        .resource_mut::<crate::world::resources::ChunksByCoord>()
        .0
        .insert(active_coord, chunk_entity);
    let entity = crate::mobility::api::spawn_agent_from_record(
        &mut world,
        AgentRecord::new(
            AgentId("agent:route".into()),
            AgentMobilityState::Walking {
                link_id: initial_link_id.into(),
                progress: 0.0,
            },
            vec![PlanStage::WalkToActivity {
                link_id: initial_link_id.into(),
                activity_id: activity_id.into(),
            }],
            walk_speed,
        ),
    );
    (world, schedule, entity)
}
fn world_schedule_and_agent_requiring_intermediate_corridor() -> (World, Schedule, Entity) {
    let graph = intermediate_cluster_route_graph();
    let (mut world, schedule) = crate::mobility::api::empty_world_and_schedule();
    let hpa = HpaIndex::build(
        &graph,
        HpaConfig {
            cluster_size_tiles: 10,
            corridor_margin_clusters: 0,
        },
    )
    .expect("HPA should build");
    let spatial = NodeSpatialIndex::from_nodes(graph.nodes());
    world.insert_resource(graph);
    world.insert_resource(hpa);
    world.insert_resource(spatial);
    world.insert_resource(FlowFieldCache::default());
    let active_coord = crate::ids::ChunkCoord { x: 0, y: 0 };
    let chunk_entity = world
        .spawn((
            ChunkCoordComp(active_coord),
            ActiveChunk,
            crate::world::components::ChunkSubscriberCount(1),
            crate::world::components::LodCooldown(0),
        ))
        .id();
    world
        .resource_mut::<crate::world::resources::ChunksByCoord>()
        .0
        .insert(active_coord, chunk_entity);
    let entity = crate::mobility::api::spawn_agent_from_record(
        &mut world,
        AgentRecord::new(
            AgentId("agent:route".into()),
            AgentMobilityState::Walking {
                link_id: "walk:0".into(),
                progress: 0.0,
            },
            vec![PlanStage::WalkToActivity {
                link_id: "walk:0".into(),
                activity_id: "activity:far".into(),
            }],
            0.0,
        ),
    );
    (world, schedule, entity)
}
#[test]
fn route_assignment_inserts_active_route() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    schedule.run(&mut world);
    let route = world
        .get::<ActiveRoute>(entity)
        .expect("route assignment should insert ActiveRoute");
    assert_eq!(route.destination, NodeId(2));
    assert_eq!(route.steps.len(), 2);
    assert_eq!(route.steps[0].canonical_edge_key, "walk:a");
    assert_eq!(route.steps[1].canonical_edge_key, "walk:b");
    assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
}
#[test]
fn route_assignment_uses_full_hpa_corridor() {
    let (mut world, mut schedule, entity) =
        world_schedule_and_agent_requiring_intermediate_corridor();
    schedule.run(&mut world);
    let route = world
        .get::<ActiveRoute>(entity)
        .expect("route assignment should include intermediate corridor clusters");
    assert_eq!(route.destination, NodeId(3));
    assert_eq!(route.steps.len(), 3);
    assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 0);
}

#[test]
fn route_assignment_activity_walker_continues_on_connected_footway() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent_with_graph(
        outgoing_from_destination_graph(),
        "walk:b",
        "activity:work",
        0.0,
    );
    world.get_mut::<WalkPlan>(entity).unwrap().stages = vec![PlanStage::Activity {
        activity_id: "activity:wander".into(),
    }];
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:b".into(),
        progress: 1.0,
    };

    schedule.run(&mut world);

    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:out" && *progress == 0.0
    ));
    assert!(
        world.resource::<DirtyAgents>().0.contains(&entity),
        "link switch must be published in the mobility delta"
    );
}

#[test]
fn route_assignment_and_advance_accept_graph_native_edge_keys() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent_with_graph(
        graph_native_route_graph(Some("activity:work")),
        "edge:0",
        "activity:work",
        1.0,
    );
    schedule.run(&mut world);
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    schedule.run(&mut world);
    let route = world
        .get::<ActiveRoute>(entity)
        .expect("graph-native route should remain active on second edge");
    assert_eq!(route.cursor, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "edge:1" && *progress == 0.0
    ));
}
#[test]
fn route_assignment_syncs_completed_initial_edge_to_first_step() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:a".into(),
        progress: 1.0,
    };
    schedule.run(&mut world);
    let route = world
        .get::<ActiveRoute>(entity)
        .expect("route assignment should insert ActiveRoute from completed edge endpoint");
    assert_eq!(route.cursor, 0);
    assert_eq!(route.steps.len(), 1);
    assert_eq!(route.steps[0].canonical_edge_key, "walk:b");
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:b" && *progress == 0.0
    ));
}
#[test]
fn route_assignment_completes_when_origin_is_destination() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:b".into(),
        progress: 1.0,
    };
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::AtActivity { activity_id } if activity_id == "activity:work"
    ));
}
#[test]
fn route_assignment_does_not_complete_mid_edge_from_destination() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent_with_graph(
        outgoing_from_destination_graph(),
        "walk:out",
        "activity:work",
        0.0,
    );
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:out".into(),
        progress: 0.5,
    };
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:out" && *progress == 0.5
    ));
}
#[test]
fn route_assignment_resolves_activity_destination_through_spatial_index() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent_without_activity_legacy();
    schedule.run(&mut world);
    let route = world
        .get::<ActiveRoute>(entity)
        .expect("route assignment should use activity geometry and spatial index");
    assert_eq!(route.destination, NodeId(2));
    assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 0);
}
#[test]
fn route_assignment_counts_unresolved_destination_as_failed() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent_without_activity_legacy();
    world.remove_resource::<NodeSpatialIndex>();
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    let stats = world.resource::<RouteAssignmentStats>();
    assert_eq!(stats.assigned, 0);
    assert_eq!(stats.skipped, 0);
    assert_eq!(stats.failed, 1);
}
#[test]
fn route_advance_crosses_edges_before_finishing_plan() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    schedule.run(&mut world);
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    schedule.run(&mut world);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    match &state.0 {
        AgentMobilityState::Walking { link_id, progress } => {
            assert_eq!(link_id, "walk:b");
            assert_eq!(*progress, 0.0);
        }
        other => panic!("expected walking on second edge, got {other:?}"),
    }
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
    assert_eq!(world.get::<ActiveRoute>(entity).unwrap().cursor, 1);
}
#[test]
fn route_advance_completes_final_activity_route() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:b".into(),
        progress: 1.0,
    };
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(2),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![RouteStep {
            edge_id: EdgeId(1),
            mode: crate::routing::ModeState::Walking,
            canonical_edge_key: "walk:b".into(),
            length: 1.0,
        }],
        cursor: 0,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::AtActivity { activity_id } if activity_id == "activity:work"
    ));
}
#[test]
fn route_advance_completes_final_stop_route() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world.get_mut::<WalkPlan>(entity).unwrap().stages = vec![PlanStage::WalkToStop {
        link_id: "walk:b".into(),
        stop_id: "activity:work".into(),
    }];
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:b".into(),
        progress: 1.0,
    };
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(2),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![RouteStep {
            edge_id: EdgeId(1),
            mode: crate::routing::ModeState::Walking,
            canonical_edge_key: "walk:b".into(),
            length: 1.0,
        }],
        cursor: 0,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::WaitingAtStop { stop_id } if stop_id == "activity:work"
    ));
    assert!(
        world
            .resource::<crate::routing::WaitingAgents>()
            .queue(NodeId(2))
            .is_some_and(|queue| queue.contains(&AgentId("agent:route".into())))
    );
}
#[test]
fn route_advance_invalidates_unexpected_current_stage() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world.get_mut::<WalkPlan>(entity).unwrap().stages = vec![PlanStage::Activity {
        activity_id: "activity:work".into(),
    }];
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:a".into(),
        progress: 1.0,
    };
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(2),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![RouteStep {
            edge_id: EdgeId(0),
            mode: crate::routing::ModeState::Walking,
            canonical_edge_key: "walk:a".into(),
            length: 1.0,
        }],
        cursor: 0,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:a" && *progress >= 1.0
    ));
}
#[test]
fn route_advance_invalidates_stale_current_link() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    schedule.run(&mut world);
    world.get_mut::<WalkSpeed>(entity).unwrap().0 = 0.0;
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:b".into(),
        progress: 1.0,
    };
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:b" && *progress >= 1.0
    ));
}
#[test]
fn route_advance_invalidates_disconnected_next_step() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:a".into(),
        progress: 1.0,
    };
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(2),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![
            RouteStep {
                edge_id: EdgeId(0),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:a".into(),
                length: 1.0,
            },
            RouteStep {
                edge_id: EdgeId(0),
                mode: crate::routing::ModeState::Walking,
                canonical_edge_key: "walk:a".into(),
                length: 1.0,
            },
        ],
        cursor: 0,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:a" && *progress >= 1.0
    ));
}
#[test]
fn route_advance_invalidates_cursor_past_steps() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(2),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![RouteStep {
            edge_id: EdgeId(0),
            mode: crate::routing::ModeState::Walking,
            canonical_edge_key: "walk:a".into(),
            length: 1.0,
        }],
        cursor: 1,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
}
#[test]
fn route_advance_invalidates_when_final_edge_misses_destination() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:a".into(),
        progress: 1.0,
    };
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(2),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![RouteStep {
            edge_id: EdgeId(0),
            mode: crate::routing::ModeState::Walking,
            canonical_edge_key: "walk:a".into(),
            length: 1.0,
        }],
        cursor: 0,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:a" && *progress >= 1.0
    ));
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
}
#[test]
fn route_advance_invalidates_when_stage_destination_mismatches_route() {
    let (mut world, mut schedule, entity) = world_schedule_and_agent();
    world
        .get_mut::<AgentMobilityStateComponent>(entity)
        .unwrap()
        .0 = AgentMobilityState::Walking {
        link_id: "walk:a".into(),
        progress: 1.0,
    };
    world.entity_mut(entity).insert(ActiveRoute {
        destination: NodeId(1),
        profile: crate::routing::RoutingProfileKey::Walk,
        steps: vec![RouteStep {
            edge_id: EdgeId(0),
            mode: crate::routing::ModeState::Walking,
            canonical_edge_key: "walk:a".into(),
            length: 1.0,
        }],
        cursor: 0,
    });
    schedule.run(&mut world);
    assert!(world.get::<ActiveRoute>(entity).is_none());
    assert_eq!(world.resource::<RouteAssignmentStats>().failed, 1);
    let state = world.get::<AgentMobilityStateComponent>(entity).unwrap();
    assert!(matches!(
        &state.0,
        AgentMobilityState::Walking { link_id, progress }
            if link_id == "walk:a" && *progress >= 1.0
    ));
    assert_eq!(world.get::<WalkPlan>(entity).unwrap().cursor, 0);
}
