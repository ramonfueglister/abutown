use std::collections::HashMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};
use crate::mobility::components::*;
use crate::mobility::resources::*;

mod records;
pub use records::*;

mod dto;
pub use dto::*;

pub mod components;
pub mod lod;
pub mod resources;
pub mod seed;
pub mod systems;

pub fn chunk_of(x: f32, y: f32, chunk_size: u16) -> crate::ids::ChunkCoord {
    let cs = chunk_size as f32;
    crate::ids::ChunkCoord {
        x: x.div_euclid(cs) as i32,
        y: y.div_euclid(cs) as i32,
    }
}

fn stable_index(id: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish() as u32
}

fn compute_agent_sprite_key(id: &AgentId) -> String {
    format!("pedestrian:{}", stable_index(&id.0) % 16)
}

fn compute_vehicle_sprite_key(id: &VehicleId) -> String {
    format!("tram:{}", stable_index(&id.0) % 4)
}

pub struct MobilityWorld {
    pub(crate) world: World,
    pub(crate) schedule: Schedule,
    pub(crate) by_agent_id: HashMap<AgentId, Entity>,
    pub(crate) by_vehicle_id: HashMap<VehicleId, Entity>,
}

impl MobilityWorld {
    pub fn empty() -> Self {
        let mut world = World::new();
        world.insert_resource(Tick(0));
        world.insert_resource(Routes::default());
        world.insert_resource(Stops::default());
        world.insert_resource(LinkPolylines::default());
        world.insert_resource(DirtyAgents::default());
        world.insert_resource(DirtyVehicles::default());
        world.insert_resource(ChunkActivities::default());
        world.insert_resource(ChunkActivityCooldowns::default());
        world.insert_resource(FlowCells::default());
        world.insert_resource(ChunkSubscribers::default());
        world.insert_resource(ChunkPopulations::default());
        world.insert_resource(AgentsByChunk::default());
        world.insert_resource(VehiclesByChunk::default());
        world.insert_resource(ChunkTransitions::default());
        world.insert_resource(PreviousAgentChunks::default());
        world.insert_resource(PreviousVehicleChunks::default());

        let mut schedule = Schedule::default();
        crate::mobility::systems::install_systems(&mut schedule);

        Self {
            world,
            schedule,
            by_agent_id: HashMap::new(),
            by_vehicle_id: HashMap::new(),
        }
    }
}

impl Default for MobilityWorld {
    fn default() -> Self {
        Self::empty()
    }
}

impl serde::Serialize for MobilityWorld {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use std::collections::HashMap;

        #[derive(serde::Serialize)]
        struct WorldRepr<'a> {
            tick: u64,
            agents: HashMap<&'a AgentId, AgentRecord>,
            vehicles: HashMap<&'a VehicleId, VehicleRecord>,
            stops: &'a HashMap<StopId, StopRecord>,
            routes: &'a HashMap<RouteId, RouteRecord>,
            link_polylines: &'a HashMap<LinkId, Vec<(f32, f32)>>,
            flow_cells: Vec<(crate::ids::ChunkCoord, &'a crate::mobility::lod::FlowCell)>,
            chunk_activities: Vec<(
                crate::ids::ChunkCoord,
                crate::mobility::lod::MobilityActivity,
            )>,
        }

        let agents_map: HashMap<&AgentId, AgentRecord> = self
            .by_agent_id
            .keys()
            .filter_map(|id| self.agent(id).map(|rec| (id, rec)))
            .collect();
        let vehicles_map: HashMap<&VehicleId, VehicleRecord> = self
            .by_vehicle_id
            .keys()
            .filter_map(|id| self.vehicle(id).map(|rec| (id, rec)))
            .collect();

        // Sort chunk-keyed entries — JSON output must round-trip byte-stably.
        let mut flow_cells: Vec<(crate::ids::ChunkCoord, &crate::mobility::lod::FlowCell)> = self
            .world
            .resource::<FlowCells>()
            .0
            .iter()
            .map(|(k, v)| (*k, v))
            .collect();
        flow_cells.sort_unstable_by_key(|(c, _)| *c);
        let mut chunk_activities: Vec<(
            crate::ids::ChunkCoord,
            crate::mobility::lod::MobilityActivity,
        )> = self
            .world
            .resource::<ChunkActivities>()
            .0
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        chunk_activities.sort_unstable_by_key(|(c, _)| *c);

        WorldRepr {
            tick: self.tick(),
            agents: agents_map,
            vehicles: vehicles_map,
            stops: &self.world.resource::<Stops>().0,
            routes: &self.world.resource::<Routes>().0,
            link_polylines: &self.world.resource::<LinkPolylines>().0,
            flow_cells,
            chunk_activities,
        }
        .serialize(ser)
    }
}

impl<'de> serde::Deserialize<'de> for MobilityWorld {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use std::collections::HashMap;

        #[derive(serde::Deserialize)]
        struct WorldRepr {
            tick: u64,
            agents: HashMap<AgentId, AgentRecord>,
            vehicles: HashMap<VehicleId, VehicleRecord>,
            stops: HashMap<StopId, StopRecord>,
            routes: HashMap<RouteId, RouteRecord>,
            link_polylines: HashMap<LinkId, Vec<(f32, f32)>>,
            #[serde(default)]
            flow_cells: Vec<(crate::ids::ChunkCoord, crate::mobility::lod::FlowCell)>,
            #[serde(default)]
            chunk_activities: Vec<(
                crate::ids::ChunkCoord,
                crate::mobility::lod::MobilityActivity,
            )>,
        }

        let repr = WorldRepr::deserialize(de)?;
        let mut world = MobilityWorld::empty();
        world.world.resource_mut::<Tick>().0 = repr.tick;
        for (_, agent) in repr.agents {
            world.spawn_agent_from_record(agent);
        }
        for (_, vehicle) in repr.vehicles {
            world.spawn_vehicle_from_record(vehicle);
        }
        {
            let mut stops_res = world.world.resource_mut::<Stops>();
            for (id, stop) in repr.stops {
                stops_res.0.insert(id, stop);
            }
        }
        {
            let mut routes_res = world.world.resource_mut::<Routes>();
            for (id, route) in repr.routes {
                routes_res.0.insert(id, route);
            }
        }
        {
            let mut links_res = world.world.resource_mut::<LinkPolylines>();
            for (id, points) in repr.link_polylines {
                links_res.0.insert(id, points);
            }
        }
        world.world.resource_mut::<FlowCells>().0 = repr.flow_cells.into_iter().collect();
        world.world.resource_mut::<ChunkActivities>().0 =
            repr.chunk_activities.into_iter().collect();
        Ok(world)
    }
}

