//! `WorldCoreSnapshot` v1 + Migrationskette (Task 10): das EINE persistierte
//! Abbild der Welt — Uhr, Bürger, Gebäude-Lebenszyklen, Wirtschaft. Spiegel
//! des geernteten `EconomyPersistSnapshot` (bbd0159, sim-core), aber auf dem
//! M1-Weltmodell: Maps als sortierte `Vec<(K, V)>` (serde_json lehnt
//! Nicht-String-Map-Keys ab; `BTreeMap`-Iteration hält die Ordnung
//! byte-stabil), `ledger_tail` = die letzten [`PERSISTED_LEDGER_TAIL`]
//! Events.
//!
//! BEWUSST NICHT im Snapshot:
//!  * `MarketDistances` — pure Funktion der Markt-Positionen, wird in
//!    [`apply`] aus `markets` neu gerechnet ([`euclid_m`], exakt die
//!    Seed-Formel). Kein zweiter Wahrheitsort.
//!  * `MarketChunks`/LOD — existiert in M1 nicht (Märkte schlafen nie).
//!  * Ephemere Econ-Resources (`FlowShipments`, `FlowRateEwma`,
//!    `RealizedFlows`, `LastTickMoney`, `SellerReceipts`, `BuyerOutlays`,
//!    `NextShipmentId`, `WageTelemetry`, `DirtyMarketGoods`) — pro Runde neu
//!    aufgebaut bzw. rekonvergierend, dokumentiert an ihren Definitionen.
//!  * Authored Config (`EconomyConfig`-Ramp, `ProducerPolicies`,
//!    `SeedParams`) — wird bei JEDEM Boot vor dem Seed-Guard neu angewandt
//!    (#83-Lehre in `seed_economy`).
//!
//! # No-Wipe-Prinzip
//!
//! Jede künftige Schema-Änderung erhöht [`WORLD_SNAPSHOT_VERSION`] und fügt
//! in [`migrate_snapshot`] einen Arm hinzu, der die alte JSON-Form in die
//! aktuelle hebt — NIE ein `DELETE FROM`-Ritual (Global Constraint des
//! M1-Plans).
//!
//! # Resume-Reihenfolge (Lehre PR #86)
//!
//! [`apply`] läuft VOR den Seed-Guards: es füllt `Markets` und
//! `CitizenRegistry`, wodurch `seed_economy` und `seed_citizens` in
//! `install_world_resources` zu No-ops werden. Einstieg:
//! [`crate::systems::install_world_systems_with_snapshot`].

use std::collections::BTreeMap;

use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::citizens::trips::{ActiveTrip, ActiveTrips, walk_ticks};
use crate::citizens::{Citizen, CitizenRegistry, CitizenState, TripKind};
use crate::clock::WorldClock;
use crate::econ::{
    AccountBook, Ask, Bid, DemandPool, DemandPools, EconomicActorId, EconomyEvent, GoodId,
    HouseholdSector, InputPool, InputPools, InventoryBalance, InventoryBook, MarketDistances,
    MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite, Markets, MoneyAccount,
    NextOrderId, OrderBook, OrderId, PERSISTED_LEDGER_TAIL, ProductionPool, ProductionPools,
    RawDeposit, RawDeposits, SupplyPool, SupplyPools, TradeLedger, euclid_m,
    init_ledger_audit_cursor,
};
use crate::model::{BuildingLifecycle, BuildingStates};
use crate::systems::SharedSimWorld;

/// Aktuelle Snapshot-Schema-Version. Jede Änderung am Snapshot-Schema nach
/// dem ersten Deploy erhöht sie UND fügt in [`migrate_snapshot`] einen
/// Migrations-Arm hinzu (No-Wipe-Prinzip).
pub const WORLD_SNAPSHOT_VERSION: u32 = 1;

/// Ein persistierter Fussweg-Rest — die EINZIGE Trip-Form im Snapshot.
///
/// Fahrzeug-Policy beim Resume (bewusster M1-Schnitt, Plan Task 10): die
/// traffic-core-Fleet ist NICHT persistiert, also wird ein laufender
/// [`ActiveTrip::Driving`] beim [`extract`] in einen Fussweg-Rest übersetzt —
/// `VehId` wird verworfen, Dauer = Rest-Distanz / 1.4 m/s ([`walk_ticks`]).
/// Kein Bürger geht verloren, die SFC-Konservierung ist unberührt (Trips
/// bewegen kein Geld).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedWalk {
    pub depart_tick: u64,
    pub arrive_tick: u64,
    pub from_building: u32,
    pub dest_building: u32,
}

