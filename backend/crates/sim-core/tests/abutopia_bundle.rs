use sim_core::base_world::BaseWorldBundle;
use sim_core::city_network::NetworkPoint;

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
    assert_eq!(bundle.world_tiles().width, 224);
    assert_eq!(bundle.world_tiles().height, 128);
    assert_eq!(bundle.transport.roads.len(), 10);
    assert_eq!(bundle.buildings.footprints.len(), 2);
    assert_eq!(bundle.transport.pedestrian_corridors.len(), 2);
    assert_eq!(bundle.spawns.pedestrian_groups.len(), 1);
    assert_eq!(bundle.spawns.pedestrian_groups[0].agents_per_corridor, 1);
    assert!(bundle.transport.rails.is_empty());
    assert!(bundle.transport.arterial_paths.is_empty());
    assert!(bundle.spawns.car_groups.is_empty());
    assert!(bundle.spawns.tram_lines.is_empty());
}

#[test]
fn abutopia_authors_sidewalk_pedestrian_corridors() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("abutopia bundle loads");

    assert_eq!(bundle.transport.pedestrian_corridors.len(), 2);
    assert_corridor_points(&bundle, "corridor:sidewalk:north", 63.49);
    assert_corridor_points(&bundle, "corridor:sidewalk:south", 64.51);
    assert!(
        bundle
            .transport
            .pedestrian_corridors
            .iter()
            .flat_map(|corridor| corridor.points.iter())
            .all(|point| point.y != 64.0)
    );

    assert_eq!(bundle.spawns.pedestrian_groups.len(), 1);
    let group = &bundle.spawns.pedestrian_groups[0];
    assert_eq!(group.id, "spawn:ped:sidewalk-south");
    assert_eq!(group.corridor_id, "corridor:sidewalk:south");
    assert_eq!(group.agents_per_corridor, 1);

    assert!(
        bundle
            .transport
            .pedestrian_corridors
            .iter()
            .any(|corridor| corridor.id == group.corridor_id)
    );
}

fn assert_corridor_points(bundle: &BaseWorldBundle, corridor_id: &str, y: f32) {
    let corridor = bundle
        .transport
        .pedestrian_corridors
        .iter()
        .find(|corridor| corridor.id == corridor_id)
        .expect("pedestrian corridor exists");
    let expected = (106..=117)
        .map(|x| NetworkPoint { x: x as f32, y })
        .collect::<Vec<_>>();
    assert_eq!(corridor.points, expected);
}
