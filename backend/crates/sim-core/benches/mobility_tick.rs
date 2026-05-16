use criterion::{criterion_group, criterion_main, Criterion};
use sim_core::city_network::{CityNetwork, NetworkCoord, WorldTiles};
use sim_core::mobility::seed::{from_network, SeedDensity};

fn big_network() -> CityNetwork {
    let mut corridors = Vec::with_capacity(1000);
    for i in 0..1000u32 {
        let y = (i % 200) * 2;
        let x_start = ((i / 200) * 50) as i32;
        corridors.push(vec![
            NetworkCoord { x: x_start, y: y as i32 },
            NetworkCoord { x: x_start + 40, y: y as i32 },
        ]);
    }
    let mut arterials = Vec::with_capacity(50);
    for i in 0..50u32 {
        let y = (i * 4) as i32;
        arterials.push(vec![
            NetworkCoord { x: 0, y },
            NetworkCoord { x: 250, y },
        ]);
    }
    CityNetwork {
        version: 1,
        world_id: "bench".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: 512, height: 512 },
        arterial_paths: arterials,
        pedestrian_corridors: corridors,
    }
}

fn tick_10k_walkers_1k_cars(c: &mut Criterion) {
    let network = big_network();
    c.bench_function("tick_10k_walkers_1k_cars", |b| {
        let mut world = from_network(&network, SeedDensity {
            pedestrians_per_corridor: 10,  // 1000 × 10 = 10_000 walkers
            cars_per_arterial: 20,          // 50 × 20 = 1_000 cars
            trams_total: 0,
        });
        b.iter(|| {
            world.tick_mobility();
        });
    });
}

criterion_group!(benches, tick_10k_walkers_1k_cars);
criterion_main!(benches);
