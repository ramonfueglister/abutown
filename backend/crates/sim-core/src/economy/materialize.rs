//! Render-only bridge: materialize each economy trader as a walking mobility
//! agent on the real footway graph, feeding the per-tick mobility delta so the
//! client renders smooth motion. Never mutates economy state (conservation-safe).
//!
//! The trader-agent carries render components ONLY (no `AgentMarker`), so no
//! mobility movement/bookkeeping system touches it. **LOD-consistent like the rest
//! of the sim:** a trader-agent exists only while its current position is in an
//! observed (Active/Hot) chunk. When it walks out of observed chunks it is
//! despawned — but ghost-free: on the leaving tick it is dirtied at its new
//! (unobserved) position so `tick_mobility` emits `left_agents` for the chunk it
//! left (the client clears it), and it is despawned the following tick. This
//! mirrors how mobility demotes its agents.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use bevy_ecs::query::Or;

use crate::economy::trader_render::{is_outbound, leg_progress, route_polyline, trader_travel};
use crate::economy::{EconomicActorId, EconomyConfig, Markets, Trader, Traders};
use crate::ids::{AgentId, ChunkCoord};
use crate::mobility::AgentMobilityState;
use crate::mobility::components::{
    AgentMobilityStateComponent, BirthTick, Direction, Position, SpriteKey, StableAgentId,
    TraderAgent, WalkPlan, WalkSpeed,
};
use crate::mobility::resources::{AgentIdIndex, DirtyAgents, Tick};
use crate::mobility_geometry::world_coord_at_progress_slice;
use crate::routing::{
    EdgeId, FlowFieldCache, FlowFieldCacheKey, FlowFieldScope, Graph, HpaIndex, ModeState, NodeId,
    RoutingProfile, RoutingProfileKey,
};
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk};
use abutown_protocol::DirectionDto;

/// Render bookkeeping for one materialized trader.
#[derive(Debug, Clone, Copy)]
pub struct MaterializedTrader {
    pub entity: Entity,
    /// Whether the trader's chunk was observed (Active/Hot) last tick. Drives the
    /// one-tick "dirty-then-despawn" so the client gets a clean `left_agents`
    /// removal when the trader walks out of view.
    pub observed: bool,
}

/// Maps each economy trader (by actor) to its materialized render entity.
#[derive(Resource, Default)]
pub struct MaterializedTraders(pub BTreeMap<EconomicActorId, MaterializedTrader>);

/// A render mutation produced by `plan_mutations` and applied by `apply_mutations`.
pub(crate) enum TraderMutation {
    Spawn {
        actor: EconomicActorId,
        agent_id: AgentId,
        x: f32,
        y: f32,
        dir: DirectionDto,
        sprite: String,
    },
    Update {
        actor: EconomicActorId,
        x: f32,
        y: f32,
        dir: DirectionDto,
        observed: bool,
    },
    Despawn {
        actor: EconomicActorId,
    },
}

/// Deterministic sprite-variant index for a trader id (FNV-1a, 8 variants).
fn sprite_hash(id: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h % 8
}

