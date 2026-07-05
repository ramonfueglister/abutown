//! Trip-Brücke Bürger↔Verkehr (Task 9): [`TripRequests`] aus dem
//! Tagesrhythmus werden echte Autos im traffic-core-Kernel (lange Wege) oder
//! „Teleport nach Dauer" (kurze Wege — M1 hat kein Fussgänger-Routing, ein
//! Fussweg ist ein Timer mit Distanz / 1.4 m/s).
//!
//! # Architektur-Seams
//!
//! world-core darf NICHT von `winterthur-traffic` abhängen (das bindet
//! world-core ein — Zirkularität). Deshalb zwei schmale Abstraktionen:
//!
//!  * [`TripRouter`]: Routing von Strassen-**Edge** zu Strassen-Edge. Die
//!    konkrete Implementierung (CH-Router + edge→lane-Expansion — Achtung,
//!    edge-id ≠ lane-id!) lebt in `winterthur-traffic`; Tests injizieren
//!    einen Fixture-Router.
//!  * [`CoreAccess`]: Zugriff auf den [`Core`], ohne die Shell-Resource
//!    (`CoreRes`) zu kennen. Die Systeme sind über `C: CoreAccess` generisch;
//!    die Shell registriert `dispatch_trips_system::<CoreRes>`.
//!
//! # Ankunfts-Erkennung (Driving)
//!
//! Der Kernel despawnt ein Fahrzeug selbst, wenn seine **offene** Route am
//! letzten Lane-Ende ankommt (`route_completed` in traffic-core) — danach ist
//! `vehicle_view(veh) == None`. Das ist die primäre Ankunfts-Signatur.
//! Zusätzlich gilt „auf der Ziel-Edge am Lane-Ende" als Ankunft (manueller
//! [`Core::despawn`]): eine Route, deren letzte Lane einen Turn zurück auf
//! die erste Route-Edge hat (Zwei-Edge-Zyklus, z.B. Hin-/Rückrichtung einer
//! Strasse), gilt dem Kernel als Loop und würde sonst ewig kreisen.
//!
//! Slot-Reuse-Sicherheit: die Kette läuft pro Tick strikt
//! `dispatch → core_tick → arrivals`. Ein Slot, den der Kernel in
//! `core_tick(N)` freigibt, wird von `arrivals(N)` noch im selben Tick als
//! `None` beobachtet und der Trip entfernt — wiederverwendet werden kann der
//! Slot frühestens von einem Spawn in Tick N+1, wenn kein `ActiveTrips`-
//! Eintrag mehr auf ihn zeigt.
//!
//! # Walk-Fallback
//!
//! Scheitert das Routing (oder der Kernel-Spawn am Kapazitäts-Cap), fällt der
//! Trip auf einen Fussweg zurück — der EINZIGE erlaubte Fallback: Netz-Inseln
//! sind im echten Bake real (Gebäude an nicht angebundenen Stichstrassen),
//! ein Bürger darf deshalb nie stranden. Alles andere (fehlende Registry-
//! Einträge, Nicht-Commuting-Zustände bei Ankunft) ist ein Bug ⇒ fail loud.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use traffic_core::{Core, VehId};

use crate::TICKS_PER_SECOND;
use crate::citizens::rhythm::{TripRequests, nearest_market_to_building};
use crate::citizens::{CitizenRegistry, CitizenState, TripKind};
use crate::clock::WorldClock;
use crate::econ::{Markets, euclid_m};
use crate::systems::SharedSimWorld;

/// Ab dieser Luftlinien-Distanz (Meter) fährt ein Bürger Auto — sofern beide
/// Gebäude einen Strassen-Zugang (`access_edge >= 0`) haben.
pub const CAR_MIN_DIST_M: i64 = 800;

/// Fussgänger-Geschwindigkeit (m/s) für die Teleport-Dauer kurzer Wege.
pub const WALK_SPEED_MPS: f32 = 1.4;

/// Toleranz (m) vor dem Lane-Ende, ab der ein Fahrzeug auf der Ziel-Edge als
/// angekommen gilt (der Kernel hält Fahrzeuge an der Stop-Linie ~0.05 m vor
/// dem Ende — 1 m Marge fängt das robust).
const ARRIVE_EPS_M: f32 = 1.0;

/// Eine konkrete, fahrbare Auto-Route: Lane-Id-Folge (wie
/// [`Core::spawn`] sie erwartet) plus Start-Bogenposition auf `lanes[0]`.
#[derive(Debug, Clone, PartialEq)]
pub struct CarRoute {
    pub lanes: Vec<u32>,
    pub start_s: f32,
}

