use std::collections::HashMap;

use sim_core::ids::AgentId;
use sim_core::mobility::{
    AgentMobilityState, AgentRecord, MobilityPersistSnapshot, PersistedActiveRoute,
    PersistedRouteStep, PlanStage, api, apply_into_world, extract_from_world,
};
use sim_core::routing::{ModeState, RoutingProfileKey};

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
                destination_node: 999,
                profile: RoutingProfileKey::Walk,
                cursor: 1,
                steps: vec![PersistedRouteStep {
                    edge_id: 999,
                    mode: ModeState::Walking,
                    canonical_edge_key: "link:future-only".to_string(),
                    length: 1.0,
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
fn active_route_round_trip_normalizes_rebuilt_graph_ids() {
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
    assert_eq!(route.cursor, 1);
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

#[test]
#[should_panic(expected = "persisted active_route canonical edge key edge:999 is missing")]
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
#[should_panic(expected = "persisted active_route cursor 2 exceeds 1 steps")]
fn active_route_hydration_rejects_cursor_past_steps() {
    let mut snap = active_route_snapshot();
    snap.agents
        .get_mut(&AgentId("agent:active-route".to_string()))
        .unwrap()
        .active_route
        .as_mut()
        .unwrap()
        .cursor = 2;

    let (mut world, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut world, snap);
}
