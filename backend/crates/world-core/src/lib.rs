//! world-core — Bürger + Wirtschaft der persistenten Winterthur-Welt.
//!
//! Weltmodell: Entitäten als Wahrheit (Gebäude/Bürger/Firmen in lokalen
//! Metern), keine Tile-Raster. Spec:
//! docs/superpowers/specs/2026-07-05-mmorpg-m1-persistent-world-design.md

pub mod model;

pub use model::{BuildingLifecycle, SimBuilding, SimWorld, Usage, WorldError};
