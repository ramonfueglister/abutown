//! Task 6 integration proof: the full economy tick chain on the REAL authored
//! `economy.json` seed conserves money byte-exactly over 10_000 world ticks
//! (= 1000 econ rounds at the 1 Hz cadence) AND actually trades.

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;
use world_core::econ::{EconomyEvent, EconomySeed};
use world_core::{SimWorld, WorldClock, WorldCorePlugin, econ, install_world_systems};

/// The REAL authored seed — the test proves the shipped data trades.
const ECONOMY_JSON: &str = include_str!("../../../../data/winterthur/economy.json");

/// 3-building fixture from `model/mod.rs` (residential B1, commercial A2,
/// unknown C3) — all buildings within 5 km of every authored market.
const FIXTURE: &str = r#"{
  "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
  "buildings": [
    {"id":"{B1}","usage":1,"x":0.0,"z":0.0,"area_m2":200.0,"height_m":9.0,"access_edge":5,"access_offset":2.0},
    {"id":"{A2}","usage":2,"x":100.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":7,"access_offset":1.0},
    {"id":"{C3}","usage":0,"x":500.0,"z":500.0,"area_m2":50.0,"height_m":4.0,"access_edge":-1,"access_offset":0.0}
  ]}"#;

fn build_test_sim() -> (World, Schedule) {
    let sim = Arc::new(SimWorld::load(FIXTURE).expect("fixture must load"));
    let seed = EconomySeed::from_json(ECONOMY_JSON).expect("authored economy.json must parse");
    let plugin = WorldCorePlugin {
        seed,
        sim_world: sim,
    };
    let mut world = World::new();
    let mut schedule = Schedule::default();
    install_world_systems(&mut world, &mut schedule, &plugin);
    (world, schedule)
}

#[test]
fn thousand_econ_ticks_conserve_money_and_trade() {
    let (mut world, mut schedule) = build_test_sim();
    let start = world.resource::<econ::AccountBook>().total_money().unwrap();
    for _ in 0..10_000 {
        schedule.run(&mut world);
    }
    assert_eq!(world.resource::<WorldClock>().world_tick, 10_000);
    let end = world.resource::<econ::AccountBook>().total_money().unwrap();
    assert_eq!(start, end, "SFC conservation violated");
    let goods = world.resource::<econ::MarketGoods>();
    assert!(
        goods
            .0
            .iter()
            .any(|(_, s)| s.traded_qty_last_tick.0 > 0 || s.ewma_reference_price.0 > 0),
        "economy is dead: nothing ever traded"
    );
    // The seed legitimately opens reference prices > 0, so the plan assertion
    // above cannot distinguish a dead economy from a live one on its own.
    // Harden (never weaken): demand at least one REAL settled trade.
    let ledger = world.resource::<econ::TradeLedger>();
    assert!(
        ledger
            .0
            .iter()
            .any(|e| matches!(e, EconomyEvent::Trade { .. })),
        "economy is dead: no Trade event in 1000 econ rounds"
    );
}
