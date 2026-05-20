//! Persistable snapshot of the mobility ECS world.
//!
//! After Phase 8a Task 9 dissolved the `MobilityWorld` wrapper, persistence
//! goes through a dedicated serializable struct. `MobilityPersistSnapshot`
//! holds exactly the fields the previous `MobilityWorld` serde impl emitted
//! — so the JSON wire format is byte-identical to the legacy one.
//!
//! Use `extract_from_world` to pull a snapshot out of a live `World`, and
//! `apply_into_world` to hydrate a freshly-installed mobility World from a
//! snapshot read back from storage.
//!
//! The schema mirrors the legacy `MobilityWorld` serde shape:
//!
//! ```text
//! { tick, agents, vehicles, stops, routes, link_polylines,
//!   flow_cells, chunk_activities }
//! ```

use std::collections::HashMap;

use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, ChunkCoord, LinkId, RouteId, StopId, VehicleId};
use crate::mobility::lod::{FlowCell, MobilityActivity};
use crate::mobility::records::{AgentRecord, RouteRecord, StopRecord, VehicleRecord};
use crate::mobility::resources::{
    ChunkActivities, FlowCells, LinkPolylines, Routes, Stops, Tick,
};

/// Serializable snapshot of mobility-world state. The JSON shape matches the
/// legacy `MobilityWorld` serde representation exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilityPersistSnapshot {
    pub tick: u64,
    pub agents: HashMap<AgentId, AgentRecord>,
    pub vehicles: HashMap<VehicleId, VehicleRecord>,
    pub stops: HashMap<StopId, StopRecord>,
    pub routes: HashMap<RouteId, RouteRecord>,
    pub link_polylines: HashMap<LinkId, Vec<(f32, f32)>>,
    pub flow_cells: HashMap<ChunkCoord, FlowCell>,
    pub chunk_activities: HashMap<ChunkCoord, MobilityActivity>,
}

impl Serialize for MobilityPersistSnapshot {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct WorldRepr<'a> {
            tick: u64,
            agents: &'a HashMap<AgentId, AgentRecord>,
            vehicles: &'a HashMap<VehicleId, VehicleRecord>,
            stops: &'a HashMap<StopId, StopRecord>,
            routes: &'a HashMap<RouteId, RouteRecord>,
            link_polylines: &'a HashMap<LinkId, Vec<(f32, f32)>>,
            flow_cells: Vec<(ChunkCoord, &'a FlowCell)>,
            chunk_activities: Vec<(ChunkCoord, MobilityActivity)>,
        }
        // Sort chunk-keyed entries — JSON output must round-trip byte-stably.
        let mut flow_cells: Vec<(ChunkCoord, &FlowCell)> =
            self.flow_cells.iter().map(|(k, v)| (*k, v)).collect();
        flow_cells.sort_unstable_by_key(|(c, _)| *c);
        let mut chunk_activities: Vec<(ChunkCoord, MobilityActivity)> = self
            .chunk_activities
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        chunk_activities.sort_unstable_by_key(|(c, _)| *c);

        WorldRepr {
            tick: self.tick,
            agents: &self.agents,
            vehicles: &self.vehicles,
            stops: &self.stops,
            routes: &self.routes,
            link_polylines: &self.link_polylines,
            flow_cells,
            chunk_activities,
        }
        .serialize(ser)
    }
}

impl<'de> Deserialize<'de> for MobilityPersistSnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WorldRepr {
            tick: u64,
            agents: HashMap<AgentId, AgentRecord>,
            vehicles: HashMap<VehicleId, VehicleRecord>,
            stops: HashMap<StopId, StopRecord>,
            routes: HashMap<RouteId, RouteRecord>,
            link_polylines: HashMap<LinkId, Vec<(f32, f32)>>,
            #[serde(default)]
            flow_cells: Vec<(ChunkCoord, FlowCell)>,
            #[serde(default)]
            chunk_activities: Vec<(ChunkCoord, MobilityActivity)>,
        }
        let repr = WorldRepr::deserialize(de)?;
        Ok(Self {
            tick: repr.tick,
            agents: repr.agents,
            vehicles: repr.vehicles,
            stops: repr.stops,
            routes: repr.routes,
            link_polylines: repr.link_polylines,
            flow_cells: repr.flow_cells.into_iter().collect(),
            chunk_activities: repr.chunk_activities.into_iter().collect(),
        })
    }
}

/// Pull a persist snapshot out of a live mobility world. The world must have
/// already had `install_mobility` called on it.
pub fn extract_from_world(world: &World) -> MobilityPersistSnapshot {
    let agents_map: HashMap<AgentId, AgentRecord> = crate::mobility::api::agents(world)
        .into_iter()
        .map(|rec| (rec.id.clone(), rec))
        .collect();
    let vehicles_map: HashMap<VehicleId, VehicleRecord> = crate::mobility::api::vehicles(world)
        .into_iter()
        .map(|rec| (rec.id.clone(), rec))
        .collect();
    MobilityPersistSnapshot {
        tick: world.resource::<Tick>().0,
        agents: agents_map,
        vehicles: vehicles_map,
        stops: world.resource::<Stops>().0.clone(),
        routes: world.resource::<Routes>().0.clone(),
        link_polylines: world.resource::<LinkPolylines>().0.clone(),
        flow_cells: world.resource::<FlowCells>().0.clone(),
        chunk_activities: world.resource::<ChunkActivities>().0.clone(),
    }
}

/// Hydrate a freshly-installed mobility World from a persist snapshot.
///
/// Registers polylines + stops + routes BEFORE spawning agents/vehicles so
/// the spawn helpers can resolve a real Position from the start (post-Phase
/// 7b — see commit 49f2f25 "compute Position at spawn so LOD classifies into
/// real chunk").
pub fn apply_into_world(world: &mut World, snap: MobilityPersistSnapshot) {
    world.resource_mut::<Tick>().0 = snap.tick;
    {
        let mut links_res = world.resource_mut::<LinkPolylines>();
        for (id, points) in snap.link_polylines {
            links_res.0.insert(id, points);
        }
    }
    {
        let mut routes_res = world.resource_mut::<Routes>();
        for (id, route) in snap.routes {
            routes_res.0.insert(id, route);
        }
    }
    {
        let mut stops_res = world.resource_mut::<Stops>();
        for (id, stop) in snap.stops {
            stops_res.0.insert(id, stop);
        }
    }
    for (_, agent) in snap.agents {
        crate::mobility::api::spawn_agent_from_record(world, agent);
    }
    for (_, vehicle) in snap.vehicles {
        crate::mobility::api::spawn_vehicle_from_record(world, vehicle);
    }
    world.resource_mut::<FlowCells>().0 = snap.flow_cells;
    world.resource_mut::<ChunkActivities>().0 = snap.chunk_activities;
}
