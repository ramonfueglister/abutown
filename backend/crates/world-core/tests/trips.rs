//! Task 9 integration proof: Bürger-Trips werden echter Verkehr.
//!
//! Ein langer Weg (> 800 m, beide Gebäude mit Strassen-Zugang) spawnt ein
//! echtes Fahrzeug im traffic-core-Kernel und die Ankunft löst den
//! `CitizenState` auf; ein kurzer Weg wird als `WalkingUntil` mit
//! deterministischer Dauer (Distanz / 1.4 m/s) teleportiert.
//!
//! Der `TripRouter` ist hier ein Fixture-Router (direkte Lane-Folge im
//! diamond-gateway-Netz, siehe `tests/fixtures/`) — der echte CH-Adapter
//! lebt in `winterthur-traffic` (world-core darf nicht davon abhängen).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;
use traffic_core::Core;
use world_core::citizens::rhythm::TripRequest;
use world_core::citizens::trips::{
    ActiveTrip, ActiveTrips, CarRoute, CoreAccess, TripRouter, TripRouterRes, dispatch_trips_system,
};
use world_core::econ::EconomySeed;
use world_core::{
    Citizen, CitizenState, SeedParams, SimWorld, TripKind, TripRequests, WorldClock,
    WorldCorePlugin, arrivals_system, install_world_resources,
};

/// Das REALE authored economy.json (Markets-Resource für die
/// ToMarket-Auflösung in `arrivals_system`).
const ECONOMY_JSON: &str = include_str!("../../../../data/winterthur/economy.json");

/// Ein Wohnhaus mit genau 1 Bewohner (40 m² × 1 Geschoss) am Anfang des
/// diamond-gateway-Netzes und ein Arbeitsplatz `dist_m` Meter entfernt am
/// Netz-Ende. `access_edge` 0 (Home) und 5 (Work) sind die Gateway-Kanten
/// des Fixtures.
fn simworld_json(dist_m: f32) -> String {
    format!(
        r#"{{
  "meta": {{"anchor": {{"lon": 8.7285, "lat": 47.5069}}, "bake_version": 1}},
  "buildings": [
    {{"id":"{{H1}}","usage":1,"x":0.0,"z":0.0,"area_m2":40.0,"height_m":3.0,"access_edge":0,"access_offset":5.0}},
    {{"id":"{{W2}}","usage":2,"x":{dist_m},"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":5,"access_offset":50.0}}
  ]}}"#
    )
}

/// Test-Wrapper um den Kernel: world-core kennt die Shell-Resource `CoreRes`
/// nicht, nur den [`CoreAccess`]-Seam.
#[derive(Resource)]
struct TestCore(Core);

impl CoreAccess for TestCore {
    fn core(&self) -> &Core {
        &self.0
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.0
    }
}

/// Fixture-Router: kennt genau die eine Route des diamond-gateway-Netzes
/// (Edge-Pfad 0→1→3→5, Lane-Ids zufällig deckungsgleich — die edge→lane-
/// Auflösung des echten CH-Adapters wird in winterthur-traffic getestet).
struct FixtureRouter;

impl TripRouter for FixtureRouter {
    fn route_between_edges(
        &self,
        from_edge: u32,
        from_offset: f32,
        to_edge: u32,
        _to_offset: f32,
    ) -> Option<CarRoute> {
        assert_eq!((from_edge, to_edge), (0, 5), "fixture knows one route");
        Some(CarRoute {
            lanes: vec![0, 1, 3, 5],
            start_s: from_offset,
        })
    }
}

/// Kernel-Tick als System, damit die Kette dem Shell-Muster entspricht:
/// dispatch → core_tick → arrivals.
fn tick_core(mut core: ResMut<TestCore>, clock: Res<WorldClock>) {
    let t = clock.world_tick;
    core.0.tick(t);
}

fn build_world(dist_m: f32) -> (World, Schedule) {
    let json = include_str!("fixtures/diamond-gateway.json");
    let net = traffic_net::load(json).expect("diamond-gateway fixture must validate");
    let core = Core::new(&net, 64, 7);

    let sim = Arc::new(SimWorld::load(&simworld_json(dist_m)).expect("fixture must load"));
    let plugin = WorldCorePlugin {
        seed: EconomySeed::from_json(ECONOMY_JSON).expect("economy.json must parse"),
        sim_world: sim,
        seed_params: SeedParams {
            center: (0.0, 0.0),
            radius_m: 10_000.0,
            residents_per_40m2: 1.0,
            seed: 42,
        },
    };

    let mut world = World::new();
    install_world_resources(&mut world, &plugin);
    world.insert_resource(TestCore(core));
    world.insert_resource(TripRouterRes(Box::new(FixtureRouter)));

    let mut schedule = Schedule::default();
    schedule.add_systems(
        (
            world_core::advance_world_clock_system,
            dispatch_trips_system::<TestCore>,
            tick_core,
            arrivals_system::<TestCore>,
        )
            .chain(),
    );
    (world, schedule)
}

