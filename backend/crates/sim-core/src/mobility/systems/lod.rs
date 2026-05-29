use super::*;

// Phase 8a follow-ups removed the resource-only compat shim
// (`classify_activity_system` + `ChunkActivities` / `ChunkSubscribers`
// resources). Chunk LOD is now classified once, on the chunk entity, by
// `crate::world::systems::reclassify_chunk_lod_system` under
// `CoreSet::LodReclassify`. Mobility consumes the resulting
// `ChunkLodChanged` event stream via
// `consume_chunk_lod_transitions_system` (below), which fills the
// `ChunkLodTransitions` scratchpad that promote/demote drain.

/// Rebuild the `SimulatedChunks` + `WarmChunkCoords` derived views from the
/// chunk-entity LOD markers. Runs at the head of `MobilitySet::LOD` each
/// tick so all downstream systems see a consistent view of which chunks
/// are simulated this tick.
pub fn refresh_simulated_chunks_system(
    hot: Query<&ChunkCoordComp, With<HotChunk>>,
    active: Query<&ChunkCoordComp, With<ActiveChunk>>,
    warm: Query<&ChunkCoordComp, With<WarmChunk>>,
    mut simulated: ResMut<SimulatedChunks>,
    mut warm_view: ResMut<WarmChunkCoords>,
) {
    simulated.0.clear();
    for c in hot.iter().chain(active.iter()) {
        simulated.0.insert(c.0);
    }
    warm_view.0.clear();
    for c in warm.iter() {
        warm_view.0.insert(c.0);
    }
}

/// Drain `ChunkLodChanged` messages emitted by the foundation's
/// `reclassify_chunk_lod_system` and stash them in the
/// `ChunkLodTransitions` scratchpad for promote/demote to consume.
///
/// The `Local<MessageCursor<…>>` survives across ticks, so even if a tick
/// is delayed the consumer never misses a transition.
pub fn consume_chunk_lod_transitions_system(
    mut cursor: Local<MessageCursor<ChunkLodChanged>>,
    messages: Res<Messages<ChunkLodChanged>>,
    mut out: ResMut<ChunkLodTransitions>,
) {
    out.0.clear();
    for event in cursor.read(&messages) {
        out.0.push((event.coord, event.from, event.to));
    }
}

