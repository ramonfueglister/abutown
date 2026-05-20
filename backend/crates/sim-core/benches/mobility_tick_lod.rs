use criterion::{Criterion, criterion_group, criterion_main};
use sim_core::ids::ChunkCoord;
use sim_core::mobility::api::{apply_subscription_diff, tick_mobility};
use sim_core::mobility::seed::{SeedDensity, from_network};

mod common;
use common::SyntheticNetwork;

fn very_big_network() -> sim_core::city_network::CityNetwork {
    SyntheticNetwork {
        world_id: "lod-bench",
        world_w: 1024,
        world_h: 512,
        corridor_count: 2000,
        corridor_rows: 250,
        corridor_x_step: 30,
        corridor_len: 25,
        arterial_count: 100,
        arterial_y_step: 5,
        arterial_len: 500,
    }
    .build()
}

fn tick_100k_with_5_subscribed(c: &mut Criterion) {
    let network = very_big_network();
    c.bench_function("tick_100k_with_5_subscribed_chunks", |b| {
        let (mut world, mut schedule) = from_network(
            &network,
            SeedDensity {
                pedestrians_per_corridor: 50,
                cars_per_arterial: 10,
                trams_total: 0,
            },
        );

        let subscribed: Vec<ChunkCoord> = (0..5).map(|i| ChunkCoord { x: 8 + i, y: 4 }).collect();
        apply_subscription_diff(&mut world, &subscribed, std::iter::empty());

        for _ in 0..50 {
            tick_mobility(&mut world, &mut schedule);
        }

        b.iter(|| {
            tick_mobility(&mut world, &mut schedule);
        });
    });
}

fn tick_100k_all_active(c: &mut Criterion) {
    let network = very_big_network();
    c.bench_function("tick_100k_all_active", |b| {
        let (mut world, mut schedule) = from_network(
            &network,
            SeedDensity {
                pedestrians_per_corridor: 50,
                cars_per_arterial: 10,
                trams_total: 0,
            },
        );

        let mut subscribed: Vec<ChunkCoord> = Vec::with_capacity(32 * 16);
        for x in 0..32 {
            for y in 0..16 {
                subscribed.push(ChunkCoord { x, y });
            }
        }
        apply_subscription_diff(&mut world, &subscribed, std::iter::empty());

        for _ in 0..50 {
            tick_mobility(&mut world, &mut schedule);
        }

        b.iter(|| {
            tick_mobility(&mut world, &mut schedule);
        });
    });
}

criterion_group!(benches, tick_100k_with_5_subscribed, tick_100k_all_active);
criterion_main!(benches);
