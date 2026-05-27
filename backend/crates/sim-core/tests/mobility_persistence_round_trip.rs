use std::collections::HashMap;

use sim_core::ids::AgentId;
use sim_core::mobility::{
    AgentMobilityState, AgentRecord, MobilityPersistSnapshot, PersistedActiveRoute,
    PersistedRouteStep, PlanStage, api, apply_into_world, extract_from_world,
};
use sim_core::routing::{
    Edge, EdgeId, EdgeKind, Graph, ModeState, Node, NodeId, NodeKind, RoutingProfileKey,
};

#[test]
fn phase3_snapshot_round_trips_byte_for_byte() {
    let fixture = include_str!("fixtures/phase3-mobility-snapshot.json");

    // Parse fixture → persist snapshot, hydrate into a real World, then
    // re-extract for the byte comparison. This exercises the full
    // World→snapshot→World round trip the persistence path takes.
    let snap: MobilityPersistSnapshot = serde_json::from_str(fixture)
        .expect("phase3 fixture should deserialize into ECS MobilityPersistSnapshot");

    let (mut world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut world, snap);
    let reloaded = extract_from_world(&world);

    let reserialized =
        serde_json::to_string_pretty(&reloaded).expect("re-serialize should not fail");

    let fixture_value: serde_json::Value =
        serde_json::from_str(fixture).expect("fixture is valid JSON");
    let reserialized_value: serde_json::Value =
        serde_json::from_str(&reserialized).expect("our re-serialized output is valid JSON");

    // Byte-identical round trip: every top-level key in the
    // (serialized) round-tripped value must match the fixture, and vice
    // versa. The fixture is the canonical persistence shape — if a new
    // top-level field is added to `MobilityPersistSnapshot`, the fixture
    // must be extended too, and this assertion catches the drift.
    assert_eq!(
        fixture_value, reserialized_value,
        "round-trip diverged from fixture",
    );
}

fn active_route_snapshot() -> MobilityPersistSnapshot {
    let mut agents = HashMap::new();
    agents.insert(
        AgentId("agent:active-route".to_string()),
        AgentRecord {
            id: AgentId("agent:active-route".to_string()),
            state: AgentMobilityState::AtActivity {
                activity_id: "activity:home".to_string(),
            },
            plan: vec![PlanStage::Activity {
                activity_id: "activity:home".to_string(),
            }],
            plan_cursor: 0,
            walk_speed_per_tick: 1.0,
            active_route: Some(PersistedActiveRoute {
                destination_node: 1,
                profile: RoutingProfileKey::Walk,
                cursor: 0,
                steps: vec![PersistedRouteStep {
                    edge_id: 0,
                    mode: ModeState::Walking,
                    canonical_edge_key: "link:future-only".to_string(),
                    length: 8.0,
                }],
            }),
        },
    );

    MobilityPersistSnapshot {
        tick: 7,
        agents,
        vehicles: HashMap::new(),
        stops: HashMap::new(),
        routes: HashMap::new(),
        link_polylines: HashMap::from([(
            "link:future-only".to_string(),
            vec![(10.0, 10.0), (18.0, 10.0)],
        )]),
        flow_cells: HashMap::new(),
        chunk_activities: HashMap::new(),
    }
}

#[test]
fn active_route_round_trip_preserves_valid_graph_ids() {
    let snap = active_route_snapshot();
    let (mut world, _schedule) = api::empty_world_and_schedule();

    apply_into_world(&mut world, snap);
    let extracted = extract_from_world(&world);
    let route = extracted
        .agents
        .get(&AgentId("agent:active-route".to_string()))
        .and_then(|agent| agent.active_route.as_ref())
        .expect("active route persists");

    assert_eq!(route.profile, RoutingProfileKey::Walk);
    assert_eq!(route.cursor, 0);
    assert_eq!(route.destination_node, 1);
    assert_eq!(route.steps.len(), 1);
    assert_eq!(route.steps[0].edge_id, 0);
    assert_eq!(route.steps[0].canonical_edge_key, "link:future-only");
    assert_eq!(route.steps[0].mode, ModeState::Walking);
    assert_eq!(route.steps[0].length, 8.0);
    assert_eq!(
        extracted
            .link_polylines
            .get("link:future-only")
            .expect("future active-route link is persisted"),
        &vec![(10.0, 10.0), (18.0, 10.0)]
    );

    let (mut reloaded_world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut reloaded_world, extracted.clone());
    assert_eq!(extract_from_world(&reloaded_world), extracted);
}