/// Ein Bürger im Snapshot: Identität, Wohn-/Arbeitsgebäude, Zustand und ein
/// allfälliger laufender Trip (immer als Fussweg-Rest, siehe
/// [`PersistedWalk`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitizenSnap {
    pub id: u32,
    pub home: u32,
    pub work: u32,
    pub state: CitizenState,
    pub active_trip: Option<PersistedWalk>,
}

/// Serialisierbare Form von [`HouseholdSector`] (`Vec<(K, V)>`-Muster wie
/// alle Maps in diesem Modul).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseholdSectorSnap {
    pub population: u64,
    pub pool_weights: Vec<(EconomicActorId, i64)>,
}

/// Alle persistierten Econ-Resources, analog dem geernteten
/// `EconomyPersistSnapshot` (bbd0159) — ohne `market_chunks` (kein LOD in
/// M1) und ohne `market_distances` (in [`apply`] aus `markets` neu
/// gerechnet).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EconSnap {
    pub accounts: Vec<(EconomicActorId, MoneyAccount)>,
    pub inventory: Vec<((EconomicActorId, GoodId), InventoryBalance)>,
    pub bids: Vec<(OrderId, Bid)>,
    pub asks: Vec<(OrderId, Ask)>,
    pub next_order_id: u64,
    pub markets: Vec<(MarketId, MarketSite)>,
    pub market_goods: Vec<(MarketGoodKey, MarketGoodState)>,
    pub demand_pools: Vec<(EconomicActorId, DemandPool)>,
    pub supply_pools: Vec<(EconomicActorId, SupplyPool)>,
    pub production_pools: Vec<(EconomicActorId, ProductionPool)>,
    /// Leontief-Input-Pools der kaufenden Produzenten — persistiert wegen des
    /// `last_generated_tick`-Cursors (frozen-time). `ProducerPolicies` ist
    /// NICHT hier: authored Config, bei jedem Boot neu angewandt.
    pub input_pools: Vec<(EconomicActorId, InputPool)>,
    /// Die Rohstoff-Faucets — persistiert wegen des `last_regen_tick`-Cursors.
    pub raw_deposits: Vec<(EconomicActorId, RawDeposit)>,
    /// Die letzten [`PERSISTED_LEDGER_TAIL`] Ledger-Events (alt → neu).
    pub ledger_tail: Vec<EconomyEvent>,
    pub household_sector: HouseholdSectorSnap,
}

/// Das versionierte Abbild der ganzen Welt-Sim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldCoreSnapshot {
    pub version: u32,
    pub clock: WorldClock,
    pub citizens: Vec<CitizenSnap>,
    /// Nur ABWEICHUNGEN von [`BuildingLifecycle::Occupied`] (M1: praktisch
    /// leer, das Datenmodell steht).
    pub building_states: Vec<(u32, BuildingLifecycle)>,
    pub econ: EconSnap,
}

#[derive(Debug, Error)]
pub enum MigrateError {
    #[error("snapshot has no numeric \"version\" field")]
    MissingVersion,
    #[error(
        "unknown snapshot version {0} — add a migration arm in migrate_snapshot (no-wipe principle)"
    )]
    UnknownVersion(u64),
    #[error("snapshot payload does not decode: {0}")]
    Decode(#[from] serde_json::Error),
}

/// Hebt einen rohen (JSON-)Snapshot auf die aktuelle Schema-Version.
///
/// Migrationskette (No-Wipe-Prinzip): `version == 1` deserialisiert direkt;
/// jede künftige Schema-Änderung fügt hier einen Arm hinzu, der die alte
/// Form in die neue transformiert (v1→v2→…), statt Welten zu wipen.
/// Unbekannte Versionen sind ein harter Fehler — nie raten.
pub fn migrate_snapshot(raw: serde_json::Value) -> Result<WorldCoreSnapshot, MigrateError> {
    let version = raw
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(MigrateError::MissingVersion)?;
    match version {
        1 => Ok(serde_json::from_value(raw)?),
        // Künftig: 2 => migrate_v1_to_v2(raw), dann rekursiv weiter.
        other => Err(MigrateError::UnknownVersion(other)),
    }
}

