use std::collections::HashMap;

use sim_core::ids::AgentId;
use sim_core::mobility::{
    AgentMobilityState, AgentRecord, MobilityPersistSnapshot, PersistedActiveRoute,
    PersistedRouteStep, PlanStage, api, apply_into_world, extract_from_world,
};
use sim_core::routing::{
    Edge, EdgeId, EdgeKind, Graph, ModeState, Node, NodeId, NodeKind, RoutingProfileKey,
};

// NOTE: the `phase3_snapshot_round_trips_byte_for_byte` test and its
// `fixtures/phase3-mobility-snapshot.json` were retired on 2026-05-29. That
// fixture was a pre-tram-retirement transit snapshot (tram vehicles, stops,
// transit routes, ride-to-stop agent plans). Trams are gone from the runtime
// (`VehicleKind` is car-only and hydration rejects retired tram modes), so the
// fixture can no longer deserialize, let alone round-trip byte-for-byte. The
// live car/walk persistence round-trip is covered by the `active_route_*`
// tests below; tram rejection is covered by
// `active_route_hydration_rejects_retired_tram_mode`.

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
            birth_tick: 0,
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
            birth_tick: 0,
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
            ("link:active:a".to_string(), vec![(0.0, 0.0), (5.0, 0.0)]),
            ("link:active:b".to_string(), vec![(5.0, 0.0), (13.0, 0.0)]),
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
            birth_tick: 0,
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
#[should_panic(expected = "retired tram mode")]
fn active_route_hydration_rejects_retired_tram_mode() {
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

#[test]
fn birth_tick_round_trips() {
    // Spawn an agent with birth_tick = 4242, extract → JSON → deserialize,
    // assert the persisted record carries birth_tick. Then re-apply into a
    // fresh world and confirm the live entity still has birth_tick == 4242.
    let (mut world, _schedule) = api::empty_world_and_schedule();
    let rec = AgentRecord::new_born_at(
        AgentId("agent:born".into()),
        AgentMobilityState::AtActivity {
            activity_id: "activity:home".into(),
        },
        vec![PlanStage::Activity {
            activity_id: "activity:home".into(),
        }],
        0.05,
        4242,
    );
    api::spawn_agent_from_record(&mut world, rec);

    let snap = extract_from_world(&world);
    let json = serde_json::to_string(&snap).unwrap();
    let back: MobilityPersistSnapshot = serde_json::from_str(&json).unwrap();

    // The persisted record must carry birth_tick through JSON serialization.
    let agent = back
        .agents
        .get(&AgentId("agent:born".into()))
        .expect("agent was persisted in snapshot");
    assert_eq!(agent.birth_tick, 4242);

    // Re-applying into a fresh world must preserve birth_tick on the live entity.
    let (mut w2, _s2) = api::empty_world_and_schedule();
    apply_into_world(&mut w2, back);
    let snap2 = extract_from_world(&w2);
    assert_eq!(
        snap2
            .agents
            .get(&AgentId("agent:born".into()))
            .expect("agent present after re-apply")
            .birth_tick,
        4242,
        "birth_tick must survive extract → JSON → apply round-trip"
    );
}
