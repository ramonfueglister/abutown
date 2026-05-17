use criterion::{criterion_group, criterion_main, Criterion};
use sim_core::city_network::{CityNetwork, NetworkCoord, WorldTiles};
use sim_core::ids::ChunkCoord;
use sim_core::mobility::seed::{from_network, SeedDensity};
use std::collections::HashSet;

fn very_big_network() -> CityNetwork {
    let mut corridors = Vec::with_capacity(2000);
    for i in 0..2000u32 {
        let y = (i % 250) * 2;
        let x_start = ((i / 250) * 30) as i32;
        corridors.push(vec![
            NetworkCoord {
                x: x_start,
                y: y as i32,
            },
            NetworkCoord {
                x: x_start + 25,
                y: y as i32,
            },
        ]);
    }
    let mut arterials = Vec::with_capacity(100);
    for i in 0..100u32 {
        let y = (i * 5) as i32;
        arterials.push(vec![NetworkCoord { x: 0, y }, NetworkCoord { x: 500, y }]);
    }
    CityNetwork {
        version: 1,
        world_id: "lod-bench".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles {
            width: 1024,
            height: 512,
        },
        arterial_paths: arterials,
        pedestrian_corridors: corridors,
    }
}

fn tick_100k_with_5_subscribed(c: &mut Criterion) {
    let network = very_big_network();
    c.bench_function("tick_100k_with_5_subscribed_chunks", |b| {
        let mut world = from_network(
            &network,
            SeedDensity {
                pedestrians_per_corridor: 50, // 2000 × 50 = 100_000 walkers
                cars_per_arterial: 10,
                trams_total: 0,
            },
        );

        let empty: HashSet<ChunkCoord> = HashSet::new();
        let mut subscribed: HashSet<ChunkCoord> = HashSet::new();
        for i in 0..5 {
            subscribed.insert(ChunkCoord { x: 8 + i, y: 4 });
        }
        world.update_chunk_subscribers(&empty, &subscribed);

        // Warm up: let LOD demote the non-subscribed chunks into FlowCells
        // and let the hysteresis cooldown settle.
        for _ in 0..50 {
            world.tick_mobility();
        }

        b.iter(|| {
            world.tick_mobility();
        });
    });
}

criterion_group!(benches, tick_100k_with_5_subscribed);
criterion_main!(benches);
