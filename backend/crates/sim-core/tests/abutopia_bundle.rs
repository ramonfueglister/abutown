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
    assert_eq!(bundle.spawns.pedestrian_groups[0].agents_per_corridor, 300);
    assert!(bundle.transport.rails.is_empty());
    assert!(bundle.transport.arterial_paths.is_empty());
    assert!(bundle.spawns.car_groups.is_empty());
}

/// The production-chain authoring: exactly one producer (8031, WOOD→TOOLS at 9001),
/// the WOOD extractor 8041 (RAW→WOOD at 9003), the 9001↔9003 distance pair (the WOOD
/// route — macro_flow prunes cross-edges without a baked distance), and the WOOD
/// opening prices at BOTH ends (50 source / 380 sink, under the 400 participation
/// bound) plus TOOLS at 9001 (the bound's reference price).
#[test]
fn abutopia_authors_the_wood_to_tools_production_chain() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("abutopia bundle loads");
    let markets = &bundle.markets;

    assert_eq!(markets.producers.len(), 1, "exactly one producer authored");
    let p = &markets.producers[0];
    assert_eq!(p.actor, 8031);
    assert_eq!(p.market, 9001);
    assert_eq!((p.in_good, p.in_qty), (2, 10), "buys WOOD");
    assert_eq!((p.out_good, p.out_qty), (4, 10), "makes TOOLS");
    assert_eq!(
        (p.qty, p.min_price),
        (10, 500),
        "sell side like an extractor"
    );
    assert_eq!((p.theta_bps, p.batches_target), (8000, 2));
    assert_eq!(p.opening_cash, 1_000_000);

    // 8041 replaces 8031 in extractors; 8031 must NOT be an extractor anymore.
    assert!(
        markets.extractors.iter().all(|e| e.actor != 8031),
        "8031 left the extractors section"
    );
    let wood = markets
        .extractors
        .iter()
        .find(|e| e.actor == 8041)
        .expect("WOOD extractor 8041 authored");
    assert_eq!(
        (wood.market, wood.in_good, wood.out_good),
        (9003, 5, 2),
        "8041: RAW→WOOD at 9003"
    );
    assert_eq!((wood.qty, wood.min_price), (10, 50));

    // The WOOD route needs a baked distance pair.
    assert!(
        markets
            .distances
            .iter()
            .any(|d| (d.from, d.to) == (9001, 9003) || (d.from, d.to) == (9003, 9001)),
        "9001↔9003 distance pair authored (the WOOD route)"
    );

    // Opening prices: WOOD at both ends + TOOLS at the producer's home market.
    let price = |market: u32, good: u16| {
        markets
            .opening_prices
            .iter()
            .find(|o| o.market == market && o.good == good)
            .map(|o| o.price)
    };
    assert_eq!(price(9003, 2), Some(50), "WOOD source opening price");
    assert_eq!(price(9001, 2), Some(380), "WOOD sink opening price");
    assert_eq!(price(9001, 4), Some(1000), "TOOLS reference at 9001");
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
    assert_eq!(group.agents_per_corridor, 300);

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