/// Zieht den Snapshot aus einer laufenden Welt. `BTreeMap`-Iteration ist
/// sortiert ⇒ die `Vec`s (und ihr JSON) sind byte-stabil.
pub fn extract(world: &World) -> WorldCoreSnapshot {
    let clock = *world.resource::<WorldClock>();
    let sim = world.resource::<SharedSimWorld>();
    let trips = world.resource::<ActiveTrips>();
    let registry = world.resource::<CitizenRegistry>();

    let mut citizens = Vec::with_capacity(registry.by_id.len());
    for (&id, &entity) in &registry.by_id {
        let entity_ref = world.entity(entity);
        let citizen = *entity_ref
            .get::<Citizen>()
            .expect("registry entity without Citizen component");
        let state = *entity_ref
            .get::<CitizenState>()
            .expect("registry entity without CitizenState component");
        let active_trip = trips
            .0
            .get(&id)
            .map(|trip| persist_trip(*trip, &citizen, state, &clock, &sim.0));
        citizens.push(CitizenSnap {
            id,
            home: citizen.home,
            work: citizen.work,
            state,
            active_trip,
        });
    }

    let building_states = world
        .resource::<BuildingStates>()
        .0
        .iter()
        .map(|(&b, &lifecycle)| (b, lifecycle))
        .collect();

    WorldCoreSnapshot {
        version: WORLD_SNAPSHOT_VERSION,
        clock,
        citizens,
        building_states,
        econ: extract_econ(world),
    }
}

/// Übersetzt einen laufenden Trip in die persistierte Fussweg-Form.
///
/// [`ActiveTrip::Driving`]: die Fahrzeug-Position ist nicht Teil des
/// Snapshots (Fleet nicht persistiert), also wird konservativ die VOLLE
/// Luftlinie des Fahr-Legs neu als Fussweg angesetzt — Origin aus dem im
/// `Commuting`-Zustand getragenen [`TripKind`] (Fahr-Trips existieren nur
/// home↔work: `ToMarket`/back-to-work sind gebäudeseitig work→work mit
/// Distanz 0 und damit immer Fusswege).
fn persist_trip(
    trip: ActiveTrip,
    citizen: &Citizen,
    state: CitizenState,
    clock: &WorldClock,
    sim: &crate::model::SimWorld,
) -> PersistedWalk {
    match trip {
        ActiveTrip::WalkingUntil {
            depart_tick,
            arrive_tick,
            from_building,
            dest_building,
        } => PersistedWalk {
            depart_tick,
            arrive_tick,
            from_building,
            dest_building,
        },
        ActiveTrip::Driving { dest_building, .. } => {
            let CitizenState::Commuting { trip: kind } = state else {
                unreachable!(
                    "citizen {} has an active Driving trip while not Commuting ({state:?}) — trips/rhythm desync bug",
                    citizen.id
                );
            };
            let origin = match kind {
                TripKind::ToWork => citizen.home,
                TripKind::ToHome | TripKind::ToMarket => citizen.work,
            };
            let from = &sim.buildings[origin as usize];
            let to = &sim.buildings[dest_building as usize];
            let dist_m = euclid_m((from.x, from.z), (to.x, to.z));
            PersistedWalk {
                depart_tick: clock.world_tick,
                arrive_tick: clock.world_tick + walk_ticks(dist_m),
                from_building: origin,
                dest_building,
            }
        }
    }
}