fn multi_step_active_route_snapshot() -> MobilityPersistSnapshot {
    let mut agents = HashMap::new();
    agents.insert(
        AgentId("agent:multi-step-active-route".to_string()),
        AgentRecord {
            id: AgentId("agent:multi-step-active-route".to_string()),
            state: AgentMobilityState::Walking {
                link_id: "link:active:a".to_string(),
                progress: 0.25,
            },
            plan: vec![PlanStage::WalkToActivity {
                link_id: "link:active:a".to_string(),
                activity_id: "activity:home".to_string(),
            }],
            plan_cursor: 0,
            walk_speed_per_tick: 1.0,
            active_route: Some(PersistedActiveRoute {
                destination_node: 2,
                profile: RoutingProfileKey::Walk,
                cursor: 0,
                steps: vec![
                    PersistedRouteStep {
                        edge_id: 100,
                        mode: ModeState::Walking,
                        canonical_edge_key: "link:active:a".to_string(),
                        length: 5.0,
                    },
                    PersistedRouteStep {
                        edge_id: 101,
                        mode: ModeState::Walking,
                        canonical_edge_key: "link:active:b".to_string(),
                        length: 8.0,
                    },
                ],
            }),
        },
    );

    MobilityPersistSnapshot {
        tick: 9,
        agents,
        vehicles: HashMap::new(),
        stops: HashMap::new(),
        routes: HashMap::new(),
        link_polylines: HashMap::from([
            (
                "link:active:a".to_string(),
                vec![(0.0, 0.0), (5.0, 0.0)],
            ),
            (
                "link:active:b".to_string(),
                vec![(5.0, 0.0), (13.0, 0.0)],
            ),
        ]),
        flow_cells: HashMap::new(),
        chunk_activities: HashMap::new(),
    }
}

#[test]
fn multi_step_active_route_round_trips_through_shared_endpoints() {
    let snap = multi_step_active_route_snapshot();
    let (mut world, _schedule) = api::empty_world_and_schedule();

    apply_into_world(&mut world, snap);
    let extracted = extract_from_world(&world);
    let route = extracted
        .agents
        .get(&AgentId("agent:multi-step-active-route".to_string()))
        .and_then(|agent| agent.active_route.as_ref())
        .expect("active route persists");

    assert_eq!(route.destination_node, 2);
    assert_eq!(route.steps.len(), 2);
    assert_eq!(route.steps[0].edge_id, 0);
    assert_eq!(route.steps[1].edge_id, 1);
    assert_eq!(route.steps[0].canonical_edge_key, "link:active:a");
    assert_eq!(route.steps[1].canonical_edge_key, "link:active:b");

    let (mut reloaded_world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut reloaded_world, extracted);
}

