//! Render-only bridge: materialize each economy trader as a walking mobility
//! agent on the real footway graph, feeding the per-tick mobility delta so the
//! client renders smooth motion. Never mutates economy state (conservation-safe).
//!
//! The trader-agent carries render components ONLY (no `AgentMarker`), so no
//! mobility movement/bookkeeping system touches it. It is dirtied every tick at
//! its current routed position; the standard `tick_mobility` delta (left_agents
//! on chunk change) handles client visibility exactly like normal agents.
//! Per-chunk LOD despawn of unobserved trader-agents is a deferred optimization;
//! the economy's dormant gate already bounds how many traders advance.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use crate::economy::trader_render::{is_outbound, leg_progress, route_polyline, trader_travel};
use crate::economy::{EconomicActorId, EconomyConfig, Markets, Trader, Traders};
use crate::ids::AgentId;
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
use abutown_protocol::DirectionDto;

/// Maps each economy trader (by actor) to its materialized render entity.
#[derive(Resource, Default)]
pub struct MaterializedTraders(pub BTreeMap<EconomicActorId, Entity>);

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
/// route polylines (keyed by actor). No ECS world access — fully unit-testable.
pub(crate) fn plan_mutations(
    traders: &Traders,
    config: &EconomyConfig,
    materialized: &MaterializedTraders,
    routes: &BTreeMap<EconomicActorId, Vec<(f32, f32)>>,
) -> Vec<TraderMutation> {
    let mut muts = Vec::new();
    for (actor, trader) in &traders.0 {
        let Some(polyline) = routes.get(actor) else {
            continue;
        };
        if polyline.is_empty() {
            continue;
        }
        let travel = trader_travel(trader, config);
        let t = leg_progress(&trader.state, travel);
        let (x, y) = world_coord_at_progress_slice(polyline, t);
        let (nx, ny) = world_coord_at_progress_slice(polyline, (t + 0.02).min(1.0));
        let dir = dir_from_delta(nx - x, ny - y);
        if materialized.0.contains_key(actor) {
            muts.push(TraderMutation::Update {
                actor: *actor,
                x,
                y,
                dir,
            });
        } else {
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
                        BirthTick(tick),
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
                world
                    .resource_mut::<MaterializedTraders>()
                    .0
                    .insert(actor, entity);
            }
            TraderMutation::Update { actor, x, y, dir } => {
                let Some(entity) = world
                    .resource::<MaterializedTraders>()
                    .0
                    .get(&actor)
                    .copied()
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
            }
            TraderMutation::Despawn { actor } => {
                let Some(entity) = world
                    .resource::<MaterializedTraders>()
                    .0
                    .get(&actor)
                    .copied()
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
        plan_mutations(traders, config, materialized, &routes)
    };
    apply_mutations(world, tick, muts);
}
