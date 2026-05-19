//! One-off profiling tool for Phase 6 perf followup.
//!
//! Builds the same world as `mobility_tick_lod` bench (100k walkers, all
//! 512 chunks subscribed so no LOD demotion, warm up 50 ticks), then runs
//! N timed ticks and prints per-SystemSet timings so we can see where the
//! ~24 ms / tick goes.
//!
//! Not a criterion bench — wall-clock numbers per system over a single run.
//! Run with: `cargo run --release -p sim-core --example profile_lod_tick`.

use bevy_ecs::schedule::Schedule;
use sim_core::city_network::{CityNetwork, NetworkCoord, WorldTiles};
use sim_core::ids::ChunkCoord;
use sim_core::mobility::seed::{SeedDensity, from_network};
use sim_core::mobility::systems::*;
use std::time::Instant;

fn very_big_network() -> CityNetwork {
    // mirrors benches/mobility_tick_lod.rs::very_big_network
    let world_w: u32 = 1024;
    let world_h: u32 = 512;
    let corridor_count: u32 = 2000;
    let corridor_rows: u32 = 250;
    let corridor_x_step: i32 = 30;
    let corridor_len: i32 = 25;
    let arterial_count: u32 = 100;
    let arterial_y_step: i32 = 5;
    let arterial_len: i32 = 500;

    let corridors = (0..corridor_count)
        .map(|i| {
            let y = ((i % corridor_rows) * 2) as i32;
            let x_start = (i / corridor_rows) as i32 * corridor_x_step;
            vec![
                NetworkCoord { x: x_start, y },
                NetworkCoord { x: x_start + corridor_len, y },
            ]
        })
        .collect();
    let arterials = (0..arterial_count)
        .map(|i| {
            let y = i as i32 * arterial_y_step;
            vec![
                NetworkCoord { x: 0, y },
                NetworkCoord { x: arterial_len, y },
            ]
        })
        .collect();
    CityNetwork {
        version: 1,
        world_id: "lod-profile".to_string(),
        chunk_size: 32,
        world_tiles: WorldTiles { width: world_w, height: world_h },
        arterial_paths: arterials,
        pedestrian_corridors: corridors,
    }
}

fn main() {
    println!("building network…");
    let network = very_big_network();

    println!("seeding world (100k walkers + 1k cars)…");
    let mut world = from_network(
        &network,
        SeedDensity {
            pedestrians_per_corridor: 50, // 2000 × 50 = 100_000
            cars_per_arterial: 10,
            trams_total: 0,
        },
    );

    println!("subscribing to ENTIRE grid (keeps all 100k entities active)…");
    // World is 1024×512 tiles, chunk_size=32 → 32×16 = 512 chunks.
    let mut subscribed: Vec<ChunkCoord> = Vec::with_capacity(32 * 16);
    for x in 0..32 {
        for y in 0..16 {
            subscribed.push(ChunkCoord { x, y });
        }
    }
    world.apply_subscription_diff(&subscribed, std::iter::empty());

    println!("warming up 50 ticks…");
    for _ in 0..50 {
        world.tick_mobility();
    }

    // Now switch to manual per-set timing. The MobilityWorld owns its own
    // schedule + post-tick bookkeeping; we need access to the inner World.
    // We re-create the same systems inside 4 separate schedules and run them
    // sequentially on `world`'s internal bevy World by going through the
    // public `tick_mobility` for the full path AND by calling individual
    // schedules to measure each set.

    // For a clean per-SystemSet measurement we expose the inner world via a
    // helper added to MobilityWorld (see profile_inner_world below). For now,
    // run two parallel measurements: (a) full tick_mobility wall time, (b)
    // per-set time on a sibling clone.

    const N: usize = 30;
    println!("\n--- baseline: tick_mobility() over {N} ticks ---");
    let mut totals = Vec::with_capacity(N);
    for _ in 0..N {
        let t0 = Instant::now();
        world.tick_mobility();
        totals.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mean = totals.iter().sum::<f64>() / totals.len() as f64;
    let max = totals.iter().cloned().fold(0.0f64, f64::max);
    let min = totals.iter().cloned().fold(f64::INFINITY, f64::min);
    println!("tick_mobility()  mean={mean:.2}ms  min={min:.2}ms  max={max:.2}ms");

    println!("\n--- per-set timing (sibling schedules on the same world) ---");
    println!("NOTE: numbers below sum to MORE than tick_mobility() because");
    println!("Commands flush 4× instead of 1× and resources rebuild redundantly.");
    println!("Treat as a relative breakdown, not an absolute decomposition.\n");

    // Fine-grained per-system schedules so we can see which individual
    // system inside Advance / Output / LOD dominates.
    let mut s_track = {
        let mut s = Schedule::default(); s.add_systems(track_chunk_populations_system); s
    };
    let mut s_classify = {
        let mut s = Schedule::default(); s.add_systems(classify_activity_system); s
    };
    let mut s_promote = {
        let mut s = Schedule::default(); s.add_systems(promote_warm_to_active_system); s
    };
    let mut s_demote = {
        let mut s = Schedule::default(); s.add_systems(demote_active_to_warm_system); s
    };
    let mut s_walk = {
        let mut s = Schedule::default(); s.add_systems(walk_advance_system); s
    };
    let mut s_board = {
        let mut s = Schedule::default(); s.add_systems(boarding_alighting_system); s
    };
    let mut s_arrive = {
        let mut s = Schedule::default(); s.add_systems(stop_arrival_system); s
    };
    let mut s_vehadv = {
        let mut s = Schedule::default(); s.add_systems(vehicle_advance_system); s
    };
    let mut s_warmflow = {
        let mut s = Schedule::default(); s.add_systems(warm_chunk_flow_system); s
    };
    let mut s_coord = {
        let mut s = Schedule::default(); s.add_systems(compute_world_coord_system); s
    };
    let mut s_dir = {
        let mut s = Schedule::default(); s.add_systems(compute_direction_system); s
    };
    let mut s_book = {
        let mut s = Schedule::default(); s.add_systems(tick_increment_system); s
    };

    let labels = [
        "track_pop", "classify", "promote", "demote",
        "walk_adv", "boarding", "stop_arrive", "veh_adv", "warm_flow",
        "world_coord", "direction", "tick_inc",
    ];
    let mut samples: [Vec<f64>; 12] = Default::default();
    for v in samples.iter_mut() { v.reserve(N); }
    for _ in 0..N {
        let w = world.profile_world_mut();
        let mut idx = 0;
        for sched in [
            &mut s_track, &mut s_classify, &mut s_promote, &mut s_demote,
            &mut s_walk, &mut s_board, &mut s_arrive, &mut s_vehadv, &mut s_warmflow,
            &mut s_coord, &mut s_dir, &mut s_book,
        ] {
            let t = Instant::now();
            sched.run(w);
            samples[idx].push(t.elapsed().as_secs_f64() * 1000.0);
            idx += 1;
        }
    }
    let report = |label: &str, samples: &[f64]| {
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let max = samples.iter().cloned().fold(0.0f64, f64::max);
        let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
        println!("{label:>14}  mean={mean:6.2}ms  min={min:6.2}ms  max={max:6.2}ms");
    };
    for (i, label) in labels.iter().enumerate() {
        report(label, &samples[i]);
    }

    // Entity count sanity-check.
    let w = world.profile_world_mut();
    let agent_count = w.query::<&sim_core::mobility::components::AgentMarker>().iter(w).count();
    let vehicle_count = w.query::<&sim_core::mobility::components::VehicleMarker>().iter(w).count();
    println!("\nfinal entity count: agents={agent_count} vehicles={vehicle_count}");
}
