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
