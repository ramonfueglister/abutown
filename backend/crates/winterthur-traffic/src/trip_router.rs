//! CH-backed [`TripRouter`]-Adapter für die Bürger-Trip-Brücke (Task 9).
//!
//! world-core kennt nur das schmale `TripRouter`-Trait (Edge→Edge); die
//! konkrete Auflösung Edge → fahrbare **Lane-Folge** passiert hier über den
//! bestehenden [`Router`] (CH auf dem Edge-Graphen mit Lane-Expansion —
//! edge-id ≠ lane-id, die Expansion ist die Phase-7a-Lektion in Code).
//!
//! Der Adapter besitzt seinen EIGENEN `Router` (frisch aus dem Netz gebaut)
//! plus einen Netz-Klon: die Shell-`RouterRes` bekommt laufend Mess-Gewichte
//! (`update_weights` + `rebuild`) und lebt exklusiv im ECS. Bewusster
//! M1-Schnitt: Bürger-Routen fahren auf Free-Flow-Gewichten und reagieren
//! nicht auf Live-Stau (das Congestion-Re-Routing der Shell greift für
//! Bürger-Autos ohnehin nicht — sie stehen nicht in der `TripRegistry`).

use traffic_net::TrafficNet;
use world_core::{CarRoute, TripRouter};

use crate::Router;

/// Sicherheitsabstand (m) vom Lane-Ende für den Start-Offset: ein Spawn
/// exakt am Lane-Ende würde den Route-Cursor sofort über die Kante tragen.
const START_END_MARGIN_M: f32 = 1.0;

/// CH-Adapter: Edge→Edge-Anfrage → konkrete Lane-Route + Start-Offset.
pub struct ChTripRouter {
    router: Router,
    net: TrafficNet,
}

impl ChTripRouter {
    /// Baut den Adapter über einem Netz (eigener CH-Prepare, eigener Klon).
    pub fn new(net: &TrafficNet) -> ChTripRouter {
        ChTripRouter {
            router: Router::new(net),
            net: net.clone(),
        }
    }
}

impl TripRouter for ChTripRouter {
    /// `to_offset` wird in M1 ignoriert (das Fahrzeug fährt bis ans Ende der
    /// Ziel-Edge und die Brücke löst dort die Ankunft auf); der Parameter ist
    /// Teil des Trait-Kontrakts für spätere feinere Ziel-Platzierung.
    fn route_between_edges(
        &self,
        from_edge: u32,
        from_offset: f32,
        to_edge: u32,
        _to_offset: f32,
    ) -> Option<CarRoute> {
        let lanes = self.router.route(&self.net, from_edge, to_edge)?;
        // Lanes eines validierten Bakes sind dicht id == index (Core::new
        // asserted das); der Offset gilt entlang der Edge-Bogenlänge, die
        // alle Lanes der Edge teilen.
        let first = *lanes.first()?;
        let len = self.net.lanes[first as usize].length_m;
        let start_s = from_offset.clamp(0.0, (len - START_END_MARGIN_M).max(0.0));
        Some(CarRoute { lanes, start_s })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_net() -> TrafficNet {
        let p = format!(
            "{}/tests/fixtures/diamond-gateway.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"));
        traffic_net::load(&json).expect("fixture must validate")
    }

    #[test]
    fn expands_edges_to_drivable_lanes_with_clamped_start() {
        let net = fixture_net();
        let adapter = ChTripRouter::new(&net);

        let route = adapter
            .route_between_edges(0, 12.5, 5, 3.0)
            .expect("edge 0 → edge 5 must route");
        // Edge-Pfad 0→1→3→5, als konkrete Lane-Ids (CH-Baseline: upper path).
        assert_eq!(route.lanes, vec![0, 1, 3, 5]);
        assert!((route.start_s - 12.5).abs() < 1e-6);

        // Offset jenseits des Lane-Endes wird vor die Kante geklemmt.
        let clamped = adapter
            .route_between_edges(0, 500.0, 5, 0.0)
            .expect("route exists");
        assert!(clamped.start_s <= 99.0 + 1e-6);

        // Unbekannte Edge: sauber None (kein Panik-Pfad im Dispatch).
        assert!(adapter.route_between_edges(0, 0.0, 999, 0.0).is_none());
    }
}