/// 8-way facing from a movement delta (screen-space atan2 octant).
fn dir_from_delta(dx: f32, dy: f32) -> DirectionDto {
    use DirectionDto as D;
    if dx == 0.0 && dy == 0.0 {
        return D::S;
    }
    let octant = (((dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() as i32) + 8) % 8;
    [D::E, D::Se, D::S, D::Sw, D::W, D::Nw, D::N, D::Ne][octant as usize]
}

fn endpoints(markets: &Markets, trader: &Trader) -> Option<(NodeId, NodeId)> {
    Some((
        markets.0.get(&trader.source)?.node_id,
        markets.0.get(&trader.dest)?.node_id,
    ))
}

/// Compute a Walk footway route polyline between two graph nodes. `from == to`
/// short-circuits to the single node position (no flow-field build). Returns
/// `None` if no walkable route exists.
fn leg_polyline(
    graph: &Graph,
    hpa: &HpaIndex,
    cache: &mut FlowFieldCache,
    from: NodeId,
    to: NodeId,
) -> Option<Vec<(f32, f32)>> {
    if from == to {
        return Some(vec![graph.node(from).position]);
    }
    let corridor = hpa
        .corridor_between(from, to, RoutingProfileKey::Walk)
        .ok()?;
    let mut corridor_key: Vec<_> = corridor.iter().copied().collect();
    corridor_key.sort_unstable();
    let key = FlowFieldCacheKey::new(to, RoutingProfileKey::Walk, 0, &corridor_key);
    let scope = FlowFieldScope::Corridor(corridor);
    let profile = RoutingProfile::for_key(RoutingProfileKey::Walk);
    let field = cache
        .get_or_build_with_cluster_lookup(graph, key, profile, scope, |n| hpa.cluster_of_node(n))
        .ok()?;
    let steps =
        crate::mobility::systems::materialize_route_steps(graph, &field, from, ModeState::Walking)?;
    let edges: Vec<EdgeId> = steps.iter().map(|s| s.edge_id).collect();
    let poly = route_polyline(graph, &edges);
    if poly.is_empty() { None } else { Some(poly) }
}

/// Pure planner: decide the render mutation for each trader given the current-leg
/// route polylines (keyed by actor) and the set of observed (Active/Hot) chunks.
/// No ECS world access — fully unit-testable.
pub(crate) fn plan_mutations(
    traders: &Traders,
    config: &EconomyConfig,
    materialized: &MaterializedTraders,
    routes: &BTreeMap<EconomicActorId, Vec<(f32, f32)>>,
    observed: &BTreeSet<ChunkCoord>,
) -> Vec<TraderMutation> {
    let mut muts = Vec::new();
    for (actor, trader) in &traders.0 {
        let was_observed = materialized.0.get(actor).map(|m| m.observed);
        let Some(polyline) = routes.get(actor).filter(|p| !p.is_empty()) else {
            // No walkable route this tick: a materialized agent can't be positioned,
            // so retire it.
            if was_observed.is_some() {
                muts.push(TraderMutation::Despawn { actor: *actor });
            }
            continue;
        };
        let travel = trader_travel(trader, config);
        let t = leg_progress(&trader.state, travel);
        let (x, y) = world_coord_at_progress_slice(polyline, t);
        let (nx, ny) = world_coord_at_progress_slice(polyline, (t + 0.02).min(1.0));
        let dir = dir_from_delta(nx - x, ny - y);
        let observed_now = observed.contains(&crate::mobility::chunk_of(x, y, 32));
        match (observed_now, was_observed) {
            // Appear: first time its current chunk is observed.
            (true, None) => {
                let agent_id = AgentId(format!("trader:{}", actor.0));
                let sprite = format!("trader:{}", sprite_hash(&agent_id.0));
                muts.push(TraderMutation::Spawn {
                    actor: *actor,
                    agent_id,
                    x,
                    y,
                    dir,
                    sprite,
                });
            }
            // Move while observed (or re-observed after being away).
            (true, Some(_)) => muts.push(TraderMutation::Update {
                actor: *actor,
                x,
                y,
                dir,
                observed: true,
            }),
            // Just walked out of observed chunks: nudge to the new (unobserved)
            // position + dirty so `tick_mobility` emits `left_agents` for the chunk
            // it left; keep the entity alive this one tick.
            (false, Some(true)) => muts.push(TraderMutation::Update {
                actor: *actor,
                x,
                y,
                dir,
                observed: false,
            }),
            // Still unobserved after the leave was emitted: despawn (LOD).
            (false, Some(false)) => muts.push(TraderMutation::Despawn { actor: *actor }),
            // Not materialized and unobserved: nothing to do.
            (false, None) => {}
        }
    }
    // Despawn agents whose trader has been removed from `Traders`.
    for actor in materialized.0.keys() {
        if !traders.0.contains_key(actor) {
            muts.push(TraderMutation::Despawn { actor: *actor });
        }
    }
    muts
}

/// Apply render mutations to the world: spawn/update/despawn the trader-agent
/// entities and keep `AgentIdIndex`/`DirtyAgents`/`MaterializedTraders` in sync.
pub(crate) fn apply_mutations(world: &mut World, tick: u64, muts: Vec<TraderMutation>) {
    for m in muts {
        match m {
            TraderMutation::Spawn {
                actor,
                agent_id,
                x,
                y,
                dir,
                sprite,
            } => {
                let entity = world
                    .spawn((
                        TraderAgent,
                        StableAgentId(agent_id.clone()),
                        AgentMobilityStateComponent(AgentMobilityState::AtActivity {
                            activity_id: "trader".to_string(),
                        }),
                        WalkPlan {
                            stages: vec![],
                            cursor: 0,
                            cyclic: false,
                        },
                        WalkSpeed(0.0),
                        BirthTick(i64::try_from(tick).unwrap_or(i64::MAX)),
                        Position { x, y },
                        Direction(dir),
                        SpriteKey(sprite),
                    ))
                    .id();
                world
                    .resource_mut::<AgentIdIndex>()
                    .0
                    .insert(agent_id, entity);
                world.resource_mut::<DirtyAgents>().0.insert(entity);
                world.resource_mut::<MaterializedTraders>().0.insert(
                    actor,
                    MaterializedTrader {
                        entity,
                        observed: true,
                    },
                );
            }
            TraderMutation::Update {
                actor,
                x,
                y,
                dir,
                observed,
            } => {
                let Some(entity) = world
                    .resource::<MaterializedTraders>()
                    .0
                    .get(&actor)
                    .map(|m| m.entity)
                else {
                    continue;
                };
                if let Some(mut p) = world.get_mut::<Position>(entity) {
                    p.x = x;
                    p.y = y;
                }
                if let Some(mut d) = world.get_mut::<Direction>(entity) {
                    d.0 = dir;
                }
                world.resource_mut::<DirtyAgents>().0.insert(entity);
                if let Some(m) = world
                    .resource_mut::<MaterializedTraders>()
                    .0
                    .get_mut(&actor)
                {
                    m.observed = observed;
                }
            }
            TraderMutation::Despawn { actor } => {
                let Some(entity) = world
                    .resource::<MaterializedTraders>()
                    .0
                    .get(&actor)
                    .map(|m| m.entity)
                else {
                    continue;
                };
                world.despawn(entity);
                world
                    .resource_mut::<AgentIdIndex>()
                    .0
                    .remove(&AgentId(format!("trader:{}", actor.0)));
                world.resource_mut::<MaterializedTraders>().0.remove(&actor);
            }
        }
    }
}

/// Exclusive system: compute each trader's current-leg footway route, then plan
/// and apply the render mutations. Routes are computed into an owned map first
/// (releasing the routing borrows) so the apply phase has clean `&mut World`.
pub fn materialize_traders_system(world: &mut World) {
    // The render bridge needs the spatial world (graph + routing indices). In
    // pure-economy schedules (no RoutingPlugin) there is nothing to route, so the
    // bridge is a no-op — keeping the economy schedule runnable without a graph
    // (the economy-lod-v0 "economy core stays graph-free" invariant).
    if world.get_resource::<Graph>().is_none()
        || world.get_resource::<HpaIndex>().is_none()
        || world.get_resource::<FlowFieldCache>().is_none()
    {
        return;
    }
    let tick = world.get_resource::<Tick>().map(|t| t.0).unwrap_or(0);

    let observed: BTreeSet<ChunkCoord> = {
        let mut q =
            world.query_filtered::<&ChunkCoordComp, Or<(With<ActiveChunk>, With<HotChunk>)>>();
        q.iter(world).map(|c| c.0).collect()
    };

    let routes: BTreeMap<EconomicActorId, Vec<(f32, f32)>> =
        world.resource_scope(|world: &mut World, mut cache: Mut<FlowFieldCache>| {
            let graph = world.resource::<Graph>();
            let hpa = world.resource::<HpaIndex>();
            let markets = world.resource::<Markets>();
            let traders = world.resource::<Traders>();
            let mut routes = BTreeMap::new();
            for (actor, trader) in &traders.0 {
                let Some((src, dst)) = endpoints(markets, trader) else {
                    continue;
                };
                let (a, b) = if is_outbound(&trader.state) {
                    (src, dst)
                } else {
                    (dst, src)
                };
                if let Some(poly) = leg_polyline(graph, hpa, &mut cache, a, b) {
                    routes.insert(*actor, poly);
                }
            }
            routes
        });

    let muts = {
        let traders = world.resource::<Traders>();
        let config = world.resource::<EconomyConfig>();
        let materialized = world.resource::<MaterializedTraders>();
        plan_mutations(traders, config, materialized, &routes, &observed)
    };
    apply_mutations(world, tick, muts);
}
