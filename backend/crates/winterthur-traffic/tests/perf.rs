//! Task 8 tick-budget measurement (spec §7): run the full shell on the REAL
//! net + REAL census trips at the 07:30 morning peak, `demand_scale = 1.0`,
//! and report wall-clock tick cost per 1000-tick window. Budget: mean tick
//! <= 50 ms at the observed peak (50 % of the 100 ms real-time tick).
//!
//! NOT a criterion bench (plan rule: plain timed runs only). Run locally:
//! `cargo test -p winterthur-traffic --release --test perf -- --ignored --nocapture`

mod common;

use std::time::Instant;
use winterthur_traffic::audit::Conservation;
use winterthur_traffic::shell::CoreRes;

#[test]
#[ignore = "real net + real trips.bin timed run — run locally with -- --ignored --nocapture"]
fn tick_budget_at_morning_peak() {
    let (mut world, mut ecs) = common::build_real_sim(0x0AB7_07A1, "07:30");

    // 1 sim-min warm-start release + 9 more sim-minutes of morning peak.
    const WINDOW: usize = 1000;
    const WINDOWS: usize = 6;
    let mut peak_alive = 0usize;
    println!("window | alive@end | mean ms | p95 ms | max ms");
    for w in 0..WINDOWS {
        let mut tick_ms = [0.0f64; WINDOW];
        for slot in tick_ms.iter_mut() {
            let t0 = Instant::now();
            ecs.run(&mut world);
            *slot = t0.elapsed().as_secs_f64() * 1e3;
        }
        let alive = world.resource::<CoreRes>().0.fleet.alive_count();
        peak_alive = peak_alive.max(alive);
        let mean = tick_ms.iter().sum::<f64>() / WINDOW as f64;
        let mut sorted = tick_ms;
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let p95 = sorted[(WINDOW as f64 * 0.95) as usize];
        let max = sorted[WINDOW - 1];
        println!(
            "{:>6} | {:>9} | {:>7.2} | {:>6.2} | {:>6.2}",
            (w + 1) * WINDOW,
            alive,
            mean,
            p95,
            max
        );
        // Budget gate on every window past the warm start.
        if w > 0 {
            assert!(
                mean <= 50.0,
                "mean tick {mean:.2} ms exceeds the 50 ms budget in window {w} (alive={alive})"
            );
        }
    }
    let cons = *world.resource::<Conservation>();
    println!("peak_alive={peak_alive} {cons:?}");
}
