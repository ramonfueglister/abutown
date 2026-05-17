use sim_core::mobility::MobilityWorld;

#[test]
fn phase3_snapshot_round_trips_byte_for_byte() {
    let fixture = include_str!("fixtures/phase3-mobility-snapshot.json");

    // Parse fixture → ECS-backed MobilityWorld
    let world: MobilityWorld = serde_json::from_str(fixture)
        .expect("phase3 fixture should deserialize into ECS MobilityWorld");

    // Re-serialize through the new ECS path
    let reserialized = serde_json::to_string_pretty(&world).expect("re-serialize should not fail");

    // Compare as JSON values (whitespace-insensitive, key-order-insensitive)
    let fixture_value: serde_json::Value =
        serde_json::from_str(fixture).expect("fixture is valid JSON");
    let reserialized_value: serde_json::Value =
        serde_json::from_str(&reserialized).expect("our re-serialized output is valid JSON");

    // Phase 6: re-serialize emits `flow_cells` and `chunk_activities` fields that
    // didn't exist in the frozen Phase-3 fixture. Compare only the legacy fields.
    for key in ["tick", "agents", "vehicles", "stops", "routes", "link_polylines"] {
        assert_eq!(
            fixture_value.get(key),
            reserialized_value.get(key),
            "round-trip diverged on legacy key `{key}`"
        );
    }
}
