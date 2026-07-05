//! Bürger der persistenten Welt (Task 7): deterministisches Seeding in echte
//! Gebäude. Jeder Bürger wohnt in einem konkreten Wohngebäude und arbeitet in
//! einem konkreten Workplace-Gebäude — Entitäten als Wahrheit, keine
//! Schatten-Populationen.
//!
//! Determinismus-Kontrakt: Bürger-IDs laufen fortlaufend `0..` in
//! BuildingId-Reihenfolge (die `SimWorld`-Sortierung nach UUID macht sie über
//! Bakes hinweg stabil); jeder Zufalls-Draw ist `traffic_core::u01` — eine
//! pure Funktion von `(seed, citizen_id, kontext)`, nie ein stateful RNG.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::econ::{HouseholdSector, MarketId, euclid_m};
use crate::model::SimWorld;

/// Kontext-Konstante für den Arbeitsplatz-Draw (drittes `u01`-Argument).
const WORK_DRAW_CONTEXT: u64 = 0xB0B;

/// Über wie viele nächstgelegene Workplaces der Arbeitsplatz-Draw streut.
const WORK_CANDIDATES: usize = 8;

/// Wieviel Wohnfläche ein Bewohner belegt (Kapazität = Fläche·Geschosse / 40).
const M2_PER_RESIDENT: f32 = 40.0;

/// Grund einer laufenden Pendel-Bewegung. Formal Teil des Tagesrhythmus
/// (Task 8), aber hier definiert, weil `CitizenState::Commuting` ihn trägt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TripKind {
    ToWork,
    ToMarket,
    ToHome,
}

/// Ein Bürger: Identität + Wohn-/Arbeitsgebäude (BuildingId-Indizes).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Citizen {
    pub id: u32,
    pub home: u32,
    pub work: u32,
}

/// Wo der Bürger gerade ist bzw. was er tut.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CitizenState {
    AtHome,
    AtWork,
    AtMarket { market: MarketId },
    Commuting { trip: TripKind },
}

/// Bürger-Verzeichnis: Kopfzahl + deterministischer Index Bürger-ID → Entity.
#[derive(Resource, Default)]
pub struct CitizenRegistry {
    pub count: u64,
    pub by_id: BTreeMap<u32, Entity>,
}

/// Seeding-Parameter (Stadtteil-Radius um ein Zentrum in lokalen Metern).
/// Auch als Resource eingefügt, damit der Tagesrhythmus (Task 8) denselben
/// `seed` für seine deterministischen Draws liest.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct SeedParams {
    pub center: (f32, f32),
    pub radius_m: f32,
    /// Skaliert die Wohnkapazität (1.0 = ein Bewohner pro 40 m² Geschossfläche).
    pub residents_per_40m2: f32,
    pub seed: u64,
}

/// Seedet Bürger in alle Wohngebäude im Radius. Idempotent: eine nicht-leere
/// Registry ⇒ no-op (Hydrate-Pfad-Sicherheit, Lehre aus #86). Gibt die
/// Kopfzahl zurück und koppelt sie in `HouseholdSector.population` (die
/// Wirtschaft muss VOR den Bürgern installiert sein — `install_world_systems`
/// garantiert die Reihenfolge, ein Fehlen der Resource ist ein Boot-Bug).
pub fn seed_citizens(world: &mut World, sim: &SimWorld, p: &SeedParams) -> u64 {
    if let Some(registry) = world.get_resource::<CitizenRegistry>()
        && registry.count > 0
    {
        return registry.count;
    }
    world.init_resource::<CitizenRegistry>();

    // Wohngebäude im Radius, aufsteigend nach BuildingId (deterministisch).
    let in_radius = sim.within_radius(p.center.0, p.center.1, p.radius_m);
    let homes: Vec<u32> = in_radius
        .into_iter()
        .filter(|&b| sim.residential().contains(&b))
        .collect();

    // Plan erst vollständig berechnen, dann spawnen (kein Iterieren über
    // `sim` während Entity-Mutation nötig, und die IDs bleiben fortlaufend).
    let mut plan: Vec<Citizen> = Vec::new();
    let mut next_id: u32 = 0;
    for &home in &homes {
        let b = &sim.buildings[home as usize];
        let floors = (b.height_m / 3.0).round().max(1.0);
        let capacity = (b.area_m2 * floors / M2_PER_RESIDENT).floor();
        let residents = (capacity * p.residents_per_40m2).floor().max(0.0) as u32;
        for _ in 0..residents {
            let work = draw_workplace(sim, home, next_id, p.seed);
            plan.push(Citizen {
                id: next_id,
                home,
                work,
            });
            next_id += 1;
        }
    }

    let mut by_id = BTreeMap::new();
    for citizen in plan {
        let id = citizen.id;
        let entity = world.spawn((citizen, CitizenState::AtHome)).id();
        by_id.insert(id, entity);
    }

    let count = by_id.len() as u64;
    let mut registry = world.resource_mut::<CitizenRegistry>();
    registry.count = count;
    registry.by_id = by_id;

    // Wirtschafts-Kopplung: die Bürger SIND der Haushaltssektor.
    world.resource_mut::<HouseholdSector>().population = count;
    count
}

