//! world-core — Bürger + Wirtschaft der persistenten Winterthur-Welt.
//!
//! Weltmodell: Entitäten als Wahrheit (Gebäude/Bürger/Firmen in lokalen
//! Metern), keine Tile-Raster. Spec:
//! docs/superpowers/specs/2026-07-05-mmorpg-m1-persistent-world-design.md

pub mod citizens;
pub mod clock;
pub mod econ;
pub mod model;
pub mod persist;
pub mod systems;

pub use citizens::rhythm::{TripRequest, TripRequests, rhythm_system};
pub use citizens::trips::{
    ActiveTrip, ActiveTrips, CarRoute, CitizenCarCounters, CoreAccess, TripRouter, TripRouterRes,
    arrivals_system, dispatch_trips_system,
};
pub use citizens::{Citizen, CitizenRegistry, CitizenState, SeedParams, TripKind, seed_citizens};
pub use clock::{TICKS_PER_SECOND, WORLD_TIME_SCALE, WorldClock};
pub use model::{BuildingLifecycle, BuildingStates, SimBuilding, SimWorld, Usage, WorldError};
pub use persist::{
    CitizenSnap, EconSnap, MigrateError, PersistedWalk, WORLD_SNAPSHOT_VERSION, WorldCoreSnapshot,
    migrate_snapshot,
};
pub use systems::{
    AuditStatus, ECONOMY_CADENCE_TICKS, SharedSimWorld, WorldCorePlugin,
    advance_world_clock_system, econ_systems, install_world_resources,
    install_world_resources_with_snapshot, install_world_systems,
    install_world_systems_with_snapshot,
};