pub fn promote_warm_to_active_system(
    transitions: Res<ChunkLodTransitions>,
    mut flow_cells: ResMut<FlowCells>,
    graph: Res<crate::routing::Graph>,
    tick: Res<Tick>,
    mut commands: Commands,
) {
    for (chunk, prev, next) in &transitions.0 {
        if *prev != ChunkLod::Warm {
            continue;
        }
        if !matches!(next, ChunkLod::Active | ChunkLod::Hot) {
            continue;
        }
        let Some(cell) = flow_cells.0.get_mut(chunk) else {
            continue;
        };
        let to_spawn = cell.population.floor() as u32;
        if to_spawn == 0 {
            continue;
        }

        // Find a link whose polyline passes through this chunk.
        let mut spawn_link: Option<String> = None;
        for edge in graph.edges() {
            if edge.kind == crate::routing::EdgeKind::Footway
                && edge
                    .polyline
                    .iter()
                    .any(|(x, y)| crate::mobility::chunk_of(*x, *y, 32) == *chunk)
                && let Some(legacy_id) = &edge.legacy_id
            {
                spawn_link = Some(legacy_id.clone());
                break;
            }
        }
        let Some(spawn_link) = spawn_link else {
            continue;
        };

        for n in 0..to_spawn {
            let agent_id = crate::ids::AgentId(format!(
                "agent:lod:{}:{}:{}:{}",
                chunk.x, chunk.y, tick.0, n
            ));
            // Deterministic pseudo-random progress in [0, 1).
            let seed = lod_seed(chunk.x, chunk.y, tick.0, n as u64);
            let progress = (seed % 1000) as f32 / 1000.0;
            let sprite_key = format!("pedestrian:{}", seed % 16);
            let spawned_state = crate::mobility::records::AgentMobilityState::Walking {
                link_id: spawn_link.clone(),
                progress,
            };
            let (px, py) = crate::mobility::agent_world_coord(&spawned_state, &graph)
                .expect("LOD promoted walking agent must resolve through routing graph");
            commands.spawn((
                AgentMarker,
                StableAgentId(agent_id),
                AgentMobilityStateComponent(spawned_state),
                WalkPlan {
                    stages: vec![crate::mobility::records::PlanStage::Activity {
                        activity_id: format!("activity:lod:{}:{}:{}", chunk.x, chunk.y, n),
                    }],
                    cursor: 0,
                },
                WalkSpeed(0.05),
                Position { x: px, y: py },
                Direction(abutown_protocol::DirectionDto::S),
                SpriteKey(sprite_key),
            ));
        }
        cell.population -= to_spawn as f32;
        cell.outflow.clear();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn demote_active_to_warm_system(
    transitions: Res<ChunkLodTransitions>,
    agents: Query<&AgentMobilityStateComponent, With<AgentMarker>>,
    agents_by_chunk: Res<AgentsByChunk>,
    graph: Res<crate::routing::Graph>,
    mut flow_cells: ResMut<FlowCells>,
    mut commands: Commands,
) {
    // Trigger on any transition *into* Warm, regardless of the previous state.
    // The legacy `Active|Hot → Warm` restriction missed the production path
    // where agents are seeded directly into chunks (snapshot hydration,
    // `from_network`) and the chunk's very first classification is
    // `Asleep → Warm`. Those agents would otherwise stay alive forever and
    // the per-tick Advance/Output systems would pay the full O(N) cost.
    for (chunk, _prev, next) in &transitions.0 {
        if *next != ChunkLod::Warm {
            continue;
        }

        let Some(agent_entities) = agents_by_chunk.0.get(chunk) else {
            continue;
        };

        let mut despawn_count = 0u32;
        let mut outflow_counts: std::collections::HashMap<crate::ids::ChunkCoord, u32> =
            std::collections::HashMap::new();

        for entity in agent_entities {
            let Ok(state) = agents.get(*entity) else {
                continue;
            };
            let dest = agent_destination_chunk(state, &graph).unwrap_or(*chunk);
            despawn_count += 1;
            *outflow_counts.entry(dest).or_insert(0) += 1;
            commands.entity(*entity).despawn();
        }

        if despawn_count == 0 {
            continue;
        }

        let cell = flow_cells.0.entry(*chunk).or_default();
        cell.population += despawn_count as f32;
        for (dest, count) in outflow_counts {
            let rate = count as f32 / 100.0; // amortise over ~100 ticks
            *cell.outflow.entry(dest).or_insert(0.0) += rate;
        }
    }
}

fn agent_destination_chunk(
    state: &AgentMobilityStateComponent,
    graph: &crate::routing::Graph,
) -> Option<crate::ids::ChunkCoord> {
    if let AgentMobilityState::Walking { link_id, .. } = &state.0 {
        return crate::mobility::api::edge_by_canonical_key(graph, link_id)
            .and_then(|edge_id| graph.edge(edge_id).polyline.last().copied())
            .map(|(x, y)| crate::mobility::chunk_of(x, y, 32));
    }
    crate::mobility::agent_world_coord(&state.0, graph)
        .map(|(x, y)| crate::mobility::chunk_of(x, y, 32))
}

pub fn warm_chunk_flow_system(
    tick: Res<Tick>,
    warm: Res<WarmChunkCoords>,
    mut flow_cells: ResMut<FlowCells>,
) {
    if !tick.0.is_multiple_of(10) {
        return;
    }

    let warm_chunks: Vec<crate::ids::ChunkCoord> = warm.0.iter().copied().collect();

    let mut transfers: Vec<(crate::ids::ChunkCoord, crate::ids::ChunkCoord, f32)> = Vec::new();
    for chunk in &warm_chunks {
        let Some(cell) = flow_cells.0.get(chunk) else {
            continue;
        };
        for (dest, rate) in &cell.outflow {
            let delta = (rate * 10.0).min(cell.population);
            if delta > 0.0 {
                transfers.push((*chunk, *dest, delta));
            }
        }
    }
    for (from, to, delta) in transfers {
        if let Some(cell) = flow_cells.0.get_mut(&from) {
            cell.population = (cell.population - delta).max(0.0);
            cell.last_tick = tick.0;
        }
        let dest_cell = flow_cells.0.entry(to).or_default();
        dest_cell.population += delta;
        dest_cell.last_tick = tick.0;
    }
}

fn lod_seed(x: i32, y: i32, tick: u64, n: u64) -> u64 {
    // FNV-1a hash for deterministic seeding (does NOT depend on RandomState).
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in (x as u32)
        .to_le_bytes()
        .iter()
        .chain((y as u32).to_le_bytes().iter())
        .chain(tick.to_le_bytes().iter())
        .chain(n.to_le_bytes().iter())
    {
        h ^= *byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