impl MobilityWorld {
    /// Apply a subscription delta for a single connection: increment for each
    /// chunk in `added`, saturating-decrement (and drop on zero) for each
    /// chunk in `removed`. The caller is responsible for de-duplicating
    /// against the connection's existing set if no-op messages matter.
    pub fn apply_subscription_diff<'a, A, R>(&mut self, added: A, removed: R)
    where
        A: IntoIterator<Item = &'a crate::ids::ChunkCoord>,
        R: IntoIterator<Item = &'a crate::ids::ChunkCoord>,
    {
        let mut subs = self.world.resource_mut::<ChunkSubscribers>();
        for coord in added {
            *subs.0.entry(*coord).or_insert(0) += 1;
        }
        for coord in removed {
            if let Some(entry) = subs.0.get_mut(coord) {
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    subs.0.remove(coord);
                }
            }
        }
    }

    /// Read-only accessor: current activity class of a chunk, or `None` if
    /// the chunk has no entry (treated as Asleep).
    pub fn activity_for_chunk(
        &self,
        chunk: crate::ids::ChunkCoord,
    ) -> Option<crate::mobility::lod::MobilityActivity> {
        self.world
            .resource::<ChunkActivities>()
            .0
            .get(&chunk)
            .copied()
    }

    /// Read-only accessor: aggregate flow-cell state for a chunk if present.
    pub fn flow_cell_for_chunk(
        &self,
        chunk: crate::ids::ChunkCoord,
    ) -> Option<&crate::mobility::lod::FlowCell> {
        self.world.resource::<FlowCells>().0.get(&chunk)
    }

    /// Return the number of active WS subscribers for a chunk (0 if none).
    pub fn chunk_subscriber_count(&self, chunk: crate::ids::ChunkCoord) -> u8 {
        self.world
            .resource::<ChunkSubscribers>()
            .0
            .get(&chunk)
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
impl MobilityWorld {
    pub(crate) fn seed_flow_cell(
        &mut self,
        chunk: crate::ids::ChunkCoord,
        cell: crate::mobility::lod::FlowCell,
    ) {
        self.world.resource_mut::<FlowCells>().0.insert(chunk, cell);
    }

    pub(crate) fn seed_chunk_activity(
        &mut self,
        chunk: crate::ids::ChunkCoord,
        activity: crate::mobility::lod::MobilityActivity,
    ) {
        self.world
            .resource_mut::<ChunkActivities>()
            .0
            .insert(chunk, activity);
    }

    pub(crate) fn seed_chunk_subscriber_count(&mut self, chunk: crate::ids::ChunkCoord, count: u8) {
        self.world
            .resource_mut::<ChunkSubscribers>()
            .0
            .insert(chunk, count);
    }
}

impl std::fmt::Debug for MobilityWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MobilityWorld")
            .field("tick", &self.tick())
            .field("agents", &self.by_agent_id.len())
            .field("vehicles", &self.by_vehicle_id.len())
            .finish()
    }
}

impl Clone for MobilityWorld {
    fn clone(&self) -> Self {
        let mut out = MobilityWorld::empty();
        // Tick.
        out.world.insert_resource(Tick(self.tick()));
        // Routes.
        for (id, record) in self.world.resource::<Routes>().0.iter() {
            out.world
                .resource_mut::<Routes>()
                .0
                .insert(id.clone(), record.clone());
        }
        // Stops.
        for (id, record) in self.world.resource::<Stops>().0.iter() {
            out.world
                .resource_mut::<Stops>()
                .0
                .insert(id.clone(), record.clone());
        }
        // Link polylines.
        for (id, points) in self.world.resource::<LinkPolylines>().0.iter() {
            out.world
                .resource_mut::<LinkPolylines>()
                .0
                .insert(id.clone(), points.clone());
        }
        // Re-spawn agents and vehicles from records.
        for record in self.agents() {
            out.spawn_agent_from_record(record);
        }
        for record in self.vehicles() {
            out.spawn_vehicle_from_record(record);
        }
        out
    }
}

impl PartialEq for MobilityWorld {
    fn eq(&self, other: &Self) -> bool {
        self.tick() == other.tick()
            && self.routes() == other.routes()
            && self.world.resource::<Stops>().0 == other.world.resource::<Stops>().0
            && self.world.resource::<LinkPolylines>().0 == other.world.resource::<LinkPolylines>().0
            && self.agents() == other.agents()
            && self.vehicles() == other.vehicles()
    }
}

impl MobilityWorld {
    pub fn tick(&self) -> u64 {
        self.world.resource::<Tick>().0
    }

    pub fn agent(&self, id: &AgentId) -> Option<AgentRecord> {
        let entity = *self.by_agent_id.get(id)?;
        self.agent_record_from_entity(entity)
    }

    pub fn vehicle(&self, id: &VehicleId) -> Option<VehicleRecord> {
        let entity = *self.by_vehicle_id.get(id)?;
        self.vehicle_record_from_entity(entity)
    }

    pub fn stop(&self, id: &StopId) -> Option<StopRecord> {
        self.world.resource::<Stops>().0.get(id).cloned()
    }

    /// Sorted by id for deterministic output.
    pub fn agents(&self) -> Vec<AgentRecord> {
        let mut out: Vec<AgentRecord> = self
            .by_agent_id
            .keys()
            .filter_map(|id| self.agent(id))
            .collect();
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        out
    }

    /// Sorted by id.
    pub fn vehicles(&self) -> Vec<VehicleRecord> {
        let mut out: Vec<VehicleRecord> = self
            .by_vehicle_id
            .keys()
            .filter_map(|id| self.vehicle(id))
            .collect();
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        out
    }

    /// Sorted by id.
    pub fn stops(&self) -> Vec<StopRecord> {
        let mut out: Vec<StopRecord> = self.world.resource::<Stops>().0.values().cloned().collect();
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        out
    }

    pub fn routes(&self) -> &HashMap<RouteId, RouteRecord> {
        &self.world.resource::<Routes>().0
    }

    pub fn link_polyline(&self, link_id: &LinkId) -> Option<Vec<(f32, f32)>> {
        self.world
            .resource::<LinkPolylines>()
            .0
            .get(link_id)
            .cloned()
    }

