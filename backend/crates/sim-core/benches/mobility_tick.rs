use criterion::{Criterion, criterion_group, criterion_main};
use sim_core::mobility::seed::{SeedDensity, from_network};

mod common;
use common::SyntheticNetwork;

fn big_network() -> sim_core::city_network::CityNetwork {
    SyntheticNetwork {
        world_id: "bench",
        world_w: 512,
        world_h: 512,
        corridor_count: 1000,
        corridor_rows: 200,
        corridor_x_step: 50,
        corridor_len: 40,
        arterial_count: 50,
        arterial_y_step: 4,
        arterial_len: 250,
    }
    .build()
}

fn tick_10k_walkers_1k_cars(c: &mut Criterion) {
    let network = big_network();
    c.bench_function("tick_10k_walkers_1k_cars", |b| {
        let mut world = from_network(
            &network,
            SeedDensity {
                pedestrians_per_corridor: 10, // 1000 × 10 = 10_000 walkers
                cars_per_arterial: 20,        // 50 × 20 = 1_000 cars
                trams_total: 0,
            },
        );
        b.iter(|| {
            world.tick_mobility();
        });
    });
}

criterion_group!(benches, tick_10k_walkers_1k_cars);
criterion_main!(benches);
