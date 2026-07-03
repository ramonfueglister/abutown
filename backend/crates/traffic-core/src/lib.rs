//! `traffic-core`: the pure microscopic traffic simulation kernel.
//!
//! No I/O, no serde, no engine bindings — just the deterministic two-phase
//! tick over a structure-of-arrays fleet on the baked lane network from
//! [`traffic_net`]. Car-following is the Intelligent Driver Model
//! ([`idm`]); randomness is the stateless splitmix64 finalizer [`u01`].
//!
//! Design constraints (project-binding):
//!  * **Determinism:** no `HashMap` iteration in the sim path; randomness only
//!    via [`u01`]`(seed, tick, id)`; phase-2 apply is sequential in fixed slot
//!    order. [`Core::state_hash`] is identical regardless of rayon thread
//!    count.
//!  * **No hot-path allocation:** all per-tick buffers are pre-sized in
//!    [`Core::new`] and reused.
//!  * **Task 4/5 ready:** [`fleet::LaneIndex`] (neighbour queries) and the
//!    cross-boundary leader lookup are the seams MOBIL lane changes and
//!    intersection gating will plug into.

pub mod fleet;
pub mod idm;
pub mod junction;
pub mod mobil;
pub mod rng;
pub mod tick;

pub use fleet::{Fleet, LaneIndex, RouteHandle, VehId};
pub use idm::{IdmParams, idm_accel};
pub use mobil::{Follower, LaneNeighbourhood, MobilDecision, MobilParams};
pub use rng::u01;
pub use tick::{Core, DT, VehicleView};