/// Arbeitsplatz-Draw: über die (bis zu) 8 nächstgelegenen Workplaces zum
/// Wohngebäude, gewichtet mit Distanz-Rang-Gewichten 8..1 (nächster = 8),
/// gezogen mit `u01(seed, citizen_id, 0xB0B)`. Distanz-Ties brechen auf die
/// kleinere BuildingId (deterministisch). Keine Workplaces im Bake wäre ein
/// Authoring-Fehler ⇒ fail loud.
fn draw_workplace(sim: &SimWorld, home: u32, citizen_id: u32, seed: u64) -> u32 {
    let hb = &sim.buildings[home as usize];
    let mut candidates: Vec<(i64, u32)> = sim
        .workplaces()
        .iter()
        .map(|&w| {
            let wb = &sim.buildings[w as usize];
            (euclid_m((hb.x, hb.z), (wb.x, wb.z)), w)
        })
        .collect();
    assert!(
        !candidates.is_empty(),
        "seed_citizens: SimWorld has no workplace buildings — invalid bake"
    );
    candidates.sort_unstable();
    candidates.truncate(WORK_CANDIDATES);

    // Rang-Gewichte 8, 7, …, 1 über die tatsächliche Kandidatenzahl.
    let weights: Vec<u32> = (0..candidates.len())
        .map(|rank| (WORK_CANDIDATES - rank) as u32)
        .collect();
    let total: u32 = weights.iter().sum();
    let r = traffic_core::u01(seed, u64::from(citizen_id), WORK_DRAW_CONTEXT);
    let mut threshold = r * total as f32;
    for (weight, (_, w)) in weights.iter().zip(&candidates) {
        threshold -= *weight as f32;
        if threshold < 0.0 {
            return *w;
        }
    }
    // r < 1.0 ⇒ die Schleife trifft immer; Float-Randfall fällt auf den
    // letzten Kandidaten (deterministisch, kein Bias von Belang).
    candidates.last().expect("non-empty by assert above").1
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy_ecs::schedule::Schedule;

    use super::*;
    use crate::econ::seed::EconomySeed;
    use crate::model::test_fixture::FIXTURE;
    use crate::systems::{WorldCorePlugin, install_world_systems};

    const ECONOMY_JSON: &str = include_str!("../../../../../data/winterthur/economy.json");

    fn build_world() -> World {
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
        world
    }

    fn triples(world: &mut World) -> Vec<(u32, u32, u32)> {
        let mut v: Vec<_> = world
            .query::<&Citizen>()
            .iter(world)
            .map(|c| (c.id, c.home, c.work))
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn fixture_seeds_fifteen_citizens_into_b1_working_at_a2() {
        // Fixture-B1: 200 m², 9 m → floors = 3 → Kapazität 15. Sortierung
        // nach UUID: {A2}=0 (einziger Workplace), {B1}=1, {C3}=2.
        let mut world = build_world();
        let t = triples(&mut world);
        assert_eq!(t.len(), 15);
        for (i, &(id, home, work)) in t.iter().enumerate() {
            assert_eq!(id, i as u32, "citizen ids run 0.. in building order");
            assert_eq!(home, 1, "all live in {{B1}}");
            assert_eq!(work, 0, "only workplace is {{A2}}");
        }
        let registry = world.resource::<CitizenRegistry>();
        assert_eq!(registry.count, 15);
        assert_eq!(registry.by_id.len(), 15);
        assert_eq!(
            world.resource::<HouseholdSector>().population,
            15,
            "citizens are the household sector's head-count"
        );
    }

    #[test]
    fn seeding_is_deterministic_across_fresh_worlds() {
        let mut a = build_world();
        let mut b = build_world();
        assert_eq!(triples(&mut a), triples(&mut b));
    }

    #[test]
    fn second_seed_call_is_a_no_op() {
        let mut world = build_world();
        let sim = Arc::clone(&world.resource::<crate::systems::SharedSimWorld>().0);
        let params = *world.resource::<SeedParams>();
        let count = seed_citizens(&mut world, &sim, &params);
        assert_eq!(count, 15, "idempotent re-seed returns the existing count");
        assert_eq!(world.resource::<CitizenRegistry>().count, 15);
        assert_eq!(triples(&mut world).len(), 15, "no duplicate citizens");
    }
}