fn extract_econ(world: &World) -> EconSnap {
    let accounts = world.resource::<AccountBook>();
    let inventory = world.resource::<InventoryBook>();
    let orders = world.resource::<OrderBook>();
    let next = world.resource::<NextOrderId>();
    let markets = world.resource::<Markets>();
    let market_goods = world.resource::<MarketGoods>();
    let demand = world.resource::<DemandPools>();
    let supply = world.resource::<SupplyPools>();
    let production = world.resource::<ProductionPools>();
    let input_pools = world.resource::<InputPools>();
    let raw_deposits = world.resource::<RawDeposits>();
    let ledger = world.resource::<TradeLedger>();
    let household = world.resource::<HouseholdSector>();

    let ledger_tail = {
        let events = &ledger.0;
        let start = events.len().saturating_sub(PERSISTED_LEDGER_TAIL);
        events[start..].to_vec()
    };

    EconSnap {
        accounts: accounts.accounts.iter().map(|(k, v)| (*k, *v)).collect(),
        inventory: inventory.balances.iter().map(|(k, v)| (*k, *v)).collect(),
        bids: orders.bids.iter().map(|(k, v)| (*k, v.clone())).collect(),
        asks: orders.asks.iter().map(|(k, v)| (*k, v.clone())).collect(),
        next_order_id: next.0,
        markets: markets.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        market_goods: market_goods
            .0
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        demand_pools: demand.0.iter().map(|(k, v)| (*k, *v)).collect(),
        supply_pools: supply.0.iter().map(|(k, v)| (*k, *v)).collect(),
        production_pools: production.0.iter().map(|(k, v)| (*k, v.clone())).collect(),
        input_pools: input_pools.0.iter().map(|(k, v)| (*k, *v)).collect(),
        raw_deposits: raw_deposits.0.iter().map(|(k, v)| (*k, *v)).collect(),
        ledger_tail,
        household_sector: HouseholdSectorSnap {
            population: household.population,
            pool_weights: household
                .pool_weights
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect(),
        },
    }
}

