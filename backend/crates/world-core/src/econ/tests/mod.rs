//! Harvested economy unit tests (bbd0159). Files dropped relative to sim-core:
//! abutopia_*, lod, materialize, persist, plugin, seed, systems, transport,
//! audit, capita*, conservation, flow_shipments (chunk-/plugin-/persistence-
//! coupled — their replacements arrive with Tasks 6/7/10). macro_flow and
//! wages are harvested MINUS their schedule-level Section-C/full-tick tests
//! (EconomyPlugin/MobilityPlugin harness), which Task 6 rebuilds.

mod auction;
mod determinism;
mod drain;
mod expiry;
mod locking;
mod macro_flow;
mod overflow;
mod pools;
mod pricing;
mod producers;
mod rationing;
mod settlement_policy;
mod wages;
