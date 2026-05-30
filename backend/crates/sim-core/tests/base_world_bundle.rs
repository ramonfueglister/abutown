use sim_core::base_world::{BaseWorldBundle, BaseWorldError};
use sim_core::tile::TileKind;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-core crate lives under backend/crates/sim-core")
        .join("data/worlds/abutopia")
}

#[test]
fn loads_abutopia_base_world_fixture() {
    let bundle = BaseWorldBundle::load_from_dir(fixture_root()).expect("bundle loads");

    assert_eq!(bundle.world_id(), "abutopia");
    assert_eq!(bundle.chunk_size(), 32);
    assert_eq!(bundle.world_tiles().width, 224);
    assert_eq!(bundle.world_tiles().height, 128);
    assert_eq!(bundle.transport.roads.len(), 10);
    assert!(bundle.transport.rails.is_empty());
    assert!(bundle.transport.arterial_paths.is_empty());
    assert!(bundle.transport.rail_paths.is_empty());
    assert_eq!(bundle.transport.pedestrian_corridors.len(), 2);
    let sidewalk_ids = bundle
        .transport
        .pedestrian_corridors
        .iter()
        .map(|corridor| corridor.id.as_str())
        .collect::<Vec<_>>();
    assert!(sidewalk_ids.contains(&"corridor:sidewalk:north"));
    assert!(sidewalk_ids.contains(&"corridor:sidewalk:south"));
    assert_eq!(bundle.buildings.footprints.len(), 2);
    assert!(bundle.decorations.trees.is_empty());
    assert!(bundle.decorations.details.is_empty());
    assert_eq!(bundle.chunk_coords().len(), 28);
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
    let chunk = bundle.tiles_for_chunk(sim_core::ids::ChunkCoord { x: 3, y: 2 }, 7);

    assert_eq!(chunk.len(), 32 * 32);
    assert!(chunk.iter().any(|tile| tile.kind == TileKind::Road));
    assert!(
        chunk
            .iter()
            .any(|tile| tile.kind == TileKind::BuildingFootprint)
    );
    assert!(chunk.iter().all(|tile| tile.version == 7));
}
