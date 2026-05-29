pub mod base_world;
pub mod chunk;
pub mod city_network;
pub mod ecs_runtime;
pub mod events;
pub mod ids;
pub mod mobility;
pub mod mobility_geometry;
pub mod persistence;
pub mod routing;
pub mod scheduler;
pub mod tile;
pub mod time;
pub mod world;

// Re-export bevy_ecs so downstream crates (sim-server, sim-runner) can refer
// to the ECS World/Entity types via `sim_core::bevy_ecs::*` instead of having
// to depend on bevy_ecs directly. This keeps the dependency graph centered on
// sim-core as the ECS host.
pub use bevy_ecs;
