use sim_core::base_world::BaseWorldBundle;

fn abutopia_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .join("data/worlds/abutopia")
}

#[test]
fn loads_abutopia_base_world() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("abutopia bundle loads");

    assert_eq!(bundle.world_id(), "abutopia");
    assert_eq!(bundle.chunk_size(), 32);
    assert_eq!(bundle.world_tiles().width, 16);
    assert_eq!(bundle.world_tiles().height, 8);
    assert_eq!(bundle.transport.roads.len(), 10);
    assert_eq!(bundle.buildings.footprints.len(), 2);
    assert_eq!(bundle.transport.pedestrian_corridors.len(), 1);
    assert_eq!(bundle.spawns.pedestrian_groups.len(), 1);
    assert_eq!(bundle.spawns.pedestrian_groups[0].agents_per_corridor, 1);
    assert!(bundle.transport.arterial_paths.is_empty());
    assert!(bundle.spawns.car_groups.is_empty());
    assert!(bundle.spawns.tram_lines.is_empty());
}

#[test]
fn abutopia_seeds_one_pedestrian_corridor() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("abutopia bundle loads");

    assert_eq!(bundle.spawns.pedestrian_groups.len(), 1);
    let group = &bundle.spawns.pedestrian_groups[0];
    assert_eq!(group.corridor_id, "corridor:main");
    assert_eq!(group.agents_per_corridor, 1);

    let corridor = bundle
        .transport
        .pedestrian_corridors
        .iter()
        .find(|corridor| corridor.id == group.corridor_id)
        .expect("pedestrian corridor exists");
    assert_eq!(corridor.points.len(), 12);
}