/// Routing-Seam: Strassen-Edge → Strassen-Edge (Offsets in Metern entlang
/// der Edge). `to_offset` ist Teil des Kontrakts für spätere feinere
/// Ziel-Platzierung; die M1-CH-Implementierung fährt bis ans Edge-Ende.
pub trait TripRouter: Send + Sync {
    fn route_between_edges(
        &self,
        from_edge: u32,
        from_offset: f32,
        to_edge: u32,
        to_offset: f32,
    ) -> Option<CarRoute>;
}

/// Der injizierte Router (CH-Adapter in Produktion, Fixture in Tests).
#[derive(Resource)]
pub struct TripRouterRes(pub Box<dyn TripRouter>);

/// Zugriff auf den Traffic-Kernel, ohne die konkrete Shell-Resource zu
/// kennen (siehe Modul-Doku).
pub trait CoreAccess: Resource {
    fn core(&self) -> &Core;
    fn core_mut(&mut self) -> &mut Core;
}

/// Eine laufende Bewegung eines Bürgers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTrip {
    /// Fährt als echtes Fahrzeug im Kernel.
    Driving { veh: VehId, dest_building: u32 },
    /// Geht zu Fuss; Ankunft, sobald `world_tick >= arrive_tick`. Trägt
    /// Start-Tick und Start-Gebäude, damit der Live-Kanal (Task 13) die
    /// Position linear from→to über die Trip-Dauer interpolieren kann.
    WalkingUntil {
        depart_tick: u64,
        arrive_tick: u64,
        from_building: u32,
        dest_building: u32,
    },
}

/// Laufende Trips, Bürger-Id → Bewegung. BTreeMap: deterministische
/// Iterationsreihenfolge im Sim-Pfad (Projektregel).
#[derive(Resource, Default)]
pub struct ActiveTrips(pub BTreeMap<u32, ActiveTrip>);

/// Monotone Zähler über Bürger-Autos, damit die Shell ihre
/// Fahrzeug-Konservierungs-Buchhaltung (`spawned == arrived + alive`) auch
/// über die Brücken-Spawns/-Despawns führen kann.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CitizenCarCounters {
    /// Von `dispatch_trips_system` in den Kernel gesetzte Fahrzeuge.
    pub spawned: u64,
    /// Von `arrivals_system` manuell am Ziel-Lane-Ende despawnte Fahrzeuge
    /// (Kernel-eigene End-of-Route-Despawns zählt die Shell bereits über
    /// `despawned_last_tick`).
    pub despawned_at_destination: u64,
}

/// Fussweg-Dauer in Ticks: `dist / 1.4 m/s`, aufgerundet (nie 0 Ticks für
/// eine echte Distanz).
pub fn walk_ticks(dist_m: i64) -> u64 {
    ((dist_m as f32 / WALK_SPEED_MPS) * TICKS_PER_SECOND as f32).ceil() as u64
}

/// Übersetzt jeden [`TripRequests`]-Eintrag in einen [`ActiveTrip`]:
/// Luftlinie > 800 m UND beide Gebäude mit Strassen-Zugang ⇒ Auto über den
/// [`TripRouter`]; sonst (oder bei Routing-/Spawn-Fehlschlag — der einzige
/// erlaubte Fallback, siehe Modul-Doku) Fussweg-Timer.
pub fn dispatch_trips_system<C: CoreAccess>(
    mut requests: ResMut<TripRequests>,
    mut trips: ResMut<ActiveTrips>,
    mut counters: ResMut<CitizenCarCounters>,
    clock: Res<WorldClock>,
    sim: Res<SharedSimWorld>,
    router: Res<TripRouterRes>,
    mut core: ResMut<C>,
) {
    for req in requests.0.drain(..) {
        let from = &sim.0.buildings[req.from_building as usize];
        let to = &sim.0.buildings[req.to_building as usize];
        let dist_m = euclid_m((from.x, from.z), (to.x, to.z));

        let drive = if dist_m > CAR_MIN_DIST_M && from.access_edge >= 0 && to.access_edge >= 0 {
            router
                .0
                .route_between_edges(
                    from.access_edge as u32,
                    from.access_offset,
                    to.access_edge as u32,
                    to.access_offset,
                )
                .and_then(|route| {
                    // `spawn` gibt bei Kapazitäts-Cap/inkonsistenter Route
                    // `None` zurück — Walk-Fallback, nie ein gestrandeter
                    // Bürger.
                    let veh = core
                        .core_mut()
                        .spawn(route.lanes[0], route.start_s, &route.lanes)?;
                    counters.spawned += 1;
                    Some(ActiveTrip::Driving {
                        veh,
                        dest_building: req.to_building,
                    })
                })
        } else {
            None
        };

        let trip = drive.unwrap_or(ActiveTrip::WalkingUntil {
            depart_tick: clock.world_tick,
            arrive_tick: clock.world_tick + walk_ticks(dist_m),
            from_building: req.from_building,
            dest_building: req.to_building,
        });
        let previous = trips.0.insert(req.citizen, trip);
        debug_assert!(
            previous.is_none(),
            "citizen {} dispatched while already travelling — rhythm double-emission bug",
            req.citizen
        );
    }
}

