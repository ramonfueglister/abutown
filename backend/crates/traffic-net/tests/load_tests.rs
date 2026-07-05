//! Integration tests against the hand-written `mini.json` fixture (4 nodes:
//! dead_end, signal, priority, dead_end; one two-lane edge) and small in-line
//! JSON mutations of it that each violate exactly one invariant.

use traffic_net::{NetError, load};

fn mini_json() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mini.json"
    ))
    .expect("mini.json fixture must exist")
}

#[test]
fn loads_valid_mini_fixture() {
    let net = load(&mini_json()).expect("mini.json should load and validate cleanly");
    assert_eq!(net.nodes.len(), 4);
    assert_eq!(net.edges.len(), 4);
    assert_eq!(net.lanes.len(), 5);
    assert_eq!(net.turns.len(), 4);
}

#[test]
fn lane_len_reads_declared_length() {
    let net = load(&mini_json()).unwrap();
    assert_eq!(net.lane_len(0), 100.0);
    assert_eq!(net.lane_len(2), 100.0);
}

#[test]
fn turns_from_returns_csr_backed_slice() {
    let net = load(&mini_json()).unwrap();
    // lane 0 (edge 0, node 1) has turn 0 departing it.
    assert_eq!(net.turns_from(0), &[0]);
    // lane 4 (edge 3, node 1) has turn 1 departing it.
    assert_eq!(net.turns_from(4), &[1]);
    // lane 3 has no turns departing it (edge 2 ends at a dead end).
    assert_eq!(net.turns_from(3), &[] as &[u32]);
}

#[test]
fn pos_at_interpolates_straight_lane_start_mid_end() {
    let net = load(&mini_json()).unwrap();
    // lane 0: straight segment (0,0) -> (100,0), length 100.
    let (p0, t0) = net.pos_at(0, 0.0);
    assert_eq!(p0, [0.0, 0.0]);
    assert_eq!(t0, [1.0, 0.0]);

    let (p_mid, t_mid) = net.pos_at(0, 50.0);
    assert_eq!(p_mid, [50.0, 0.0]);
    assert_eq!(t_mid, [1.0, 0.0]);

    let (p_end, _t_end) = net.pos_at(0, 100.0);
    assert_eq!(p_end, [100.0, 0.0]);
}

#[test]
fn pos_at_clamps_beyond_lane_length() {
    let net = load(&mini_json()).unwrap();
    let (p_over, _) = net.pos_at(0, 999.0);
    assert_eq!(p_over, [100.0, 0.0]);
    let (p_under, _) = net.pos_at(0, -50.0);
    assert_eq!(p_under, [0.0, 0.0]);
}

// ---- corrupt-fixture -> typed NetError tests ----

fn replace_one(json: &str, from: &str, to: &str) -> String {
    let n = json.matches(from).count();
    assert_eq!(
        n, 1,
        "expected exactly one occurrence of {from:?} to replace, found {n}"
    );
    json.replacen(from, to, 1)
}

#[test]
fn dangling_lane_id_on_edge_is_rejected() {
    let json = mini_json();
    // edge 0's lanes array references lane 0; point it at a nonexistent lane 99.
    let corrupt = replace_one(&json, "\"lanes\": [0]", "\"lanes\": [99]");
    let err = load(&corrupt).expect_err("dangling lane id must fail validation");
    assert_eq!(err, NetError::DanglingLane { edge: 0, lane: 99 });
}

#[test]
fn dangling_node_on_edge_is_rejected() {
    let json = mini_json();
    let corrupt = replace_one(
        &json,
        "\"from\": 0,\n      \"to\": 1,",
        "\"from\": 77,\n      \"to\": 1,",
    );
    let err = load(&corrupt).expect_err("dangling node id must fail validation");
    assert_eq!(
        err,
        NetError::DanglingNode {
            edge: 0,
            node: 77,
            field: "from"
        }
    );
}

#[test]
fn lane_length_mismatch_beyond_tolerance_is_rejected() {
    let json = mini_json();
    // lane 0's true polyline length is 100; declare 105 (5% off, exceeds 1% tolerance).
    let corrupt = replace_one(
        &json,
        "\"id\": 0,\n      \"edge\": 0,\n      \"index\": 0,\n      \"lengthM\": 100,",
        "\"id\": 0,\n      \"edge\": 0,\n      \"index\": 0,\n      \"lengthM\": 105,",
    );
    let err = load(&corrupt).expect_err("length mismatch beyond tolerance must fail validation");
    match err {
        NetError::LaneLengthMismatch {
            lane,
            declared,
            actual,
            ..
        } => {
            assert_eq!(lane, 0);
            assert_eq!(declared, 105.0);
            assert_eq!(actual, 100.0);
        }
        other => panic!("expected LaneLengthMismatch, got {other:?}"),
    }
}

#[test]
fn signal_missing_a_turn_in_its_phases_is_rejected() {
    let json = mini_json();
    // node 1 is a signal whose two phases gate turns [0] and [1]; drop turn 1
    // from its phase so incoming turn 1 (at node 1) is left uncovered.
    let corrupt = replace_one(&json, "\"turns\": [1]", "\"turns\": []");
    let err = load(&corrupt).expect_err("uncovered signal turn must fail validation");
    match err {
        NetError::SignalPhaseCoverageMismatch { node, .. } => assert_eq!(node, 1),
        other => panic!("expected SignalPhaseCoverageMismatch, got {other:?}"),
    }
}

#[test]
fn dangling_turn_conflict_reference_is_rejected() {
    let json = mini_json();
    let corrupt = replace_one(&json, "\"conflictsWith\": [2]", "\"conflictsWith\": [42]");
    let err = load(&corrupt).expect_err("dangling conflictsWith id must fail validation");
    assert_eq!(err, NetError::TurnDanglingConflict { turn: 3, other: 42 });
}

#[test]
fn malformed_json_is_a_parse_error() {
    let err = load("{ not valid json").unwrap_err();
    match err {
        NetError::Parse(_) => {}
        other => panic!("expected Parse error, got {other:?}"),
    }
}

/// Ignored by default: point `TRAFFICNET_JSON` at the real baked asset and
/// run once to confirm the schema mirrors production exactly, e.g.:
///   TRAFFICNET_JSON=../../../data/winterthur/trafficnet.json \
///     scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p traffic-net -- --ignored
#[test]
#[ignore]
fn loads_baked_winterthur() {
    let path = std::env::var("TRAFFICNET_JSON")
        .expect("set TRAFFICNET_JSON to the baked trafficnet.json path");
    let json =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let net = load(&json).expect("real baked trafficnet.json must load and validate cleanly");
    assert!(net.edges.len() > 100);
}
