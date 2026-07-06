//! Tagesrhythmus auf dem 4h-Weltentag (Task 8): Bürger emittieren zu festen
//! Weltzeiten Trip-Wünsche, die Task 9 in echten Verkehr übersetzt.
//!
//! Fahrplan (Welt-Sekunde des Tages, pro Bürger deterministisch gejittert um
//! ±45 Welt-Minuten): 07:30 → ToWork, 12:00 → ToMarket (20 % der Bürger,
//! Ziel = nächster Markt zum Arbeitsplatz), 13:00 → zurück zur Arbeit,
//! 17:30 → ToHome.
//!
//! Emission ist NACHHOLEND, nicht flankengetriggert: ein Trip ist fällig,
//! sobald `s_of_world_day >= geplante Zeit` UND der Bürger noch im
//! Ausgangszustand ist. Damit verpasst weder ein Server-Resume mitten im
//! Fenster noch ein früher Jitter-Slot einen Trip — und der Wechsel auf
//! `CitizenState::Commuting` bei der Emission garantiert genau EINEN Request
//! pro Zustand (keine Doppel-Emission über Ticks hinweg). Nach Mitternacht
//! springt `s_of_world_day` auf 0 zurück; ein noch nicht abgereister Bürger
//! wird schlicht am Folgetag wieder fällig.

use bevy_ecs::prelude::*;
use traffic_core::u01;

use crate::citizens::{Citizen, CitizenState, SeedParams, TripKind};
use crate::clock::WorldClock;
use crate::econ::{MarketId, Markets, euclid_m};
use crate::model::SimWorld;
use crate::systems::SharedSimWorld;

