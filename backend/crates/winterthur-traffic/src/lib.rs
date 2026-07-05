//! `winterthur-traffic`: contraction-hierarchy (CH) route computation over
//! the baked Winterthur traffic network, with live weight updates.
//!
//! CH runs on the **edge graph** (see [`router`] module docs): CH node id =
//! `traffic-net` edge id. This crate depends only on `traffic-net` for the
//! network representation and `fast_paths` for the CH engine; `traffic-core`
//! is a dev-dependency used only by integration tests that spawn a computed
//! route into the sim kernel.

pub mod audit;
pub mod cells;
pub mod clock;
pub mod demand;
pub mod flow;
pub mod gateway;
pub mod measure;
pub mod router;
pub mod shell;
pub mod spawner;

pub use router::Router;
