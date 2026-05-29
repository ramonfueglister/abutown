use super::*;

pub fn tick_increment_system(mut tick: ResMut<Tick>) {
    tick.0 += 1;
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn track_chunk_populations_system(
    moved_agents: Query<(Entity, &Position), (With<AgentMarker>, Changed<Position>)>,
    moved_vehicles: Query<(Entity, &Position), (With<VehicleMarker>, Changed<Position>)>,
    all_agents: Query<(Entity, &Position), With<AgentMarker>>,
    all_vehicles: Query<(Entity, &Position), With<VehicleMarker>>,
    flow_cells: Res<FlowCells>,
    mut populations: ResMut<ChunkPopulations>,
    mut agents_by_chunk: ResMut<AgentsByChunk>,
    mut vehicles_by_chunk: ResMut<VehiclesByChunk>,
    mut previous: ResMut<crate::mobility::resources::PreviousChunkByEntity>,
    mut prev_flow: ResMut<crate::mobility::resources::PreviousFlowCellContrib>,
) {
    use std::collections::HashMap;

    let first_run = previous.0.is_empty();
    if first_run {
        // First run after world creation / hydration: full rebuild.
        agents_by_chunk.0.clear();
        vehicles_by_chunk.0.clear();
        populations.0.clear();
        for (entity, pos) in all_agents.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            agents_by_chunk.0.entry(chunk).or_default().insert(entity);
            previous.0.insert(entity, chunk);
        }
        for (entity, pos) in all_vehicles.iter() {
            let chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            *populations.0.entry(chunk).or_insert(0) += 1;
            vehicles_by_chunk.0.entry(chunk).or_default().insert(entity);
            previous.0.insert(entity, chunk);
        }
    } else {
        // Step A: undo the previous tick's FlowCell aggregate so the
        // entity-count deltas below operate on a clean entity-only base.
        for (chunk, amount) in prev_flow.0.drain() {
            if let Some(p) = populations.0.get_mut(&chunk) {
                *p = p.saturating_sub(amount);
            }
        }

        // Step B: incremental rebucketing of moved entities.
        for (entity, pos) in moved_agents.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = agents_by_chunk.0.get_mut(&old_chunk) {
                    bucket.remove(&entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            agents_by_chunk
                .0
                .entry(new_chunk)
                .or_default()
                .insert(entity);
            previous.0.insert(entity, new_chunk);
        }
        for (entity, pos) in moved_vehicles.iter() {
            let new_chunk = crate::mobility::chunk_of(pos.x, pos.y, 32);
            if let Some(old_chunk) = previous.0.get(&entity).copied() {
                if old_chunk == new_chunk {
                    continue;
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.remove(&entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
            *populations.0.entry(new_chunk).or_insert(0) += 1;
            vehicles_by_chunk
                .0
                .entry(new_chunk)
                .or_default()
                .insert(entity);
            previous.0.insert(entity, new_chunk);
        }

        // Step C: reconcile despawns — any entity in `previous` that no
        // longer has Position is removed from its bucket.
        let stale: Vec<Entity> = previous
            .0
            .keys()
            .copied()
            .filter(|e| all_agents.get(*e).is_err() && all_vehicles.get(*e).is_err())
            .collect();
        for entity in stale {
            if let Some(old_chunk) = previous.0.remove(&entity) {
                if let Some(bucket) = agents_by_chunk.0.get_mut(&old_chunk) {
                    bucket.remove(&entity);
                }
                if let Some(bucket) = vehicles_by_chunk.0.get_mut(&old_chunk) {
                    bucket.remove(&entity);
                }
                if let Some(p) = populations.0.get_mut(&old_chunk) {
                    *p = p.saturating_sub(1);
                }
            }
        }
    }

    // Step D: re-add current FlowCell aggregate and remember it for next tick.
    let mut current_flow: HashMap<crate::ids::ChunkCoord, u32> = HashMap::new();
    for (chunk, cell) in &flow_cells.0 {
        let aggregate = cell.population.floor().max(0.0) as u32;
        if aggregate > 0 {
            *populations.0.entry(*chunk).or_insert(0) += aggregate;
            current_flow.insert(*chunk, aggregate);
        }
    }
    prev_flow.0 = current_flow;

    // Drop empty buckets so demote doesn't pay for dead entries.
    agents_by_chunk.0.retain(|_, bucket| !bucket.is_empty());
    vehicles_by_chunk.0.retain(|_, bucket| !bucket.is_empty());
}