/// Ausstehende Trip-Wünsche; Task 9 (Trip-Brücke) konsumiert und leert sie.
#[derive(Resource, Default)]
pub struct TripRequests(pub Vec<TripRequest>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TripRequest {
    pub citizen: u32,
    pub from_building: u32,
    pub to_building: u32,
    pub kind: TripKind,
}

/// Fahrplan-Slots in Welt-Sekunden des Tages.
const S_TO_WORK: i64 = 7 * 3600 + 30 * 60; // 07:30
const S_TO_MARKET: i64 = 12 * 3600; // 12:00
const S_BACK_TO_WORK: i64 = 13 * 3600; // 13:00
const S_TO_HOME: i64 = 17 * 3600 + 30 * 60; // 17:30

/// Jitter-Halbbreite: ±45 Welt-Minuten.
const JITTER_HALF_S: f32 = 45.0 * 60.0;

/// Slot-Salts für die Jitter-Draws (`u01(seed, citizen_id, world_day ^ salt)`),
/// disjunkt von 0 (= Markt-Teilnahme-Draw) und untereinander.
const SALT_WORK: u64 = 0x5107_0001;
const SALT_MARKET: u64 = 0x5107_0002;
const SALT_BACK: u64 = 0x5107_0003;
const SALT_HOME: u64 = 0x5107_0004;

/// Anteil der Bürger, die mittags zum Markt gehen.
const MARKET_SHARE: f32 = 0.2;

/// Geplante Slot-Zeit eines Bürgers an einem Welttag: Basiszeit ± Jitter,
/// deterministisch in `(seed, citizen_id, world_day, slot)`.
fn jittered_s(seed: u64, citizen_id: u32, world_day: u64, base_s: i64, salt: u64) -> i64 {
    let r = u01(seed, u64::from(citizen_id), world_day ^ salt);
    base_s + ((r - 0.5) * 2.0 * JITTER_HALF_S) as i64
}

/// Der nächste Markt (euklidisch, Meter) zum gegebenen Gebäude; Ties brechen
/// auf die kleinere `MarketId` (BTreeMap-Ordnung + strikt-kleiner). Task 9
/// löst die Ankunft eines ToMarket-Trips über DENSELBEN Helper in
/// `CitizenState::AtMarket { market }` auf — deshalb trägt weder `TripKind`
/// noch `TripRequest` die MarketId. Eine Welt ohne Märkte ist ein Seed-Bug.
pub fn nearest_market_to_building(markets: &Markets, sim: &SimWorld, building: u32) -> MarketId {
    let b = &sim.buildings[building as usize];
    markets
        .0
        .values()
        .min_by_key(|site| (euclid_m((b.x, b.z), (site.x, site.z)), site.id))
        .map(|site| site.id)
        .expect("nearest_market_to_building: economy seeded without markets — invalid seed")
}

/// Läuft JEDEN Tick (nicht econ-cadenced), nach `advance_world_clock`, vor
/// der Econ-Kette. Vereinfachung M1: ein Markt ist KEIN Gebäude — sowohl der
/// ToMarket-Hinweg als auch der 13:00-Rückweg werden gebäudeseitig am
/// Arbeitsplatz verankert (`from_building`/`to_building` = work); der Trip
/// modelliert den kurzen Marktbesuch rund um den Arbeitsplatz.
pub fn rhythm_system(
    clock: Res<WorldClock>,
    params: Res<SeedParams>,
    markets: Res<Markets>,
    sim: Res<SharedSimWorld>,
    mut requests: ResMut<TripRequests>,
    mut citizens: Query<(&Citizen, &mut CitizenState)>,
) {
    let s = i64::from(clock.s_of_world_day());
    let day = clock.world_day();
    let seed = params.seed;

    for (citizen, mut state) in &mut citizens {
        let due = |base: i64, salt: u64| s >= jittered_s(seed, citizen.id, day, base, salt);
        match *state {
            CitizenState::AtHome => {
                if due(S_TO_WORK, SALT_WORK) {
                    requests.0.push(TripRequest {
                        citizen: citizen.id,
                        from_building: citizen.home,
                        to_building: citizen.work,
                        kind: TripKind::ToWork,
                    });
                    *state = CitizenState::Commuting {
                        trip: TripKind::ToWork,
                    };
                }
            }
            CitizenState::AtWork => {
                // Feierabend gewinnt gegen einen (nachholend) noch offenen
                // Marktbesuch: wer um 17:30 noch im Büro sitzt, geht heim.
                if due(S_TO_HOME, SALT_HOME) {
                    requests.0.push(TripRequest {
                        citizen: citizen.id,
                        from_building: citizen.work,
                        to_building: citizen.home,
                        kind: TripKind::ToHome,
                    });
                    *state = CitizenState::Commuting {
                        trip: TripKind::ToHome,
                    };
                } else if u01(seed, u64::from(citizen.id), day) < MARKET_SHARE
                    && due(S_TO_MARKET, SALT_MARKET)
                    && !due(S_BACK_TO_WORK, SALT_BACK)
                {
                    // Marktfenster = [Hinweg-Slot, Rückweg-Slot); kreuzen sich
                    // die Jitter (später Hinweg, früher Rückweg), fällt der
                    // Besuch an diesem Tag schlicht aus — deterministisch.
                    let market = nearest_market_to_building(&markets, &sim.0, citizen.work);
                    debug_assert!(markets.0.contains_key(&market));
                    requests.0.push(TripRequest {
                        citizen: citizen.id,
                        from_building: citizen.work,
                        to_building: citizen.work,
                        kind: TripKind::ToMarket,
                    });
                    *state = CitizenState::Commuting {
                        trip: TripKind::ToMarket,
                    };
                }
            }
            CitizenState::AtMarket { .. } => {
                if due(S_BACK_TO_WORK, SALT_BACK) {
                    requests.0.push(TripRequest {
                        citizen: citizen.id,
                        from_building: citizen.work,
                        to_building: citizen.work,
                        kind: TripKind::ToWork,
                    });
                    *state = CitizenState::Commuting {
                        trip: TripKind::ToWork,
                    };
                }
            }
            // Unterwegs wird nichts Neues emittiert — die Auflösung der
            // Ankunft (zurück in einen ruhenden Zustand) ist Task 9.
            CitizenState::Commuting { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use bevy_ecs::schedule::Schedule;

    use super::*;
    use crate::econ::seed::EconomySeed;
    use crate::model::test_fixture::FIXTURE;
    use crate::systems::{WorldCorePlugin, install_world_systems};

    const ECONOMY_JSON: &str = include_str!("../../../../../data/winterthur/economy.json");

    fn build_world() -> (World, Schedule) {
        let sim = Arc::new(SimWorld::load(FIXTURE).expect("fixture must load"));
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
        let mut schedule = Schedule::default();
        install_world_systems(&mut world, &mut schedule, &plugin);
        (world, schedule)
    }

    #[test]
    fn all_fixture_citizens_emit_exactly_one_to_work_request() {
        let (mut world, mut schedule) = build_world();
        // 07:29:00 Weltzeit: world_seconds = 26_940 → world_tick = 26_940·10/6.
        world.resource_mut::<WorldClock>().world_tick = 44_900;
        assert_eq!(world.resource::<WorldClock>().s_of_world_day(), 26_940);

        // 2 Welt-Stunden = 7200 Welt-Sekunden = 1200 reale Sekunden = 12_000
        // Ticks. Das überdeckt das ganze Jitter-Fenster 06:45–08:15 — frühe
        // Slots (< 07:29) werden dank nachholender Emission sofort im ersten
        // Tick fällig, späte spätestens um 08:15.
        for _ in 0..12_000 {
            schedule.run(&mut world);
        }

        let requests = world.resource::<TripRequests>();
        assert_eq!(requests.0.len(), 15, "exactly one request per citizen");
        let mut per_citizen: BTreeMap<u32, u32> = BTreeMap::new();
        for req in &requests.0 {
            assert_eq!(req.kind, TripKind::ToWork);
            assert_eq!(req.from_building, 1, "from home {{B1}}");
            assert_eq!(req.to_building, 0, "to work {{A2}}");
            *per_citizen.entry(req.citizen).or_default() += 1;
        }
        assert_eq!(per_citizen.len(), 15);
        assert!(per_citizen.values().all(|&n| n == 1));

        for state in world.query::<&CitizenState>().iter(&world) {
            assert_eq!(
                *state,
                CitizenState::Commuting {
                    trip: TripKind::ToWork
                }
            );
        }
    }

    #[test]
    fn at_work_citizens_head_home_after_1730() {
        let (mut world, mut schedule) = build_world();
        // Alle manuell auf AtWork setzen (Ankunfts-Auflösung ist Task 9).
        let entities: Vec<Entity> = world
            .query_filtered::<Entity, With<Citizen>>()
            .iter(&world)
            .collect();
        for e in entities {
            *world.get_mut::<CitizenState>(e).unwrap() = CitizenState::AtWork;
        }
        // 16:44 Weltzeit (vor jedem 17:30±45min-Slot), dann 2 Welt-Stunden.
        let s: u64 = 16 * 3600 + 44 * 60;
        world.resource_mut::<WorldClock>().world_tick = s * 10 / 6;
        for _ in 0..12_000 {
            schedule.run(&mut world);
        }
        let requests = world.resource::<TripRequests>();
        let home: Vec<_> = requests
            .0
            .iter()
            .filter(|r| r.kind == TripKind::ToHome)
            .collect();
        assert_eq!(home.len(), 15, "everyone heads home exactly once");
        assert!(home.iter().all(|r| r.from_building == 0));
        assert!(home.iter().all(|r| r.to_building == 1));
    }
}