    /// Public iterator for callers that want all link polylines (used by serde + tests).
    pub fn link_polylines_iter(&self) -> impl Iterator<Item = (&LinkId, &Vec<(f32, f32)>)> + '_ {
        self.world.resource::<LinkPolylines>().0.iter()
    }

    pub fn snapshot(&self) -> MobilitySnapshot {
        MobilitySnapshot {
            agents: self.agents(),
            vehicles: self.vehicles(),
            stops: self.stops(),
        }
    }

    /// Collect all agents + vehicles whose current position falls inside
    /// `chunk`. The new WS subscribe path sends this as a `MobilityChunkSnapshot`
    /// frame so a client gets the current state of newly-subscribed chunks
    /// without waiting for the next tick.
    pub fn build_chunk_snapshot(
        &self,
        chunk: crate::ids::ChunkCoord,
    ) -> MobilityChunkSnapshot {
        let agents = self
            .agents()
            .into_iter()
            .filter(|record| {
                self.world_coord_for_agent(&record.id)
                    .map(|(x, y)| crate::mobility::chunk_of(x, y, 32) == chunk)
                    .unwrap_or(false)
            })
            .collect();
        let vehicles = self
            .vehicles()
            .into_iter()
            .filter(|record| {
                self.world_coord_for_vehicle(&record.id)
                    .map(|(x, y)| crate::mobility::chunk_of(x, y, 32) == chunk)
                    .unwrap_or(false)
            })
            .collect();
        MobilityChunkSnapshot { chunk, agents, vehicles }
    }

    /// Test-only helper: mark a wide range of chunks as `Active` so the LOD
    /// activity filter does not skip them. Used by integration tests that
    /// exercise `tick_mobility()` without standing up a full ChunkSubscribers
    /// pipeline.
    ///
    /// Sets both `ChunkActivities` and `ChunkSubscribers` (1 subscriber per
    /// chunk) so that `classify_activity_system` — which now runs in
    /// `MobilitySet::LOD` before Advance — does not immediately reclassify
    /// these chunks back to `Asleep`.
    #[cfg(test)]
    pub fn force_all_chunks_active_for_test(&mut self) {
        use crate::ids::ChunkCoord;
        use crate::mobility::lod::MobilityActivity;
        let chunks: Vec<ChunkCoord> = (-16..=32)
            .flat_map(|x: i32| (-16..=32).map(move |y| ChunkCoord { x, y }))
            .collect();
        {
            let mut activities = self.world.resource_mut::<ChunkActivities>();
            for chunk in &chunks {
                activities.0.insert(*chunk, MobilityActivity::Active);
            }
        }
        {
            let mut subscribers = self.world.resource_mut::<ChunkSubscribers>();
            for chunk in &chunks {
                subscribers.0.insert(*chunk, 1);
            }
        }
    }

    pub fn tick_mobility(
        &mut self,
    ) -> std::collections::HashMap<crate::ids::ChunkCoord, MobilityChunkDelta> {
        self.schedule.run(&mut self.world);

        // Sync by_agent_id with newly-spawned agents (from promote_warm_to_active_system).
        let mut new_agents: Vec<(AgentId, Entity)> = Vec::new();
        {
            let mut q = self.world.query::<(Entity, &StableAgentId)>();
            for (entity, stable) in q.iter(&self.world) {
                if !self.by_agent_id.contains_key(&stable.0) {
                    new_agents.push((stable.0.clone(), entity));
                }
            }
        }
        for (id, entity) in new_agents {
            self.by_agent_id.insert(id, entity);
        }

        // Remove despawned agents from the index (from demote_active_to_warm_system).
        let agent_ids_to_remove: Vec<AgentId> = self
            .by_agent_id
            .iter()
            .filter(|(_, entity)| self.world.get_entity(**entity).is_err())
            .map(|(id, _)| id.clone())
            .collect();
        for id in agent_ids_to_remove {
            self.by_agent_id.remove(&id);
        }

        // Same for vehicles — sync newly-spawned vehicles.
        let mut new_vehicles: Vec<(VehicleId, Entity)> = Vec::new();
        {
            let mut q = self.world.query::<(Entity, &StableVehicleId)>();
            for (entity, stable) in q.iter(&self.world) {
                if !self.by_vehicle_id.contains_key(&stable.0) {
                    new_vehicles.push((stable.0.clone(), entity));
                }
            }
        }
        for (id, entity) in new_vehicles {
            self.by_vehicle_id.insert(id, entity);
        }

        // Remove despawned vehicles from the index.
        let vehicle_ids_to_remove: Vec<VehicleId> = self
            .by_vehicle_id
            .iter()
            .filter(|(_, entity)| self.world.get_entity(**entity).is_err())
            .map(|(id, _)| id.clone())
            .collect();
        for id in vehicle_ids_to_remove {
            self.by_vehicle_id.remove(&id);
        }

        // Drain dirty sets populated by the Advance systems.
        let dirty_agents: Vec<Entity> =
            std::mem::take(&mut self.world.resource_mut::<DirtyAgents>().0)
                .into_iter()
                .collect();
        let dirty_vehicles: Vec<Entity> =
            std::mem::take(&mut self.world.resource_mut::<DirtyVehicles>().0)
                .into_iter()
                .collect();

        // Build (current chunk → changed records) for agents.
        let mut changed_by_chunk_agents: HashMap<crate::ids::ChunkCoord, Vec<AgentRecord>> =
            HashMap::new();
        let mut current_agent_chunks: HashMap<crate::ids::AgentId, crate::ids::ChunkCoord> =
            HashMap::new();
        for entity in &dirty_agents {
            if let Some(record) = self.agent_record_from_entity(*entity) {
                // Fall back to (0,0) for agents whose geometry cannot be resolved
                // (e.g. link polyline not registered), so they are never silently dropped.
                let (x, y) = self.world_coord_for_agent(&record.id).unwrap_or((0.0, 0.0));
                let chunk = crate::mobility::chunk_of(x, y, 32);
                current_agent_chunks.insert(record.id.clone(), chunk);
                changed_by_chunk_agents.entry(chunk).or_default().push(record);
            }
        }

        // Same for vehicles.
        let mut changed_by_chunk_vehicles: HashMap<crate::ids::ChunkCoord, Vec<VehicleRecord>> =
            HashMap::new();
        let mut current_vehicle_chunks: HashMap<crate::ids::VehicleId, crate::ids::ChunkCoord> =
            HashMap::new();
        for entity in &dirty_vehicles {
            if let Some(record) = self.vehicle_record_from_entity(*entity) {
                // Fall back to (0,0) for vehicles whose geometry cannot be resolved.
                let (x, y) = self.world_coord_for_vehicle(&record.id).unwrap_or((0.0, 0.0));
                let chunk = crate::mobility::chunk_of(x, y, 32);
                current_vehicle_chunks.insert(record.id.clone(), chunk);
                changed_by_chunk_vehicles
                    .entry(chunk)
                    .or_default()
                    .push(record);
            }
        }

        // Compute left_* by comparing current chunk vs PreviousAgentChunks.
        let mut left_by_chunk_agents: HashMap<crate::ids::ChunkCoord, Vec<crate::ids::AgentId>> =
            HashMap::new();
        {
            let prev = self.world.resource::<PreviousAgentChunks>();
            for (id, current_chunk) in &current_agent_chunks {
                if let Some(prev_chunk) = prev.0.get(id)
                    && prev_chunk != current_chunk
                {
                    left_by_chunk_agents
                        .entry(*prev_chunk)
                        .or_default()
                        .push(id.clone());
                }
            }
        }
        let mut left_by_chunk_vehicles: HashMap<
            crate::ids::ChunkCoord,
            Vec<crate::ids::VehicleId>,
        > = HashMap::new();
        {
            let prev = self.world.resource::<PreviousVehicleChunks>();
            for (id, current_chunk) in &current_vehicle_chunks {
                if let Some(prev_chunk) = prev.0.get(id)
                    && prev_chunk != current_chunk
                {
                    left_by_chunk_vehicles
                        .entry(*prev_chunk)
                        .or_default()
                        .push(id.clone());
                }
            }
        }

        // Update PreviousAgentChunks + PreviousVehicleChunks for next tick.
        {
            let mut prev = self.world.resource_mut::<PreviousAgentChunks>();
            for (id, chunk) in &current_agent_chunks {
                prev.0.insert(id.clone(), *chunk);
            }
        }
        {
            let mut prev = self.world.resource_mut::<PreviousVehicleChunks>();
            for (id, chunk) in &current_vehicle_chunks {
                prev.0.insert(id.clone(), *chunk);
            }
        }

        // Assemble per-chunk delta map: union of all chunks that have either
        // a changed entity or a left entity.
        let mut out: HashMap<crate::ids::ChunkCoord, MobilityChunkDelta> = HashMap::new();
        for (chunk, agents) in changed_by_chunk_agents {
            out.entry(chunk)
                .or_insert_with(|| MobilityChunkDelta {
                    chunk,
                    changed_agents: Vec::new(),
                    changed_vehicles: Vec::new(),
                    left_agents: Vec::new(),
                    left_vehicles: Vec::new(),
                })
                .changed_agents = agents;
        }
        for (chunk, vehicles) in changed_by_chunk_vehicles {
            out.entry(chunk)
                .or_insert_with(|| MobilityChunkDelta {
                    chunk,
                    changed_agents: Vec::new(),
                    changed_vehicles: Vec::new(),
                    left_agents: Vec::new(),
                    left_vehicles: Vec::new(),
                })
                .changed_vehicles = vehicles;
        }
        for (chunk, ids) in left_by_chunk_agents {
            out.entry(chunk)
                .or_insert_with(|| MobilityChunkDelta {
                    chunk,
                    changed_agents: Vec::new(),
                    changed_vehicles: Vec::new(),
                    left_agents: Vec::new(),
                    left_vehicles: Vec::new(),
                })
                .left_agents = ids;
        }
        for (chunk, ids) in left_by_chunk_vehicles {
            out.entry(chunk)
                .or_insert_with(|| MobilityChunkDelta {
                    chunk,
                    changed_agents: Vec::new(),
                    changed_vehicles: Vec::new(),
                    left_agents: Vec::new(),
                    left_vehicles: Vec::new(),
                })
                .left_vehicles = ids;
        }

        out
    }

    pub fn spawn_agent_from_record(&mut self, record: AgentRecord) -> Entity {
        let id = record.id.clone();
        let sprite_key = compute_agent_sprite_key(&id);
        let entity = self
            .world
            .spawn((
                AgentMarker,
                StableAgentId(record.id),
                AgentMobilityStateComponent(record.state),
                WalkPlan {
                    stages: record.plan,
                    cursor: record.plan_cursor,
                },
                WalkSpeed(record.walk_speed_per_tick),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(sprite_key),
            ))
            .id();
        self.by_agent_id.insert(id, entity);
        entity
    }

    pub fn spawn_vehicle_from_record(&mut self, record: VehicleRecord) -> Entity {
        let id = record.id.clone();
        let sprite_key = compute_vehicle_sprite_key(&id);
        let entity = self
            .world
            .spawn((
                VehicleMarker,
                StableVehicleId(record.id),
                VehicleKindComponent(record.kind),
                RoutePosition {
                    route_id: record.route_id,
                    link_index: record.link_index,
                    progress: record.progress,
                    speed: record.speed_per_tick,
                },
                Capacity(record.capacity),
                Occupants(record.occupants),
                DwellTicksRemaining(record.dwell_ticks_remaining),
                Position { x: 0.0, y: 0.0 },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(sprite_key),
            ))
            .id();
        self.by_vehicle_id.insert(id, entity);
        entity
    }

    pub fn add_stop(&mut self, stop: StopRecord) {
        self.world
            .resource_mut::<Stops>()
            .0
            .insert(stop.id.clone(), stop);
    }

    pub fn add_route(&mut self, route: RouteRecord) {
        self.world
            .resource_mut::<Routes>()
            .0
            .insert(route.id.clone(), route);
    }

    pub fn set_link_polyline(&mut self, link_id: LinkId, points: Vec<(f32, f32)>) {
        self.world
            .resource_mut::<LinkPolylines>()
            .0
            .insert(link_id, points);
    }

    fn agent_record_from_entity(&self, entity: Entity) -> Option<AgentRecord> {
        let stable = self.world.get::<StableAgentId>(entity)?;
        let state = self.world.get::<AgentMobilityStateComponent>(entity)?;
        let plan = self.world.get::<WalkPlan>(entity)?;
        let speed = self.world.get::<WalkSpeed>(entity)?;
        Some(AgentRecord {
            id: stable.0.clone(),
            state: state.0.clone(),
            plan: plan.stages.clone(),
            plan_cursor: plan.cursor,
            walk_speed_per_tick: speed.0,
        })
    }

    fn vehicle_record_from_entity(&self, entity: Entity) -> Option<VehicleRecord> {
        let stable = self.world.get::<StableVehicleId>(entity)?;
        let kind = self.world.get::<VehicleKindComponent>(entity)?;
        let pos = self.world.get::<RoutePosition>(entity)?;
        let cap = self.world.get::<Capacity>(entity)?;
        let occ = self.world.get::<Occupants>(entity)?;
        let dwell = self.world.get::<DwellTicksRemaining>(entity)?;
        Some(VehicleRecord {
            id: stable.0.clone(),
            kind: kind.0,
            route_id: pos.route_id.clone(),
            link_index: pos.link_index,
            progress: pos.progress,
            speed_per_tick: pos.speed,
            capacity: cap.0,
            occupants: occ.0.clone(),
            dwell_ticks_remaining: dwell.0,
        })
    }

    fn resolve_link_polyline(
        &self,
        link_id: &LinkId,
    ) -> Option<crate::mobility_geometry::LinkGeometry> {
        if let Some(points) = self.world.resource::<LinkPolylines>().0.get(link_id) {
            return Some(crate::mobility_geometry::LinkGeometry {
                points: points.clone(),
            });
        }
        crate::mobility_geometry::link_geometry(&link_id.0)
    }

    pub fn world_coord_for_agent(&self, agent_id: &AgentId) -> Option<(f32, f32)> {
        use crate::mobility_geometry::{activity_geometry, stop_geometry};
        let entity = *self.by_agent_id.get(agent_id)?;
        let state = self.world.get::<AgentMobilityStateComponent>(entity)?;
        match &state.0 {
            AgentMobilityState::AtActivity { activity_id } => {
                activity_geometry(activity_id).map(|g| g.coord)
            }
            AgentMobilityState::Walking { link_id, progress } => {
                let geom = self.resolve_link_polyline(link_id)?;
                Some(geom.world_coord_at_progress(*progress))
            }
            AgentMobilityState::WaitingAtStop { stop_id }
            | AgentMobilityState::Boarding { stop_id, .. }
            | AgentMobilityState::Alighting { stop_id, .. } => {
                stop_geometry(&stop_id.0).map(|g| g.coord)
            }
            AgentMobilityState::InVehicle { vehicle_id, .. } => {
                self.world_coord_for_vehicle(vehicle_id)
            }
        }
    }

    pub fn direction_for_agent(
        &self,
        agent_id: &AgentId,
    ) -> Option<abutown_protocol::DirectionDto> {
        let entity = *self.by_agent_id.get(agent_id)?;
        let state = self.world.get::<AgentMobilityStateComponent>(entity)?;
        match &state.0 {
            AgentMobilityState::Walking { link_id, progress } => {
                let geom = self.resolve_link_polyline(link_id)?;
                Some(geom.direction_at_progress(*progress))
            }
            AgentMobilityState::InVehicle { vehicle_id, .. } => {
                self.direction_for_vehicle(vehicle_id)
            }
            _ => Some(abutown_protocol::DirectionDto::S),
        }
    }

    pub fn world_coord_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<(f32, f32)> {
        let entity = *self.by_vehicle_id.get(vehicle_id)?;
        let pos = self.world.get::<RoutePosition>(entity)?;
        let routes = &self.world.resource::<Routes>().0;
        let route = routes.get(&pos.route_id)?;
        let link_id = route.links.get(pos.link_index)?;
        let geom = self.resolve_link_polyline(link_id)?;
        Some(geom.world_coord_at_progress(pos.progress))
    }

    pub fn direction_for_vehicle(
        &self,
        vehicle_id: &VehicleId,
    ) -> Option<abutown_protocol::DirectionDto> {
        let entity = *self.by_vehicle_id.get(vehicle_id)?;
        let pos = self.world.get::<RoutePosition>(entity)?;
        let routes = &self.world.resource::<Routes>().0;
        let route = routes.get(&pos.route_id)?;
        let link_id = route.links.get(pos.link_index)?;
        let geom = self.resolve_link_polyline(link_id)?;
        Some(geom.direction_at_progress(pos.progress))
    }

    pub fn sprite_key_for_agent(&self, agent_id: &AgentId) -> Option<String> {
        let entity = *self.by_agent_id.get(agent_id)?;
        self.world.get::<SpriteKey>(entity).map(|s| s.0.clone())
    }

    pub fn sprite_key_for_vehicle(&self, vehicle_id: &VehicleId) -> Option<String> {
        let entity = *self.by_vehicle_id.get(vehicle_id)?;
        self.world.get::<SpriteKey>(entity).map(|s| s.0.clone())
    }

    /// Builds an AgentMobilityDto for the given agent id, including the computed
    /// world_coord / direction / sprite_key. Returns None if the agent does not exist.
    pub fn agent_dto_for(&self, agent_id: &AgentId) -> Option<abutown_protocol::AgentMobilityDto> {
        let entity = *self.by_agent_id.get(agent_id)?;
        let state = self.world.get::<AgentMobilityStateComponent>(entity)?;
        let plan = self.world.get::<WalkPlan>(entity)?;
        let stable = self.world.get::<StableAgentId>(entity)?;
        let (cx, cy) = self.world_coord_for_agent(agent_id).unwrap_or((0.0, 0.0));
        let direction = self
            .direction_for_agent(agent_id)
            .unwrap_or(abutown_protocol::DirectionDto::S);
        let sprite_key = self
            .sprite_key_for_agent(agent_id)
            .unwrap_or_else(|| "pedestrian:0".to_string());
        Some(abutown_protocol::AgentMobilityDto {
            id: abutown_protocol::EntityId(stable.0.0.clone()),
            state: abutown_protocol::AgentMobilityStateDto::from(&state.0),
            plan_cursor: plan.cursor,
            world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
            direction,
            sprite_key,
        })
    }

    pub fn vehicle_dto_for(
        &self,
        vehicle_id: &VehicleId,
    ) -> Option<abutown_protocol::VehicleMobilityDto> {
        let entity = *self.by_vehicle_id.get(vehicle_id)?;
        let stable = self.world.get::<StableVehicleId>(entity)?;
        let kind = self.world.get::<VehicleKindComponent>(entity)?;
        let pos = self.world.get::<RoutePosition>(entity)?;
        let cap = self.world.get::<Capacity>(entity)?;
        let occ = self.world.get::<Occupants>(entity)?;
        let dwell = self.world.get::<DwellTicksRemaining>(entity)?;
        let (cx, cy) = self
            .world_coord_for_vehicle(vehicle_id)
            .unwrap_or((0.0, 0.0));
        let direction = self
            .direction_for_vehicle(vehicle_id)
            .unwrap_or(abutown_protocol::DirectionDto::S);
        let sprite_key = self
            .sprite_key_for_vehicle(vehicle_id)
            .unwrap_or_else(|| "tram:0".to_string());
        Some(abutown_protocol::VehicleMobilityDto {
            id: abutown_protocol::EntityId(stable.0.0.clone()),
            kind: kind.0.into(),
            route_id: pos.route_id.0.clone(),
            link_index: pos.link_index,
            progress: pos.progress,
            capacity: cap.0,
            occupants: occ
                .0
                .iter()
                .map(|agent_id| abutown_protocol::EntityId(agent_id.0.clone()))
                .collect(),
            dwell_ticks_remaining: dwell.0,
            world_coord: abutown_protocol::WorldCoordDto { x: cx, y: cy },
            direction,
            sprite_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, LinkId, RouteId, StopId, VehicleId};
    use abutown_protocol::WorldId;
    use std::collections::VecDeque;

    #[test]
    fn initial_world_seeds_expected_population() {
        let world = seed::initial_world();

        assert_eq!(world.tick(), 0);
        assert_eq!(world.routes().len(), 2, "expected 2 routes");

        let snapshot = world.snapshot();
        assert_eq!(snapshot.stops.len(), 4, "expected 4 stops");
        assert_eq!(snapshot.vehicles.len(), 4, "expected 4 vehicles");
        assert_eq!(snapshot.agents.len(), 20, "expected 20 agents");

        for agent in &snapshot.agents {
            assert!(
                !agent.plan.is_empty(),
                "every agent must have at least one plan stage"
            );
        }
        for vehicle in &snapshot.vehicles {
            assert!(vehicle.capacity > 0, "vehicle capacity must be positive");
        }
    }

    #[test]
    fn initial_world_is_deterministic() {
        let a = seed::initial_world();
        let b = seed::initial_world();
        assert_eq!(a, b, "initial_world() must be deterministic across calls");
    }

    #[test]
    fn sample_world_starts_with_agent_walking_to_pickup_stop() {
        let world = sample_world();
        let agent = world
            .agent(&AgentId("agent:pedestrian:0".to_string()))
            .expect("sample agent exists");
        let vehicle = world
            .vehicle(&VehicleId("vehicle:shuttle:0".to_string()))
            .expect("sample vehicle exists");
        let stop = world
            .stop(&StopId("stop:old-town".to_string()))
            .expect("sample stop exists");

        assert_eq!(agent.plan_cursor, 0);
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:home-to-old-town-stop".to_string()),
                progress: 0.0
            }
        );
        assert_eq!(vehicle.route_id, RouteId("route:old-town-loop".to_string()));
        assert_eq!(vehicle.capacity, 4);
        assert_eq!(stop.route_id, RouteId("route:old-town-loop".to_string()));
    }

    #[test]
    fn walking_agent_reaches_pickup_stop_and_waits() {
        let mut world = sample_world();
        world.force_all_chunks_active_for_test();
        let agent_id = AgentId("agent:pedestrian:0".to_string());

        let first_map = world.tick_mobility();
        let agent = world.agent(&agent_id).expect("agent exists");
        assert_eq!(
            agent.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:home-to-old-town-stop".to_string()),
                progress: 0.5
            }
        );
        assert_eq!(
            first_map.values().flat_map(|d| d.changed_agents.iter()).count(),
            1
        );

        let second_map = world.tick_mobility();
        let agent = world.agent(&agent_id).expect("agent exists");
        let stop = world
            .stop(&StopId("stop:old-town".to_string()))
            .expect("pickup stop exists");

        assert_eq!(
            agent.state,
            AgentMobilityState::WaitingAtStop {
                stop_id: StopId("stop:old-town".to_string())
            }
        );
        assert_eq!(agent.plan_cursor, 1);
        assert_eq!(
            stop.waiting_agents.iter().cloned().collect::<Vec<_>>(),
            vec![agent_id]
        );
        assert_eq!(
            second_map.values().flat_map(|d| d.changed_agents.iter()).count(),
            1
        );
    }

    #[test]
    fn vehicle_respects_initial_dwell_then_moves_on_route() {
        let mut world = sample_world();
        world.force_all_chunks_active_for_test();
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        let first_map = world.tick_mobility();
        let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.progress, 0.0);
        assert_eq!(vehicle.dwell_ticks_remaining, 1);
        assert_eq!(
            first_map.values().flat_map(|d| d.changed_vehicles.iter()).count(),
            1
        );

        let second_map = world.tick_mobility();
        let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.progress, 0.0);
        assert_eq!(vehicle.dwell_ticks_remaining, 0);
        assert_eq!(
            second_map.values().flat_map(|d| d.changed_vehicles.iter()).count(),
            1
        );

        let third_map = world.tick_mobility();
        let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.progress, 0.5);
        assert_eq!(vehicle.dwell_ticks_remaining, 0);
        assert_eq!(
            third_map.values().flat_map(|d| d.changed_vehicles.iter()).count(),
            1
        );
    }

    #[test]
    fn agent_boards_rides_alights_and_walks_to_activity() {
        let mut world = sample_world();
        world.force_all_chunks_active_for_test();
        let agent_id = AgentId("agent:pedestrian:0".to_string());
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        world.tick_mobility();
        world.tick_mobility();

        let waiting = world.agent(&agent_id).expect("agent exists");
        assert_eq!(
            waiting.state,
            AgentMobilityState::WaitingAtStop {
                stop_id: StopId("stop:old-town".to_string())
            }
        );

        world.tick_mobility();
        let boarded = world.agent(&agent_id).expect("agent exists");
        let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
        assert_eq!(
            boarded.state,
            AgentMobilityState::InVehicle {
                vehicle_id: vehicle_id.clone(),
                seat_index: 0
            }
        );
        assert_eq!(vehicle.occupants, vec![agent_id.clone()]);

        world.tick_mobility();
        let riding = world.agent(&agent_id).expect("agent exists");
        assert!(matches!(riding.state, AgentMobilityState::InVehicle { .. }));

        world.tick_mobility();
        let alighted = world.agent(&agent_id).expect("agent exists");
        let vehicle = world.vehicle(&vehicle_id).expect("vehicle exists");
        assert_eq!(vehicle.occupants, Vec::<AgentId>::new());
        assert_eq!(
            alighted.state,
            AgentMobilityState::Walking {
                link_id: LinkId("link:station-to-work".to_string()),
                progress: 0.0
            }
        );
        assert_eq!(alighted.plan_cursor, 2);

        world.tick_mobility();
        world.tick_mobility();
        let arrived = world.agent(&agent_id).expect("agent exists");
        assert_eq!(
            arrived.state,
            AgentMobilityState::AtActivity {
                activity_id: "activity:work".to_string()
            }
        );
        assert_eq!(arrived.plan_cursor, 3);
    }

    #[test]
    fn mobility_world_serde_round_trip_preserves_state() {
        let world = sample_world();
        let json = serde_json::to_value(&world).expect("serialize");
        let back: MobilityWorld = serde_json::from_value(json.clone()).expect("deserialize");
        let rejson = serde_json::to_value(&back).expect("re-serialize");
        assert_eq!(json, rejson, "round-trip should preserve state");
    }

    fn sample_world() -> MobilityWorld {
        let route_id = RouteId("route:old-town-loop".to_string());
        let pickup_stop_id = StopId("stop:old-town".to_string());
        let dropoff_stop_id = StopId("stop:station".to_string());
        let walk_to_pickup = LinkId("link:home-to-old-town-stop".to_string());
        let vehicle_link = LinkId("link:old-town-to-station".to_string());
        let walk_to_activity = LinkId("link:station-to-work".to_string());
        let agent_id = AgentId("agent:pedestrian:0".to_string());
        let vehicle_id = VehicleId("vehicle:shuttle:0".to_string());

        let mut routes = HashMap::new();
        routes.insert(
            route_id.clone(),
            RouteRecord {
                id: route_id.clone(),
                links: vec![vehicle_link],
            },
        );

        let mut stops = HashMap::new();
        stops.insert(
            pickup_stop_id.clone(),
            StopRecord {
                id: pickup_stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 0.0,
                waiting_agents: VecDeque::new(),
            },
        );
        stops.insert(
            dropoff_stop_id.clone(),
            StopRecord {
                id: dropoff_stop_id.clone(),
                route_id: route_id.clone(),
                link_index: 0,
                progress: 1.0,
                waiting_agents: VecDeque::new(),
            },
        );

        let mut agents = HashMap::new();
        agents.insert(
            agent_id.clone(),
            AgentRecord::new(
                agent_id,
                AgentMobilityState::Walking {
                    link_id: walk_to_pickup.clone(),
                    progress: 0.0,
                },
                vec![
                    PlanStage::WalkToStop {
                        link_id: walk_to_pickup,
                        stop_id: pickup_stop_id,
                    },
                    PlanStage::RideToStop {
                        route_id: route_id.clone(),
                        stop_id: dropoff_stop_id,
                    },
                    PlanStage::WalkToActivity {
                        link_id: walk_to_activity,
                        activity_id: "activity:work".to_string(),
                    },
                    PlanStage::Activity {
                        activity_id: "activity:work".to_string(),
                    },
                ],
                0.5,
            ),
        );

        let mut vehicles = HashMap::new();
        vehicles.insert(
            vehicle_id.clone(),
            VehicleRecord {
                id: vehicle_id,
                kind: VehicleKind::Tram,
                route_id,
                link_index: 0,
                progress: 0.0,
                speed_per_tick: 0.5,
                capacity: 4,
                occupants: Vec::new(),
                dwell_ticks_remaining: 2,
            },
        );

        let mut world = MobilityWorld::empty();
        for (_, route) in routes {
            world.add_route(route);
        }
        for (_, stop) in stops {
            world.add_stop(stop);
        }
        for (_, agent) in agents {
            world.spawn_agent_from_record(agent);
        }
        for (_, vehicle) in vehicles {
            world.spawn_vehicle_from_record(vehicle);
        }
        world
    }

    #[test]
    fn world_coord_for_walking_agent_interpolates_link() {
        use crate::mobility_geometry::link_geometry;

        let world = seed::initial_world();
        let agent_id = AgentId("agent:seed:0".to_string());
        // Agent is already Walking on link:walk:default at progress 0.0 in tiny_world.
        // We verify interpolation at progress 0.0 (the seeded value).
        let geom = link_geometry("link:walk:default").unwrap();
        let coord = world
            .world_coord_for_agent(&agent_id)
            .expect("agent resolves to coord");
        let expected = geom.world_coord_at_progress(0.0);
        assert!((coord.0 - expected.0).abs() < 0.01);
        assert!((coord.1 - expected.1).abs() < 0.01);
    }

    #[test]
    fn world_coord_for_agent_waiting_at_stop_uses_stop_coord() {
        use crate::mobility_geometry::stop_geometry;

        let mut world = MobilityWorld::empty();
        let stop_id = StopId("stop:horizontal:pickup".to_string());
        let route_id = RouteId("route:horizontal".to_string());
        world.add_route(RouteRecord {
            id: route_id.clone(),
            links: vec![LinkId("link:horizontal:main".to_string())],
        });
        world.add_stop(StopRecord {
            id: stop_id.clone(),
            route_id: route_id.clone(),
            link_index: 0,
            progress: 0.0,
            waiting_agents: VecDeque::new(),
        });
        let agent_id = AgentId("agent:waiter".to_string());
        world.spawn_agent_from_record(AgentRecord::new(
            agent_id.clone(),
            AgentMobilityState::WaitingAtStop {
                stop_id: stop_id.clone(),
            },
            vec![PlanStage::RideToStop {
                route_id,
                stop_id: stop_id.clone(),
            }],
            0.5,
        ));
        let coord = world
            .world_coord_for_agent(&agent_id)
            .expect("agent coord resolves");
        let expected = stop_geometry(&stop_id.0).expect("stop has geometry").coord;
        assert!((coord.0 - expected.0).abs() < 0.01);
        assert!((coord.1 - expected.1).abs() < 0.01);
    }

    #[test]
    fn world_coord_for_transit_vehicle_interpolates_route() {
        use crate::mobility_geometry::link_geometry;

        let mut world = MobilityWorld::empty();
        let route_id = RouteId("route:horizontal".to_string());
        let link_id = LinkId("link:horizontal:main".to_string());
        world.add_route(RouteRecord {
            id: route_id.clone(),
            links: vec![link_id.clone()],
        });
        let vehicle_id = VehicleId("vehicle:test".to_string());
        world.spawn_vehicle_from_record(VehicleRecord {
            id: vehicle_id.clone(),
            kind: VehicleKind::Tram,
            route_id,
            link_index: 0,
            progress: 0.5,
            speed_per_tick: 0.1,
            capacity: 4,
            occupants: Vec::new(),
            dwell_ticks_remaining: 0,
        });
        let coord = world
            .world_coord_for_vehicle(&vehicle_id)
            .expect("vehicle coord resolves");
        let geom = link_geometry(&link_id.0).expect("link geometry exists");
        let expected = geom.world_coord_at_progress(0.5);
        assert!((coord.0 - expected.0).abs() < 0.01);
        assert!((coord.1 - expected.1).abs() < 0.01);
    }

    #[test]
    fn sprite_key_for_agent_is_deterministic_by_id_hash() {
        let world = seed::initial_world();
        let a = world
            .sprite_key_for_agent(&AgentId("agent:seed:0".to_string()))
            .unwrap();
        let b = world
            .sprite_key_for_agent(&AgentId("agent:seed:0".to_string()))
            .unwrap();
        assert_eq!(
            a, b,
            "sprite key must be deterministic across calls for the same id"
        );
        assert!(a.starts_with("pedestrian:"));
    }

    #[test]
    fn agent_dto_built_through_world_includes_world_coord_direction_and_sprite_key() {
        let world = seed::initial_world();
        let agent_id = AgentId("agent:seed:0".to_string());
        let dto = world.agent_dto_for(&agent_id).expect("agent exists");
        assert!(dto.sprite_key.starts_with("pedestrian:"));
        assert!(dto.world_coord.x.is_finite());
    }

    #[test]
    fn seeded_world_vehicles_default_to_tram_kind() {
        let world = seed::initial_world();
        for vehicle in world.vehicles() {
            assert_eq!(vehicle.kind, VehicleKind::Tram);
        }
    }

    #[test]
    fn from_network_produces_expected_population_counts() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};

        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![
                vec![NetworkCoord { x: 10, y: 20 }, NetworkCoord { x: 30, y: 20 }],
                vec![NetworkCoord { x: 40, y: 60 }, NetworkCoord { x: 60, y: 60 }],
            ],
            pedestrian_corridors: vec![
                vec![NetworkCoord { x: 11, y: 30 }, NetworkCoord { x: 31, y: 30 }],
                vec![NetworkCoord { x: 41, y: 70 }, NetworkCoord { x: 61, y: 70 }],
                vec![NetworkCoord { x: 71, y: 80 }, NetworkCoord { x: 91, y: 80 }],
            ],
        };

        let density = seed::SeedDensity {
            pedestrians_per_corridor: 6,
            cars_per_arterial: 4,
            trams_total: 4,
        };
        let world = seed::from_network(&network, density);

        let walking_agents = world
            .agents()
            .into_iter()
            .filter(|a| matches!(a.state, AgentMobilityState::Walking { .. }))
            .count();
        let driving_agents = world
            .agents()
            .into_iter()
            .filter(|a| matches!(a.state, AgentMobilityState::InVehicle { .. }))
            .count();
        let cars = world
            .vehicles()
            .into_iter()
            .filter(|v| v.kind == VehicleKind::Car)
            .count();
        let trams = world
            .vehicles()
            .into_iter()
            .filter(|v| v.kind == VehicleKind::Tram)
            .count();

        assert_eq!(walking_agents, 18, "3 corridors x 6 = 18 walkers");
        assert_eq!(cars, 8, "2 arterials x 4 = 8 cars");
        assert_eq!(driving_agents, 8, "one driver per car");
        assert_eq!(trams, 4);
    }

    #[test]
    fn from_network_is_deterministic() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![vec![
                NetworkCoord { x: 0, y: 5 },
                NetworkCoord { x: 10, y: 5 },
            ]],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 3,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let a = seed::from_network(&network, density);
        let b = seed::from_network(&network, density);
        assert_eq!(a, b);
    }

    #[test]
    fn from_network_assigns_drivers_to_cars() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 0,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let world = seed::from_network(&network, density);

        let vehicles = world.vehicles();
        assert_eq!(vehicles.len(), 2);
        for vehicle in &vehicles {
            assert_eq!(vehicle.kind, VehicleKind::Car);
            assert_eq!(vehicle.capacity, 1);
            assert_eq!(vehicle.occupants.len(), 1, "each car has its driver");
            let driver_id = &vehicle.occupants[0];
            let driver = world.agent(driver_id).expect("driver agent exists");
            match &driver.state {
                AgentMobilityState::InVehicle { vehicle_id, .. } => {
                    assert_eq!(vehicle_id, &vehicle.id);
                }
                other => panic!("driver state expected InVehicle, got {other:?}"),
            }
        }
    }


    #[test]
    fn chunk_of_truncates_to_chunk_grid() {
        use crate::ids::ChunkCoord;
        assert_eq!(chunk_of(0.0, 0.0, 32), ChunkCoord { x: 0, y: 0 });
        assert_eq!(chunk_of(31.9, 31.9, 32), ChunkCoord { x: 0, y: 0 });
        assert_eq!(chunk_of(32.0, 0.0, 32), ChunkCoord { x: 1, y: 0 });
        assert_eq!(chunk_of(150.5, 95.0, 32), ChunkCoord { x: 4, y: 2 });
    }

    #[test]
    fn chunk_of_handles_negative_coords() {
        use crate::ids::ChunkCoord;
        assert_eq!(chunk_of(-0.1, -0.1, 32), ChunkCoord { x: -1, y: -1 });
    }

    #[test]
    fn snapshot_dto_includes_all_agents_even_in_vehicle() {
        use crate::city_network::{CityNetwork, NetworkCoord, WorldTiles};
        let network = CityNetwork {
            version: 1,
            world_id: "test".to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: 256,
                height: 256,
            },
            arterial_paths: vec![vec![
                NetworkCoord { x: 0, y: 0 },
                NetworkCoord { x: 10, y: 0 },
            ]],
            pedestrian_corridors: vec![],
        };
        let density = seed::SeedDensity {
            pedestrians_per_corridor: 0,
            cars_per_arterial: 2,
            trams_total: 0,
        };
        let world = seed::from_network(&network, density);
        let world_id = WorldId("test".to_string());
        let snap = build_mobility_snapshot_dto(&world_id, world.tick(), &world);
        assert_eq!(
            snap.agents.len(),
            2,
            "snapshot must include in_vehicle drivers so clients can hydrate state"
        );
    }


    #[test]
    fn tick_mobility_indexes_lod_spawned_agents() {
        use crate::ids::ChunkCoord;
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let mut world = MobilityWorld::empty();
        world.set_link_polyline(LinkId("l:0".into()), vec![(10.0, 10.0), (20.0, 10.0)]);

        let chunk = ChunkCoord { x: 0, y: 0 };

        world.seed_flow_cell(
            chunk,
            FlowCell {
                population: 2.0,
                outflow: std::collections::HashMap::new(),
                attractiveness: 1.0,
                last_tick: 0,
            },
        );
        // Warm + 1 subscriber: classify_activity_system promotes Warm → Active.
        world.seed_chunk_activity(chunk, MobilityActivity::Warm);
        world.seed_chunk_subscriber_count(chunk, 1);

        world.tick_mobility();

        // Find the spawned agent ids in by_agent_id.
        let lod_agents: Vec<_> = world
            .by_agent_id
            .keys()
            .filter(|id| id.0.starts_with("agent:lod:"))
            .collect();
        assert_eq!(
            lod_agents.len(),
            2,
            "by_agent_id contains 2 LOD-spawned agents"
        );
    }

    #[test]
    fn snapshot_with_flow_cells_and_activities_round_trips() {
        use crate::ids::ChunkCoord;
        use crate::mobility::lod::{FlowCell, MobilityActivity};

        let mut world = MobilityWorld::empty();
        let chunk = ChunkCoord { x: 1, y: 1 };
        world.seed_flow_cell(
            chunk,
            FlowCell {
                population: 4.2,
                outflow: std::collections::HashMap::from([(ChunkCoord { x: 2, y: 1 }, 0.3)]),
                attractiveness: 1.5,
                last_tick: 100,
            },
        );
        world.seed_chunk_activity(chunk, MobilityActivity::Warm);
        let json = serde_json::to_value(&world).unwrap();
        let back: MobilityWorld = serde_json::from_value(json.clone()).unwrap();
        let rejson = serde_json::to_value(&back).unwrap();
        assert_eq!(json, rejson);
    }

    #[test]
    fn tick_mobility_returns_per_chunk_deltas_with_changed_and_left() {
        use crate::ids::{AgentId, ChunkCoord, LinkId};

        let mut world = MobilityWorld::empty();
        world.force_all_chunks_active_for_test();
        // A walkable polyline that crosses chunk boundary: chunk_size 32.
        // The polyline goes from x=4 (deep in chunk 0) to x=60 (chunk 1).
        // Total length = 56.  With walk_speed=0.1 the agent advances
        // progress by 0.1/tick, covering 5.6 world-units/tick.  It reaches
        // x=32 (chunk boundary) after roughly (32-4)/5.6 ≈ 5 ticks, well
        // within the 20-tick window.  On tick 1 it is still at x≈8.6, firmly
        // inside chunk(0,0).
        world.set_link_polyline(LinkId("l".into()), vec![(4.0, 10.0), (60.0, 10.0)]);

        // Walk speed 0.1 per tick so it takes several ticks to cross.
        world.spawn_agent_from_record(AgentRecord::new(
            AgentId("walker".into()),
            AgentMobilityState::Walking { link_id: LinkId("l".into()), progress: 0.0 },
            vec![PlanStage::Activity { activity_id: "act".into() }],
            0.1,
        ));

        // First tick: agent enters world still inside chunk(0,0).
        let map1 = world.tick_mobility();
        assert!(map1.contains_key(&ChunkCoord { x: 0, y: 0 }),
            "tick 1: agent should be in chunk(0,0); map keys: {:?}", map1.keys().collect::<Vec<_>>());
        let delta1 = &map1[&ChunkCoord { x: 0, y: 0 }];
        assert!(!delta1.changed_agents.is_empty());
        assert!(delta1.left_agents.is_empty(), "first tick: no previous chunk to leave");

        // Tick enough times to cross into chunk(1,0).
        let mut crossed = false;
        for _ in 0..20 {
            let map = world.tick_mobility();
            if let Some(delta) = map.get(&ChunkCoord { x: 0, y: 0 })
                && !delta.left_agents.is_empty()
            {
                assert!(
                    delta.left_agents.iter().any(|id| id.0 == "walker"),
                    "walker shows up in chunk(0,0).left_agents when it crosses out"
                );
                assert!(
                    map.get(&ChunkCoord { x: 1, y: 0 })
                        .map(|d| d.changed_agents.iter().any(|r| r.id.0 == "walker"))
                        .unwrap_or(false),
                    "walker shows up in chunk(1,0).changed_agents when it crosses in"
                );
                crossed = true;
                break;
            }
        }
        assert!(crossed, "agent must cross chunk boundary within 20 ticks");
    }

    #[test]
    fn tick_mobility_omits_unchanged_chunks() {
        use crate::ids::{AgentId, ChunkCoord, LinkId};
        let mut world = MobilityWorld::empty();
        world.force_all_chunks_active_for_test();
        world.set_link_polyline(LinkId("l".into()), vec![(10.0, 10.0), (20.0, 10.0)]);

        // walk_speed=0 → no progress change → no dirty agents → empty delta map.
        world.spawn_agent_from_record(AgentRecord::new(
            AgentId("stationary".into()),
            AgentMobilityState::Walking { link_id: LinkId("l".into()), progress: 0.0 },
            vec![PlanStage::Activity { activity_id: "act".into() }],
            0.0,
        ));

        // First tick spawns the agent → it's "changed" because newly created.
        let _ = world.tick_mobility();

        // Second tick: no movement, no plan transitions.
        let map = world.tick_mobility();
        assert!(
            map.get(&ChunkCoord { x: 0, y: 0 })
                .map(|d| d.changed_agents.is_empty() && d.left_agents.is_empty())
                .unwrap_or(true),
            "chunk with no changes should either be absent or have empty changed/left lists"
        );
    }

    #[test]
    fn build_chunk_snapshot_returns_only_entities_in_that_chunk() {
        use crate::ids::{AgentId, ChunkCoord, LinkId};

        let mut world = MobilityWorld::empty();

        // Two distinct chunks at chunk_size=32: chunk (0,0) covers world x,y in [0,32);
        // chunk (1,0) covers x in [32,64), y in [0,32).
        world.set_link_polyline(LinkId("l:a".into()), vec![(10.0, 10.0), (20.0, 10.0)]);
        world.set_link_polyline(LinkId("l:b".into()), vec![(40.0, 10.0), (50.0, 10.0)]);

        world.spawn_agent_from_record(AgentRecord::new(
            AgentId("agent-a".into()),
            AgentMobilityState::Walking { link_id: LinkId("l:a".into()), progress: 0.0 },
            vec![PlanStage::Activity { activity_id: "act".into() }],
            0.0,
        ));
        world.spawn_agent_from_record(AgentRecord::new(
            AgentId("agent-b".into()),
            AgentMobilityState::Walking { link_id: LinkId("l:b".into()), progress: 0.0 },
            vec![PlanStage::Activity { activity_id: "act".into() }],
            0.0,
        ));

        // Ensure chunks are active so tick_mobility does not demote/despawn agents.
        world.force_all_chunks_active_for_test();
        // Tick once so compute_world_coord_system runs and Position components are set.
        world.tick_mobility();

        let snapshot = world.build_chunk_snapshot(ChunkCoord { x: 0, y: 0 });
        let agent_ids: Vec<String> = snapshot.agents.iter().map(|a| a.id.0.clone()).collect();
        assert_eq!(agent_ids, vec!["agent-a"], "snapshot returns only chunk(0,0) agents");
        assert!(snapshot.vehicles.is_empty());
        assert_eq!(snapshot.chunk, ChunkCoord { x: 0, y: 0 });
    }
}