fn graph_native_active_route_world() -> bevy_ecs::world::World {
    let (mut world, _schedule) = api::empty_world_and_schedule();
    world.insert_resource(Graph::new(
        vec![
            Node {
                id: NodeId(0),
                position: (0.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (5.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(2),
                position: (13.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ],
        vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (5.0, 0.0)],
                length: 5.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 16,
                legacy_id: None,
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(1),
                to: NodeId(2),
                polyline: vec![(5.0, 0.0), (13.0, 0.0)],
                length: 8.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 16,
                legacy_id: None,
            },
        ],
    ));
    api::spawn_agent_from_record(
        &mut world,
        AgentRecord {
            id: AgentId("agent:graph-native-active-route".to_string()),
            state: AgentMobilityState::AtActivity {
                activity_id: "activity:home".to_string(),
            },
            plan: vec![PlanStage::Activity {
                activity_id: "activity:home".to_string(),
            }],
            plan_cursor: 0,
                    walk_speed_per_tick: 1.0,
                    active_route: Some(PersistedActiveRoute {
                        destination_node: 1,
                        profile: RoutingProfileKey::Walk,
                        cursor: 0,
                        steps: vec![PersistedRouteStep {
                            edge_id: 0,
                            mode: ModeState::Walking,
                            canonical_edge_key: "edge:0".to_string(),
                            length: 5.0,
                        }],
                    }),
                },
            );
    world
}

fn edge_canonical_key(edge: &Edge) -> String {
    edge.legacy_id
        .clone()
        .unwrap_or_else(|| format!("edge:{}", edge.id.0))
}

#[test]
fn graph_native_active_route_edge_key_round_trips_with_matching_raw_id() {
    let world = graph_native_active_route_world();

    let extracted = extract_from_world(&world);
    let extracted_route = extracted
        .agents
        .get(&AgentId("agent:graph-native-active-route".to_string()))
        .and_then(|agent| agent.active_route.as_ref())
        .expect("active route persists from live graph");
    assert_eq!(extracted_route.steps[0].edge_id, 0);
    assert_eq!(extracted_route.steps[0].canonical_edge_key, "edge:0");
    assert_eq!(
        extracted
            .link_polylines
            .get("edge:0")
            .expect("first graph-native active-route polyline is persisted"),
        &vec![(0.0, 0.0), (5.0, 0.0)]
    );

    let (mut reloaded_world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut reloaded_world, extracted);
    let reloaded = extract_from_world(&reloaded_world);
    let reloaded_route = reloaded
        .agents
        .get(&AgentId("agent:graph-native-active-route".to_string()))
        .and_then(|agent| agent.active_route.as_ref())
        .expect("active route survives graph rebuild");
    let reloaded_step = &reloaded_route.steps[0];
    assert_eq!(reloaded_step.canonical_edge_key, "edge:0");
    assert_eq!(reloaded_step.edge_id, 0);

    let graph = reloaded_world.resource::<Graph>();
    let normalized_edge = graph.edge(EdgeId(reloaded_step.edge_id));
    assert_eq!(edge_canonical_key(normalized_edge), "edge:0");
}

#[test]
#[should_panic(expected = "no graph polyline available for persisted link edge:999")]
fn active_route_hydration_rejects_missing_canonical_key() {
    let mut snap = active_route_snapshot();
    snap.agents
        .get_mut(&AgentId("agent:active-route".to_string()))
        .unwrap()
        .active_route
        .as_mut()
        .unwrap()
        .steps[0]
        .canonical_edge_key = "edge:999".to_string();

    let (mut world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut world, snap);
}

#[test]
fn active_route_hydration_normalizes_transient_destination_node() {
    let mut snap = active_route_snapshot();
    snap.agents
        .get_mut(&AgentId("agent:active-route".to_string()))
        .unwrap()
        .active_route
        .as_mut()
        .unwrap()
        .destination_node = 999;

    let (mut world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut world, snap);
    let extracted = extract_from_world(&world);
    let route = extracted
        .agents
        .get(&AgentId("agent:active-route".to_string()))
        .and_then(|agent| agent.active_route.as_ref())
        .expect("active route persists");
    assert_eq!(route.destination_node, 1);
}

#[test]
#[should_panic(expected = "cannot traverse TramTrack from Intersection with profile WalkTransit")]
fn active_route_hydration_rejects_walk_transit_boarding_away_from_stop() {
    let mut snap = active_route_snapshot();
    let agent = snap
        .agents
        .get_mut(&AgentId("agent:active-route".to_string()))
        .unwrap();
    let route = agent.active_route.as_mut().unwrap();
    route.profile = RoutingProfileKey::WalkTransit;
    route.steps[0].mode = ModeState::OnTram;

    let (mut world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut world, snap);
}

#[test]
#[should_panic(expected = "persisted active_route cursor 1 is outside 1 steps")]
fn active_route_hydration_rejects_cursor_past_steps() {
    let mut snap = active_route_snapshot();
    snap.agents
        .get_mut(&AgentId("agent:active-route".to_string()))
        .unwrap()
        .active_route
        .as_mut()
        .unwrap()
        .cursor = 1;

    let (mut world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut world, snap);
}
