use sim_core::base_world::{BaseWorldBundle, BaseWorldError};
use sim_core::tile::TileKind;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-core crate lives under backend/crates/sim-core")
        .join("data/worlds/zurich-river-city-v1")
}

#[test]
fn loads_zurich_base_world_fixture() {
    let bundle = BaseWorldBundle::load_from_dir(fixture_root()).expect("bundle loads");

    assert_eq!(bundle.world_id(), "zurich-river-city-v1");
    assert_eq!(bundle.chunk_size(), 32);
    assert_eq!(bundle.world_tiles().width, 256);
    assert_eq!(bundle.world_tiles().height, 256);
    assert_eq!(bundle.transport.roads.len(), 3_396);
    assert_eq!(bundle.transport.rails.len(), 256);
    assert_eq!(bundle.transport.arterial_paths.len(), 3);
    assert_eq!(bundle.transport.rail_paths.len(), 1);
    assert_eq!(bundle.transport.pedestrian_corridors.len(), 160);
    assert!(bundle.buildings.footprints.len() >= 2_268);
    assert!(bundle.decorations.trees.len() > 3_000);
    assert!(bundle.decorations.details.len() >= 260);
    assert_eq!(bundle.chunk_coords().len(), 64);
}

#[test]
fn missing_manifest_fails_closed() {
    let err = BaseWorldBundle::load_from_dir(fixture_root().join("missing"))
        .expect_err("missing manifest is fatal");

    assert!(matches!(err, BaseWorldError::MissingManifest(_)));
}

#[test]
fn materializes_chunk_tiles_from_bundle_layers() {
    let bundle = BaseWorldBundle::load_from_dir(fixture_root()).expect("bundle loads");
    let chunk = bundle.tiles_for_chunk(sim_core::ids::ChunkCoord { x: 4, y: 4 }, 7);

    assert_eq!(chunk.len(), 32 * 32);
    assert!(chunk.iter().any(|tile| tile.kind == TileKind::Road));
    assert!(chunk.iter().any(|tile| tile.kind == TileKind::Water));
    assert!(
        chunk
            .iter()
            .any(|tile| tile.kind == TileKind::BuildingFootprint)
    );
    assert!(chunk.iter().all(|tile| tile.version == 7));
}
