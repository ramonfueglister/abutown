//! Vehicle-conservation accounting (spec §8).
//!
//! Every vehicle the shell ever places into the kernel must be accounted for
//! at every tick:
//!
//! ```text
//! spawned == arrived + alive
//! ```
//!
//! where *arrived* counts every end-of-route despawn — **including gateway
//! sinks**: a route ending on a Gemeinde-boundary stub's in-lane despawns via
//! the kernel's normal end-of-route path, so boundary out-flow is arrivals,
//! not leakage. There is no other way for a vehicle to leave the kernel, so
//! any drift means a spawn or despawn went unobserved (an accounting bug, or
//! a kernel change that removes vehicles outside the despawn list).
//!
//! [`Conservation`] is a shell resource: `spawn_trips` adds this tick's
//! successful placements, `core_tick` adds this tick's despawn count (from
//! [`traffic_core::Core::despawned_last_tick`]) and `debug_assert!`s the
//! invariant — free in release, fatal-with-context under test.

use bevy_ecs::prelude::Resource;

/// Monotonic vehicle-conservation counters over a sim's lifetime.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Conservation {
    /// Vehicles placed into the kernel by the spawn path (live windows +
    /// warm start). Mirrors `SpawnCounters::spawned` by construction.
    pub spawned: u64,
    /// Vehicles removed via the kernel's end-of-route despawn — internal
    /// destinations and gateway sinks alike.
    pub arrived: u64,
    /// Trips dropped because the router found no origin→destination path
    /// (informational; such trips never enter the kernel, so they are NOT
    /// part of the invariant).
    pub skipped_no_route: u64,
}

impl Conservation {
    /// The conservation invariant against the kernel's current population:
    /// `spawned == arrived + alive`.
    #[must_use]
    pub fn holds(&self, alive: usize) -> bool {
        self.spawned == self.arrived + alive as u64
    }
}