/// Rekonstruiert die Welt aus einem Snapshot: Uhr, Econ-Resources (inkl. neu
/// gerechneter [`MarketDistances`]), Gebäude-Zustände, Bürger-Entities samt
/// [`CitizenRegistry`] und laufende Fusswege.
///
/// MUSS vor den Seed-Guards laufen (Lehre PR #86): die gefüllten `Markets` /
/// `CitizenRegistry` machen `seed_economy` / `seed_citizens` zu No-ops.
/// Einstieg dafür: [`crate::systems::install_world_systems_with_snapshot`].
pub fn apply(world: &mut World, snap: WorldCoreSnapshot) {
    // Uhr zuerst: Zeit existiert, bevor irgendetwas sie liest.
    world.insert_resource(snap.clock);

    // ── Wirtschaft ───────────────────────────────────────────────────────
    let econ = snap.econ;
    world.insert_resource(AccountBook {
        accounts: econ.accounts.into_iter().collect(),
    });
    world.insert_resource(InventoryBook {
        balances: econ.inventory.into_iter().collect(),
    });
    world.insert_resource(OrderBook {
        bids: econ.bids.into_iter().collect(),
        asks: econ.asks.into_iter().collect(),
    });
    world.insert_resource(NextOrderId(econ.next_order_id));

    // MarketDistances NICHT persistiert: pure Funktion der Markt-Positionen,
    // hier mit exakt der Seed-Formel (euclid_m, beide Richtungen) neu gebaut.
    let markets: BTreeMap<MarketId, MarketSite> = econ.markets.into_iter().collect();
    let mut distances = BTreeMap::new();
    for a in markets.values() {
        for b in markets.values() {
            if a.id < b.id {
                let d = euclid_m((a.x, a.z), (b.x, b.z));
                distances.insert((a.id, b.id), d);
                distances.insert((b.id, a.id), d);
            }
        }
    }
    world.insert_resource(Markets(markets));
    world.insert_resource(MarketDistances(distances));

    world.insert_resource(MarketGoods(econ.market_goods.into_iter().collect()));
    world.insert_resource(DemandPools(econ.demand_pools.into_iter().collect()));
    world.insert_resource(SupplyPools(econ.supply_pools.into_iter().collect()));
    world.insert_resource(ProductionPools(econ.production_pools.into_iter().collect()));
    world.insert_resource(InputPools(econ.input_pools.into_iter().collect()));
    world.insert_resource(RawDeposits(econ.raw_deposits.into_iter().collect()));
    world.insert_resource(TradeLedger(econ.ledger_tail));
    // Der restaurierte Tail war vor dem Shutdown bereits durabel geflusht —
    // Cursor ans Ende, damit er nicht doppelt appended wird.
    init_ledger_audit_cursor(world);
    world.insert_resource(HouseholdSector {
        population: econ.household_sector.population,
        pool_weights: econ.household_sector.pool_weights.into_iter().collect(),
    });

    // ── Gebäude-Lebenszyklen (nur Abweichungen von Occupied) ─────────────
    world.insert_resource(BuildingStates(snap.building_states.into_iter().collect()));

    // ── Bürger: Entities spawnen, Registry + laufende Fusswege aufbauen ──
    let mut by_id = BTreeMap::new();
    let mut trips = BTreeMap::new();
    for c in snap.citizens {
        let entity = world
            .spawn((
                Citizen {
                    id: c.id,
                    home: c.home,
                    work: c.work,
                },
                c.state,
            ))
            .id();
        by_id.insert(c.id, entity);
        if let Some(walk) = c.active_trip {
            trips.insert(
                c.id,
                ActiveTrip::WalkingUntil {
                    depart_tick: walk.depart_tick,
                    arrive_tick: walk.arrive_tick,
                    from_building: walk.from_building,
                    dest_building: walk.dest_building,
                },
            );
        }
    }
    world.insert_resource(CitizenRegistry {
        count: by_id.len() as u64,
        by_id,
    });
    world.insert_resource(ActiveTrips(trips));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::world::World;

    use super::*;
    use crate::citizens::SeedParams;
    use crate::econ::seed::EconomySeed;
    use crate::model::SimWorld;
    use crate::model::test_fixture::FIXTURE;
    use crate::systems::{
        WorldCorePlugin, install_world_systems, install_world_systems_with_snapshot,
    };

    const ECONOMY_JSON: &str = include_str!("../../../../data/winterthur/economy.json");

    fn plugin() -> WorldCorePlugin {
        WorldCorePlugin {
            seed: EconomySeed::from_json(ECONOMY_JSON).expect("economy.json must parse"),
            sim_world: Arc::new(SimWorld::load(FIXTURE).expect("fixture must load")),
            seed_params: SeedParams {
                center: (0.0, 0.0),
                radius_m: 10_000.0,
                residents_per_40m2: 1.0,
                seed: 42,
            },
        }
    }

    fn fresh_world() -> (World, Schedule) {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        install_world_systems(&mut world, &mut schedule, &plugin());
        (world, schedule)
    }

    #[test]
    fn roundtrip_conserves_money_citizens_and_clock() {
        let (mut a, mut sched_a) = fresh_world();
        for _ in 0..500 {
            sched_a.run(&mut a);
        }
        let money = a.resource::<AccountBook>().total_money().expect("sum");
        let count = a.resource::<CitizenRegistry>().count;
        assert!(count > 0, "fixture must seed citizens");
        let snap = extract(&a);
        assert_eq!(snap.version, WORLD_SNAPSHOT_VERSION);
        assert_eq!(snap.clock.world_tick, 500);
        assert_eq!(snap.citizens.len() as u64, count);

        // Frische Welt: apply läuft VOR den Seed-Guards ⇒ beide Seeds no-op.
        let mut b = World::new();
        let mut sched_b = Schedule::default();
        install_world_systems_with_snapshot(&mut b, &mut sched_b, &plugin(), Some(snap));
        assert_eq!(
            b.resource::<WorldClock>().world_tick,
            500,
            "clock resumes, no re-seed reset"
        );
        assert_eq!(b.resource::<CitizenRegistry>().count, count);
        assert_eq!(
            b.resource::<AccountBook>().total_money().expect("sum"),
            money
        );
        assert_eq!(
            b.resource::<MarketDistances>(),
            a.resource::<MarketDistances>(),
            "recomputed distances match the seeded bake"
        );
        assert_eq!(
            b.resource::<HouseholdSector>().population,
            count,
            "household head-count restored, not re-derived from a fresh seed"
        );

        // 500 Ticks weiter: Uhr läuft fort, Geld bleibt byte-konserviert
        // (der fail-fast SFC-Audit im Schedule würde jede Drift panicen).
        for _ in 0..500 {
            sched_b.run(&mut b);
        }
        assert_eq!(b.resource::<WorldClock>().world_tick, 1_000);
        assert!(b.resource::<WorldClock>().world_tick > 500);
        assert_eq!(
            b.resource::<AccountBook>().total_money().expect("sum"),
            money,
            "total_money conserved across extract/apply + 500 more ticks"
        );
        assert_eq!(b.resource::<CitizenRegistry>().count, count);
    }

    #[test]
    fn migrate_rejects_unknown_version() {
        let err = migrate_snapshot(serde_json::json!({"version": 99})).unwrap_err();
        assert!(matches!(err, MigrateError::UnknownVersion(99)), "{err:?}");

        let err = migrate_snapshot(serde_json::json!({"no_version": true})).unwrap_err();
        assert!(matches!(err, MigrateError::MissingVersion), "{err:?}");
    }

    #[test]
    fn migrate_roundtrips_current_version_via_json_value() {
        let (mut world, mut schedule) = fresh_world();
        for _ in 0..50 {
            schedule.run(&mut world);
        }
        // Eine Lebenszyklus-Abweichung, damit building_states nicht leer ist.
        world
            .resource_mut::<BuildingStates>()
            .0
            .insert(2, BuildingLifecycle::Vacant);
        let snap = extract(&world);
        assert_eq!(snap.building_states, vec![(2, BuildingLifecycle::Vacant)]);

        let raw = serde_json::to_value(&snap).expect("snapshot serializes");
        let restored = migrate_snapshot(raw).expect("version 1 migrates");
        assert_eq!(restored, snap);
    }

    #[test]
    fn driving_trip_is_persisted_as_walk_remainder() {
        let (mut world, _schedule) = fresh_world();
        world.resource_mut::<WorldClock>().world_tick = 1_200;

        // Bürger 0: fährt Richtung Arbeit (dest {A2}=0, home {B1}=1, 100 m).
        // Bürger 1: geht bereits zu Fuss heim.
        let e0 = world.resource::<CitizenRegistry>().by_id[&0];
        let e1 = world.resource::<CitizenRegistry>().by_id[&1];
        *world.get_mut::<CitizenState>(e0).unwrap() = CitizenState::Commuting {
            trip: TripKind::ToWork,
        };
        *world.get_mut::<CitizenState>(e1).unwrap() = CitizenState::Commuting {
            trip: TripKind::ToHome,
        };
        {
            let mut trips = world.resource_mut::<ActiveTrips>();
            trips.0.insert(
                0,
                ActiveTrip::Driving {
                    veh: 7,
                    dest_building: 0,
                },
            );
            trips.0.insert(
                1,
                ActiveTrip::WalkingUntil {
                    depart_tick: 1_100,
                    arrive_tick: 1_300,
                    from_building: 0,
                    dest_building: 1,
                },
            );
        }

        let snap = extract(&world);
        let by_id: BTreeMap<u32, &CitizenSnap> = snap.citizens.iter().map(|c| (c.id, c)).collect();
        // Driving → Walk-Rest: volle Luftlinie home→work = 100 m ⇒ 715 Ticks.
        assert_eq!(walk_ticks(100), 715, "fixture leg sanity");
        assert_eq!(
            by_id[&0].active_trip,
            Some(PersistedWalk {
                depart_tick: 1_200,
                arrive_tick: 1_200 + 715,
                from_building: 1,
                dest_building: 0
            }),
            "VehId dropped, remainder resumed on foot"
        );
        // Walking bleibt unverändert.
        assert_eq!(
            by_id[&1].active_trip,
            Some(PersistedWalk {
                depart_tick: 1_100,
                arrive_tick: 1_300,
                from_building: 0,
                dest_building: 1
            })
        );
        assert_eq!(by_id[&2].active_trip, None);

        // apply macht daraus wieder WalkingUntil-Einträge (nie Driving).
        let mut b = World::new();
        apply(&mut b, snap);
        let trips = b.resource::<ActiveTrips>();
        assert_eq!(
            trips.0[&0],
            ActiveTrip::WalkingUntil {
                depart_tick: 1_200,
                arrive_tick: 1_915,
                from_building: 1,
                dest_building: 0
            }
        );
        assert_eq!(
            trips.0[&1],
            ActiveTrip::WalkingUntil {
                depart_tick: 1_100,
                arrive_tick: 1_300,
                from_building: 0,
                dest_building: 1
            }
        );
        assert_eq!(trips.0.len(), 2);
        assert_eq!(b.resource::<CitizenRegistry>().count, 15);
    }
}