/// Der eine geseedete Bürger (id 0, home {H1}=0, work {W2}=1), manuell auf
/// Commuting gesetzt plus der zugehörige ToWork-Request (das Emissionsmuster
/// des Tagesrhythmus, ohne dessen Fahrplan-Jitter im Test).
fn request_to_work(world: &mut World) {
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Citizen>>()
        .iter(world)
        .collect();
    assert_eq!(entities.len(), 1, "fixture seeds exactly one citizen");
    *world.get_mut::<CitizenState>(entities[0]).unwrap() = CitizenState::Commuting {
        trip: TripKind::ToWork,
    };
    world.resource_mut::<TripRequests>().0.push(TripRequest {
        citizen: 0,
        from_building: 0,
        to_building: 1,
        kind: TripKind::ToWork,
    });
}

fn only_state(world: &mut World) -> CitizenState {
    let states: Vec<CitizenState> = world
        .query::<&CitizenState>()
        .iter(world)
        .copied()
        .collect();
    assert_eq!(states.len(), 1);
    states[0]
}

#[test]
fn long_trip_drives_a_real_vehicle_and_arrives_at_work() {
    // 900 m Luftlinie > 800 m ⇒ Auto.
    let (mut world, mut schedule) = build_world(900.0);
    request_to_work(&mut world);

    // Tick 1: dispatch spawnt das Fahrzeug.
    schedule.run(&mut world);
    {
        let trips = world.resource::<ActiveTrips>();
        assert!(
            matches!(trips.0.get(&0), Some(ActiveTrip::Driving { .. })),
            "long trip must become a Driving entry, got {:?}",
            trips.0.get(&0)
        );
        assert_eq!(
            world.resource::<TestCore>().0.fleet.alive_count(),
            1,
            "exactly one vehicle must exist in the core"
        );
        assert!(
            world.resource::<TripRequests>().0.is_empty(),
            "dispatch must drain the request queue"
        );
    }

    // Route = 4 Lanes à 100 m ab s=5 bei ≤ v0: grosszügig zu Ende ticken.
    let mut arrived_at = None;
    for tick in 2..=5_000u64 {
        schedule.run(&mut world);
        if world.resource::<ActiveTrips>().0.is_empty() {
            arrived_at = Some(tick);
            break;
        }
    }
    let arrived_at = arrived_at.expect("the drive must complete within 5000 ticks");
    assert!(arrived_at > 100, "400 m cannot be driven in 10 s");

    assert_eq!(only_state(&mut world), CitizenState::AtWork);
    assert_eq!(
        world.resource::<TestCore>().0.fleet.alive_count(),
        0,
        "the arrived vehicle must be despawned"
    );
}

#[test]
fn short_trip_walks_with_correct_duration() {
    // 100 m < 800 m ⇒ Fussweg: 100 / 1.4 m/s = 71.43 s → 715 Ticks (ceil).
    let (mut world, mut schedule) = build_world(100.0);
    request_to_work(&mut world);

    // Tick 1 (WorldClock steht danach auf 1): dispatch legt den Walk an.
    schedule.run(&mut world);
    {
        let trips = world.resource::<ActiveTrips>();
        assert_eq!(
            trips.0.get(&0),
            Some(&ActiveTrip::WalkingUntil {
                depart_tick: 1,
                arrive_tick: 1 + 715,
                from_building: 0,
                dest_building: 1
            }),
            "short trip must walk with duration dist/1.4 m/s in ticks"
        );
        assert_eq!(
            world.resource::<TestCore>().0.fleet.alive_count(),
            0,
            "no vehicle may spawn for a walk"
        );
    }

    // Bis kurz vor der Ankunft: noch unterwegs.
    while world.resource::<WorldClock>().world_tick < 715 {
        schedule.run(&mut world);
    }
    assert!(matches!(
        only_state(&mut world),
        CitizenState::Commuting { .. }
    ));

    // Tick 716 erreicht arrive_tick ⇒ Ankunft.
    schedule.run(&mut world);
    assert!(world.resource::<ActiveTrips>().0.is_empty());
    assert_eq!(only_state(&mut world), CitizenState::AtWork);
}
