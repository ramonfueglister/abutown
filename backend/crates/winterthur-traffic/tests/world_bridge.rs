//! Task 9 shell-integration proof: `build_sim` mit [`WorldCoreExt`] webt die
//! Bürger-Welt in die Traffic-Kette — ein Bürger pendelt via Tagesrhythmus
//! als ECHTES Fahrzeug über das diamond-gateway-Netz (CH-Adapter, kein
//! Fixture-Router), die Fahrzeug-Konservierung hält über Brücken-Spawns und
//! -Despawns hinweg, und die Ankunft löst `CitizenState::AtWork` auf.

mod common;

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use demand_gen::output;
use winterthur_traffic::ChTripRouter;
use winterthur_traffic::audit::Conservation;
use winterthur_traffic::demand::TripSchedule;
use winterthur_traffic::shell::{self, CoreRes, WorldCoreExt};
use winterthur_traffic::spawner::SpawnerCfg;
use world_core::econ::EconomySeed;
use world_core::{
    ActiveTrips, Citizen, CitizenState, SeedParams, SimWorld, WorldClock, WorldCorePlugin,
};

const ECONOMY_JSON: &str = include_str!("../../../../data/winterthur/economy.json");

/// Ein Wohnhaus (1 Bewohner) an Edge 0 und sein Arbeitsplatz 900 m entfernt
/// an Edge 5 — der Pendelweg MUSS als Auto durchs Netz führen.
const SIMWORLD: &str = r#"{
  "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
  "buildings": [
    {"id":"{H1}","usage":1,"x":0.0,"z":0.0,"area_m2":40.0,"height_m":3.0,"access_edge":0,"access_offset":5.0},
    {"id":"{W2}","usage":2,"x":900.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":5,"access_offset":50.0}
  ]}"#;

fn fixture_json() -> String {
    let p = format!(
        "{}/tests/fixtures/diamond-gateway.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

/// Leere Census-Tabelle: der einzige Verkehr in diesem Test sind Bürger.
fn empty_trips(net_json: &str) -> TripSchedule {
    let net_hash = *blake3::hash(net_json.as_bytes()).as_bytes();
    let mut bytes = Vec::new();
    output::write_trips(&mut bytes, &net_hash, &[], &[]).unwrap();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "winterthur-traffic-world-bridge-test-{}.bin",
        std::process::id()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&bytes).unwrap();
    TripSchedule::load(&path, net_json.as_bytes()).unwrap()
}

#[test]
fn citizen_commutes_as_real_vehicle_through_the_shell() {
    let json = fixture_json();
    let net = traffic_net::load(&json).expect("diamond-gateway fixture must validate");
    let router = ChTripRouter::new(&net);

    let ext = WorldCoreExt {
        plugin: WorldCorePlugin {
            seed: EconomySeed::from_json(ECONOMY_JSON).expect("economy.json must parse"),
            sim_world: Arc::new(SimWorld::load(SIMWORLD).expect("simworld fixture must load")),
            seed_params: SeedParams {
                center: (0.0, 0.0),
                radius_m: 10_000.0,
                residents_per_40m2: 1.0,
                seed: 42,
            },
        },
        router: Box::new(router),
        snapshot: None,
    };

    let (mut world, mut schedule) = shell::build_sim(
        net,
        7,
        empty_trips(&json),
        common::workday_clock("07:30"),
        SpawnerCfg::default(),
        Some(ext),
    );

    // Weltzeit auf 07:29 stellen (vor jedem 07:30±45min-Jitter-Slot) — der
    // Rhythmus muss den ToWork-Trip im 2-Welt-Stunden-Fenster emittieren.
    world.insert_resource(WorldClock { world_tick: 44_900 });

    let citizens: Vec<Citizen> = world.query::<&Citizen>().iter(&world).copied().collect();
    assert_eq!(citizens.len(), 1, "fixture seeds exactly one citizen");
    assert_eq!((citizens[0].home, citizens[0].work), (0, 1));

    // 2 Welt-Stunden = 12 000 Ticks: Emission + 900 m Fahrt + Ankunft, mit
    // Konservierungs-Check an jedem 500-Tick-Checkpoint (der debug_assert in
    // core_tick prüft ihn ohnehin jeden Tick — hier explizit als Testbeweis).
    let mut drove = false;
    for block in 0..24 {
        for _ in 0..500 {
            schedule.run(&mut world);
            drove |= world.resource::<CoreRes>().0.fleet.alive_count() > 0;
        }
        let alive = world.resource::<CoreRes>().0.fleet.alive_count();
        let cons = *world.resource::<Conservation>();
        assert!(
            cons.holds(alive),
            "vehicle conservation violated at block {block}: {cons:?} alive={alive}"
        );
    }

    assert!(
        drove,
        "the commute must have put a real vehicle on the road"
    );
    assert!(
        world.resource::<ActiveTrips>().0.is_empty(),
        "the trip must resolve within 2 world hours"
    );
    let states: Vec<CitizenState> = world
        .query::<&CitizenState>()
        .iter(&world)
        .copied()
        .collect();
    assert_eq!(states, vec![CitizenState::AtWork]);
    let cons = *world.resource::<Conservation>();
    assert_eq!(cons.spawned, 1, "exactly the one citizen car was spawned");
    assert_eq!(cons.arrived, 1, "and it arrived (booked in conservation)");
    assert_eq!(world.resource::<CoreRes>().0.fleet.alive_count(), 0);
}