/// Löst Ankünfte auf: Driving-Fahrzeuge, die despawnt sind (Kernel-Route-
/// Ende) oder auf der Ziel-Edge am Lane-Ende stehen (manueller Despawn),
/// und abgelaufene Fussweg-Timer. Setzt den [`CitizenState`] gemäss dem
/// im `Commuting`-Zustand getragenen [`TripKind`] und entfernt den Eintrag.
#[allow(clippy::too_many_arguments)]
pub fn arrivals_system<C: CoreAccess>(
    mut trips: ResMut<ActiveTrips>,
    mut counters: ResMut<CitizenCarCounters>,
    clock: Res<WorldClock>,
    registry: Res<CitizenRegistry>,
    markets: Res<Markets>,
    sim: Res<SharedSimWorld>,
    mut core: ResMut<C>,
    mut states: Query<&mut CitizenState>,
) {
    // (citizen, dest_building, manuell zu despawnendes Fahrzeug)
    let mut done: Vec<(u32, u32, Option<VehId>)> = Vec::new();
    for (&citizen, trip) in &trips.0 {
        match *trip {
            ActiveTrip::Driving { veh, dest_building } => {
                match core.core().vehicle_view(veh) {
                    // Kernel hat die Route zu Ende gefahren und despawnt.
                    None => done.push((citizen, dest_building, None)),
                    // Auf der Ziel-Edge am Lane-Ende: angekommen (deckt
                    // Zwei-Edge-Zyklus-Routen ab, die der Kernel nie selbst
                    // beendet — siehe Modul-Doku).
                    Some(view) => {
                        let dest_edge = sim.0.buildings[dest_building as usize].access_edge;
                        if i64::from(view.edge) == dest_edge
                            && view.s >= core.core().lane_len(view.lane) - ARRIVE_EPS_M
                        {
                            done.push((citizen, dest_building, Some(veh)));
                        }
                    }
                }
            }
            ActiveTrip::WalkingUntil {
                arrive_tick,
                dest_building,
                ..
            } => {
                if clock.world_tick >= arrive_tick {
                    done.push((citizen, dest_building, None));
                }
            }
        }
    }

    for (citizen, dest_building, manual_despawn) in done {
        if let Some(veh) = manual_despawn {
            let removed = core.core_mut().despawn(veh);
            debug_assert!(removed, "arrival despawn of a dead slot");
            counters.despawned_at_destination += 1;
        }
        trips.0.remove(&citizen);

        let entity = *registry
            .by_id
            .get(&citizen)
            .expect("ActiveTrips citizen missing from CitizenRegistry — trip/registry desync bug");
        let mut state = states
            .get_mut(entity)
            .expect("registry entity without CitizenState component");
        let CitizenState::Commuting { trip: kind } = *state else {
            unreachable!(
                "citizen {citizen} arrived while not Commuting ({:?}) — rhythm/trips desync bug",
                *state
            );
        };
        *state = match kind {
            TripKind::ToWork => CitizenState::AtWork,
            TripKind::ToHome => CitizenState::AtHome,
            TripKind::ToMarket => CitizenState::AtMarket {
                // DERSELBE Helper wie die Rhythmus-Emission (Kontrakt in
                // `nearest_market_to_building`s Doku).
                market: nearest_market_to_building(&markets, &sim.0, dest_building),
            },
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_ticks_is_ceiled_distance_over_speed() {
        // 100 m / 1.4 m/s = 71.43 s → 714.3 Ticks → 715.
        assert_eq!(walk_ticks(100), 715);
        // 0 m (Markt am Arbeitsplatz): sofort fällig.
        assert_eq!(walk_ticks(0), 0);
        // 1.4 m/s über 7 m sind exakt 50 Ticks (kein Aufrunden nötig).
        assert_eq!(walk_ticks(7), 50);
    }
}
