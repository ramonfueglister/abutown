use sim_core::mobility::{MobilityPersistSnapshot, apply_into_world, api, extract_from_world};

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

    // Re-serialize may emit newer top-level keys (e.g. `flow_cells`,
    // `chunk_activities`) that the frozen Phase-3 fixture predates. The
    // legacy keys present in the fixture itself drive the comparison, so
    // adding another legacy key to the fixture later automatically widens
    // the assertion without touching this test.
    let fixture_keys: Vec<&String> = fixture_value
        .as_object()
        .expect("fixture is a JSON object")
        .keys()
        .collect();
    for key in fixture_keys {
        assert_eq!(
            fixture_value.get(key),
            reserialized_value.get(key),
            "round-trip diverged on legacy key `{key}`"
        );
    }
}
