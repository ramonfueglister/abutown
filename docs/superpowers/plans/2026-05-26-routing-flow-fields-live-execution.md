# Routing Flow Fields Live Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Build graph flow fields and wire walking agents to execute multi-edge graph routes in the live mobility schedule.

**Architecture:** Add `routing/flow_field.rs` as the batch-routing layer over `Graph`, `RoutingProfile`, and HPA corridor clusters. Add `FlowFieldCache` through a plugin, then add mobility `ActiveRoute` execution components and systems that assign and advance walking routes without changing vehicle/tram execution. Persist route execution in the existing JSONB mobility payload and tighten frontend proto decoding so malformed mobility frames are not rendered as valid state.

**Tech Stack:** Rust 2024, `bevy_ecs 0.18`, existing `sim_core::routing::{Graph, HpaIndex, RoutingProfile}`, `HashMap`/`HashSet`/`BinaryHeap`, serde JSONB payloads, protobuf/prost backend, TypeScript/Vitest frontend.

---

## File Structure

### Create

- `backend/crates/sim-core/src/routing/flow_field.rs` - flow-field model, reverse Dijkstra builder, corridor scope, materialization, bounded cache, tests.

### Modify

- `backend/crates/sim-core/src/routing/mod.rs` - add module and public re-exports.
- `backend/crates/sim-core/src/routing/plugin.rs` - add `FlowFieldPlugin` and plugin tests.
- `backend/crates/sim-core/src/mobility/components.rs` - add `ActiveRoute` and `RouteStep`.
- `backend/crates/sim-core/src/mobility/records.rs` - add persisted route execution records and `AgentRecord.active_route`.
- `backend/crates/sim-core/src/mobility/api.rs` - spawn/extract active routes, expose canonical edge resolution helpers, keep `AgentRecord` round-trip complete.
- `backend/crates/sim-core/src/mobility/systems.rs` - add `route_assignment_system` and `route_advance_system`, order them before `walk_advance_system`.
- `backend/crates/sim-core/src/mobility/persist_snapshot.rs` - validate/hydrate persisted active route data.
- `backend/crates/sim-server/src/runtime.rs` - install `FlowFieldPlugin`; clear flow-field cache whenever routing graph/HPA resources refresh.
- `src/backend/mobilityProtocol.ts` - reject missing proto state/world coordinates in conversion helpers.
- `tests/backend/mobilityProtocol.test.ts` - cover strict proto decode behavior and graph edge ids.
- `progress.md` - add final verification entry after implementation passes.

### Do Not Modify

- `backend/crates/protocol/proto/abutown.proto` unless the implementation proves a compile-time protobuf shape change is required. The approved design keeps `Walking { link_id, progress }` on the wire.
- `src/render/**` unless tests expose a rendering regression from stricter DTO validation.
- Database migrations. 8e evolves the JSONB payload shape through serde defaults, not a table migration.

---

## Task 1: Flow Field Engine

**Files:**
- Create: `backend/crates/sim-core/src/routing/flow_field.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [x] **Step 1: Add module export and failing flow-field tests**

In `backend/crates/sim-core/src/routing/mod.rs`, add:

```rust
pub mod flow_field;
```

Add these re-exports near the other `pub use` blocks:

```rust
pub use flow_field::{
    FlowField, FlowFieldCache, FlowFieldCacheKey, FlowFieldCacheStats, FlowFieldError,
    FlowFieldEntry, FlowFieldRouter, FlowFieldScope,
};
```

Create `backend/crates/sim-core/src/routing/flow_field.rs` with the test module and minimal type imports:

```rust
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bevy_ecs::prelude::*;

use crate::routing::{
    ClusterId, EdgeId, EdgeKind, Graph, ModeState, NodeId, RoutingProfile, RoutingProfileKey,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{Edge, Node, NodeKind};

    fn node(id: u32, x: f32, y: f32) -> Node {
        Node {
            id: NodeId(id),
            position: (x, y),
            kind: NodeKind::Intersection,
            legacy_id: None,
        }
    }

    fn edge(id: u32, from: u32, to: u32, kind: EdgeKind, legacy: &str) -> Edge {
        Edge {
            id: EdgeId(id),
            from: NodeId(from),
            to: NodeId(to),
            polyline: vec![(from as f32, 0.0), (to as f32, 0.0)],
            length: 1.0,
            kind,
            speed_limit: 1.0,
            capacity: 1,
            legacy_id: Some(legacy.to_string()),
        }
    }

    fn walk_graph() -> Graph {
        Graph::new(
            vec![node(0, 0.0, 0.0), node(1, 1.0, 0.0), node(2, 2.0, 0.0)],
            vec![
                edge(0, 0, 1, EdgeKind::Footway, "walk:0"),
                edge(1, 1, 2, EdgeKind::Footway, "walk:1"),
                edge(2, 0, 2, EdgeKind::Road, "road:shortcut"),
            ],
        )
    }

    #[test]
    fn field_points_multiple_origins_to_destination() {
        let graph = walk_graph();
        let field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::AllEdges,
        )
        .expect("walk field should build");

        assert_eq!(
            field.entry(NodeId(0), ModeState::Walking).unwrap().next_edge,
            Some(EdgeId(0))
        );
        assert_eq!(
            field.entry(NodeId(1), ModeState::Walking).unwrap().next_edge,
            Some(EdgeId(1))
        );
        assert_eq!(
            field.entry(NodeId(2), ModeState::Walking).unwrap().next_edge,
            None
        );
    }

    #[test]
    fn field_respects_profile_edge_legality() {
        let graph = walk_graph();
        let walk_field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::AllEdges,
        )
        .expect("walk field should build");
        assert_ne!(
            walk_field.entry(NodeId(0), ModeState::Walking).unwrap().next_edge,
            Some(EdgeId(2))
        );

        let car_field = FlowFieldRouter::build(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Car),
            FlowFieldScope::AllEdges,
        )
        .expect("car field should build over road edge");
        assert_eq!(
            car_field.entry(NodeId(0), ModeState::Driving).unwrap().next_edge,
            Some(EdgeId(2))
        );
    }

    #[test]
    fn corridor_scope_rejects_edges_outside_cluster_set() {
        let graph = walk_graph();
        let mut clusters = HashSet::new();
        clusters.insert(ClusterId(0));
        let result = FlowFieldRouter::build_with_cluster_lookup(
            &graph,
            NodeId(2),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::Corridor(clusters),
            |node| match node.0 {
                0 | 1 => Some(ClusterId(0)),
                2 => Some(ClusterId(1)),
                _ => None,
            },
        );

        assert_eq!(
            result,
            Err(FlowFieldError::Unreachable {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            })
        );
    }
}
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::flow_field -- --nocapture
```

Expected: compile fails because `FlowFieldRouter`, `FlowFieldScope`, `FlowFieldError`, and `FlowField` are not implemented.

- [x] **Step 3: Implement flow-field types and reverse Dijkstra**

In `backend/crates/sim-core/src/routing/flow_field.rs`, above the test module, add:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum FlowFieldError {
    MissingNode(NodeId),
    MissingCluster(NodeId),
    NoCorridor {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    Unreachable {
        from: NodeId,
        to: NodeId,
        profile: RoutingProfileKey,
    },
    InvalidGraph(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowFieldEntry {
    pub next_edge: Option<EdgeId>,
    pub next_mode: ModeState,
    pub cost_to_goal: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowField {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    entries: HashMap<(NodeId, ModeState), FlowFieldEntry>,
}

impl FlowField {
    pub fn entry(&self, node: NodeId, mode: ModeState) -> Option<&FlowFieldEntry> {
        self.entries.get(&(node, mode))
    }

    pub fn reachable_state_count(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowFieldScope {
    AllEdges,
    Corridor(HashSet<ClusterId>),
}

#[derive(Debug, Clone, Copy)]
struct QueueEntry {
    node: NodeId,
    mode: ModeState,
    cost: f32,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.node == other.node && self.mode == other.mode
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.node.0.cmp(&self.node.0))
            .then_with(|| other.mode.cmp(&self.mode))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct FlowFieldRouter;

impl FlowFieldRouter {
    pub fn build(
        graph: &Graph,
        destination: NodeId,
        profile: RoutingProfile,
        scope: FlowFieldScope,
    ) -> Result<FlowField, FlowFieldError> {
        Self::build_with_cluster_lookup(graph, destination, profile, scope, |_| None)
    }

    pub fn build_with_cluster_lookup<F>(
        graph: &Graph,
        destination: NodeId,
        profile: RoutingProfile,
        scope: FlowFieldScope,
        cluster_of_node: F,
    ) -> Result<FlowField, FlowFieldError>
    where
        F: Fn(NodeId) -> Option<ClusterId>,
    {
        validate_node(graph, destination)?;

        let destination_mode = profile.initial_mode();
        let mut open = BinaryHeap::new();
        let mut entries: HashMap<(NodeId, ModeState), FlowFieldEntry> = HashMap::new();

        entries.insert(
            (destination, destination_mode),
            FlowFieldEntry {
                next_edge: None,
                next_mode: destination_mode,
                cost_to_goal: 0.0,
            },
        );
        open.push(QueueEntry {
            node: destination,
            mode: destination_mode,
            cost: 0.0,
        });

        while let Some(entry) = open.pop() {
            let best = entries
                .get(&(entry.node, entry.mode))
                .map(|e| e.cost_to_goal)
                .unwrap_or(f32::INFINITY);
            if entry.cost > best {
                continue;
            }

            for edge_id in graph.incoming(entry.node) {
                let edge = graph.edge(*edge_id);
                if !scope_allows_edge(&scope, edge.from, edge.to, &cluster_of_node)? {
                    continue;
                }
                let from_node = graph.node(edge.from);
                for prior_mode in [ModeState::Walking, ModeState::Driving, ModeState::OnTram] {
                    let Some((next_mode, edge_cost)) =
                        profile.transition(prior_mode, from_node.kind, edge)
                    else {
                        continue;
                    };
                    if next_mode != entry.mode {
                        continue;
                    }
                    if edge_cost < 0.0 || !edge_cost.is_finite() {
                        return Err(FlowFieldError::InvalidGraph(
                            "edge cost must be finite and non-negative",
                        ));
                    }
                    let next_cost = entry.cost + edge_cost;
                    let key = (edge.from, prior_mode);
                    let old_cost = entries
                        .get(&key)
                        .map(|existing| existing.cost_to_goal)
                        .unwrap_or(f32::INFINITY);
                    if next_cost < old_cost {
                        entries.insert(
                            key,
                            FlowFieldEntry {
                                next_edge: Some(*edge_id),
                                next_mode,
                                cost_to_goal: next_cost,
                            },
                        );
                        open.push(QueueEntry {
                            node: edge.from,
                            mode: prior_mode,
                            cost: next_cost,
                        });
                    }
                }
            }
        }

        Ok(FlowField {
            destination,
            profile: profile.key,
            entries,
        })
    }

    pub fn require_reachable(
        field: &FlowField,
        from: NodeId,
        to: NodeId,
        mode: ModeState,
    ) -> Result<(), FlowFieldError> {
        if field.entry(from, mode).is_some() {
            Ok(())
        } else {
            Err(FlowFieldError::Unreachable {
                from,
                to,
                profile: field.profile,
            })
        }
    }
}

fn validate_node(graph: &Graph, node: NodeId) -> Result<(), FlowFieldError> {
    if (node.0 as usize) < graph.node_count() {
        Ok(())
    } else {
        Err(FlowFieldError::MissingNode(node))
    }
}

fn scope_allows_edge<F>(
    scope: &FlowFieldScope,
    from: NodeId,
    to: NodeId,
    cluster_of_node: &F,
) -> Result<bool, FlowFieldError>
where
    F: Fn(NodeId) -> Option<ClusterId>,
{
    match scope {
        FlowFieldScope::AllEdges => Ok(true),
        FlowFieldScope::Corridor(clusters) => {
            let Some(from_cluster) = cluster_of_node(from) else {
                return Err(FlowFieldError::MissingCluster(from));
            };
            let Some(to_cluster) = cluster_of_node(to) else {
                return Err(FlowFieldError::MissingCluster(to));
            };
            Ok(clusters.contains(&from_cluster) && clusters.contains(&to_cluster))
        }
    }
}
```

- [x] **Step 4: Adjust the corridor test to assert explicit reachability**

Replace the final assertion in `corridor_scope_rejects_edges_outside_cluster_set` with:

```rust
        let field = result.expect("field can build even when origin is unreachable");
        assert_eq!(
            FlowFieldRouter::require_reachable(
                &field,
                NodeId(0),
                NodeId(2),
                ModeState::Walking,
            ),
            Err(FlowFieldError::Unreachable {
                from: NodeId(0),
                to: NodeId(2),
                profile: RoutingProfileKey::Walk,
            })
        );
```

- [x] **Step 5: Run targeted tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::flow_field -- --nocapture
```

Expected: all `routing::flow_field` tests pass.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/flow_field.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8e): add graph flow fields"
```

---

## Task 2: Flow Field Cache and Plugin

**Files:**
- Modify: `backend/crates/sim-core/src/routing/flow_field.rs`
- Modify: `backend/crates/sim-core/src/routing/plugin.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`

- [x] **Step 1: Add failing cache tests**

Append to `routing/flow_field.rs` tests:

```rust
    #[test]
    fn cache_tracks_miss_hit_insert_and_eviction() {
        let graph = walk_graph();
        let mut cache = FlowFieldCache::with_capacity(1);
        let key = FlowFieldCacheKey::new(NodeId(2), RoutingProfileKey::Walk, 0, &[ClusterId(0)]);

        let first = cache
            .get_or_build(
                &graph,
                key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("first field should build");
        let second = cache
            .get_or_build(
                &graph,
                key,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("second field should hit");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            cache.stats(),
            FlowFieldCacheStats {
                hits: 1,
                misses: 1,
                inserts: 1,
                evictions: 0,
            }
        );

        let other = FlowFieldCacheKey::new(NodeId(1), RoutingProfileKey::Walk, 0, &[ClusterId(1)]);
        let _ = cache
            .get_or_build(
                &graph,
                other,
                RoutingProfile::for_key(RoutingProfileKey::Walk),
                FlowFieldScope::AllEdges,
            )
            .expect("other field should build");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().evictions, 1);
    }
```

- [x] **Step 2: Run test to verify it fails**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::flow_field::tests::cache_tracks_miss_hit_insert_and_eviction -- --nocapture
```

Expected: compile fails because cache types and methods are missing.

- [x] **Step 3: Implement cache types**

Add to `routing/flow_field.rs` above tests:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowFieldCacheKey {
    pub destination: NodeId,
    pub profile: RoutingProfileKey,
    pub graph_generation: u64,
    pub corridor_hash: u64,
}

impl FlowFieldCacheKey {
    pub fn new(
        destination: NodeId,
        profile: RoutingProfileKey,
        graph_generation: u64,
        corridor: &[ClusterId],
    ) -> Self {
        let mut sorted = corridor.to_vec();
        sorted.sort_unstable();
        let mut hash = 1469598103934665603_u64;
        for cluster in sorted {
            hash ^= u64::from(cluster.0);
            hash = hash.wrapping_mul(1099511628211);
        }
        Self {
            destination,
            profile,
            graph_generation,
            corridor_hash: hash,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlowFieldCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}

#[derive(Resource)]
pub struct FlowFieldCache {
    capacity: usize,
    entries: HashMap<FlowFieldCacheKey, Arc<FlowField>>,
    order: VecDeque<FlowFieldCacheKey>,
    stats: FlowFieldCacheStats,
}

impl Default for FlowFieldCache {
    fn default() -> Self {
        Self::with_capacity(4096)
    }
}

impl FlowFieldCache {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
            stats: FlowFieldCacheStats::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn stats(&self) -> FlowFieldCacheStats {
        self.stats
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub fn insert(&mut self, key: FlowFieldCacheKey, field: Arc<FlowField>) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key);
        }
        self.entries.insert(key, field);
        self.stats.inserts += 1;
        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front()
                && self.entries.remove(&oldest).is_some()
            {
                self.stats.evictions += 1;
            }
        }
    }

    pub fn get_or_build(
        &mut self,
        graph: &Graph,
        key: FlowFieldCacheKey,
        profile: RoutingProfile,
        scope: FlowFieldScope,
    ) -> Result<Arc<FlowField>, FlowFieldError> {
        debug_assert_eq!(key.destination, key.destination);
        debug_assert_eq!(key.profile, profile.key);
        if let Some(existing) = self.entries.get(&key) {
            self.stats.hits += 1;
            return Ok(Arc::clone(existing));
        }
        self.stats.misses += 1;
        let field = Arc::new(FlowFieldRouter::build(graph, key.destination, profile, scope)?);
        self.insert(key, Arc::clone(&field));
        Ok(field)
    }
}
```

- [x] **Step 4: Add plugin failing tests**

In `backend/crates/sim-core/src/routing/plugin.rs`, add `FlowFieldCache` to imports:

```rust
use crate::routing::flow_field::FlowFieldCache;
```

Add plugin type:

```rust
pub struct FlowFieldPlugin {
    pub cache_capacity: usize,
}

impl Default for FlowFieldPlugin {
    fn default() -> Self {
        Self {
            cache_capacity: 4096,
        }
    }
}

impl SimPlugin for FlowFieldPlugin {
    fn name(&self) -> &'static str {
        "flow_field"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        world.insert_resource(FlowFieldCache::with_capacity(self.cache_capacity));
    }
}
```

Add a test:

```rust
    #[test]
    fn flow_field_plugin_installs_cache() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        FlowFieldPlugin::default().install(&mut world, &mut schedule);

        assert!(world.contains_resource::<crate::routing::FlowFieldCache>());
        assert_eq!(world.resource::<crate::routing::FlowFieldCache>().len(), 0);
    }
```

- [x] **Step 5: Re-export plugin**

In `routing/mod.rs`, extend the plugin re-export:

```rust
pub use plugin::{FlowFieldPlugin, HierarchicalRoutingPlugin, PathfindingPlugin, RoutingPlugin};
```

- [x] **Step 6: Run targeted tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::flow_field routing::plugin -- --nocapture
```

Expected: flow-field and routing plugin tests pass.

- [x] **Step 7: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/routing/flow_field.rs backend/crates/sim-core/src/routing/plugin.rs backend/crates/sim-core/src/routing/mod.rs
git commit -m "feat(8e): add flow field cache plugin"
```

---

## Task 3: Persistable Active Route Types

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/components.rs`
- Modify: `backend/crates/sim-core/src/mobility/records.rs`
- Modify: `backend/crates/sim-core/src/mobility/api.rs`
- Modify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs`

- [x] **Step 1: Add failing record round-trip test**

In `backend/crates/sim-core/src/mobility/records.rs`, append:

```rust
#[cfg(test)]
mod route_execution_tests {
    use super::*;
    use crate::routing::{ModeState, RoutingProfileKey};

    #[test]
    fn agent_record_round_trips_active_route() {
        let record = AgentRecord {
            id: AgentId("agent:route".into()),
            state: AgentMobilityState::Walking {
                link_id: "edge:7".into(),
                progress: 0.25,
            },
            plan: vec![PlanStage::Activity {
                activity_id: "activity:work".into(),
            }],
            plan_cursor: 0,
            walk_speed_per_tick: 0.1,
            active_route: Some(PersistedActiveRoute {
                destination_node: 3,
                profile: RoutingProfileKey::Walk,
                cursor: 1,
                steps: vec![PersistedRouteStep {
                    edge_id: 7,
                    mode: ModeState::Walking,
                    canonical_edge_key: "edge:7".into(),
                    length: 12.0,
                }],
            }),
        };

        let encoded = serde_json::to_string(&record).expect("record should serialize");
        let decoded: AgentRecord = serde_json::from_str(&encoded).expect("record should deserialize");
        assert_eq!(decoded, record);
    }

    #[test]
    fn legacy_agent_record_defaults_active_route_to_none() {
        let encoded = r#"{
            "id":"agent:legacy",
            "state":{"Walking":{"link_id":"link:walk:default","progress":0.0}},
            "plan":[],
            "plan_cursor":0,
            "walk_speed_per_tick":0.1
        }"#;
        let decoded: AgentRecord = serde_json::from_str(encoded).expect("legacy shape should load");
        assert!(decoded.active_route.is_none());
    }
}
```

- [x] **Step 2: Run test to verify it fails**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core mobility::records::route_execution_tests -- --nocapture
```

Expected: compile fails because persisted route types and `AgentRecord.active_route` are missing.

- [x] **Step 3: Add persisted and ECS route types**

In `mobility/records.rs`, import routing types:

```rust
use crate::routing::{ModeState, RoutingProfileKey};
```

Add before `AgentRecord`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedActiveRoute {
    pub destination_node: u32,
    pub profile: RoutingProfileKey,
    pub cursor: usize,
    pub steps: Vec<PersistedRouteStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedRouteStep {
    pub edge_id: u32,
    pub mode: ModeState,
    pub canonical_edge_key: String,
    pub length: f32,
}
```

Update `AgentRecord`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: AgentId,
    pub state: AgentMobilityState,
    pub plan: Vec<PlanStage>,
    pub plan_cursor: usize,
    pub walk_speed_per_tick: f32,
    #[serde(default)]
    pub active_route: Option<PersistedActiveRoute>,
}
```

Update `AgentRecord::new` return value:

```rust
            active_route: None,
```

In `mobility/components.rs`, add:

```rust
/// Current multi-edge graph route for an agent walking a graph-backed leg.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct ActiveRoute {
    pub destination: crate::routing::NodeId,
    pub profile: crate::routing::RoutingProfileKey,
    pub steps: Vec<RouteStep>,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteStep {
    pub edge_id: crate::routing::EdgeId,
    pub mode: crate::routing::ModeState,
    pub canonical_edge_key: String,
    pub length: f32,
}
```

- [x] **Step 4: Wire spawn/extract through API**

In `mobility/api.rs`, update `spawn_agent_from_record` so it inserts `ActiveRoute` when present:

```rust
    let active_route = record.active_route.clone().map(|route| ActiveRoute {
        destination: crate::routing::NodeId(route.destination_node),
        profile: route.profile,
        steps: route
            .steps
            .into_iter()
            .map(|step| RouteStep {
                edge_id: crate::routing::EdgeId(step.edge_id),
                mode: step.mode,
                canonical_edge_key: step.canonical_edge_key,
                length: step.length,
            })
            .collect(),
        cursor: route.cursor,
    });
```

Replace the direct spawn tuple with a mutable entity builder:

```rust
    let mut entity = world.spawn((
        AgentMarker,
        StableAgentId(record.id),
        AgentMobilityStateComponent(record.state),
        WalkPlan {
            stages: record.plan,
            cursor: record.plan_cursor,
        },
        WalkSpeed(record.walk_speed_per_tick),
        Position { x: px, y: py },
        Direction(abutown_protocol::DirectionDto::S),
        SpriteKey(sprite_key),
    ));
    if let Some(active_route) = active_route {
        entity.insert(active_route);
    }
    let entity = entity.id();
```

In `agent_record_from_entity`, read `ActiveRoute`:

```rust
    let active_route = world.get::<ActiveRoute>(entity).map(|route| PersistedActiveRoute {
        destination_node: route.destination.0,
        profile: route.profile,
        cursor: route.cursor,
        steps: route
            .steps
            .iter()
            .map(|step| PersistedRouteStep {
                edge_id: step.edge_id.0,
                mode: step.mode,
                canonical_edge_key: step.canonical_edge_key.clone(),
                length: step.length,
            })
            .collect(),
    });
```

Return:

```rust
    Some(AgentRecord {
        id: stable.0.clone(),
        state: state.0.clone(),
        plan: plan.stages.clone(),
        plan_cursor: plan.cursor,
        walk_speed_per_tick: speed.0,
        active_route,
    })
```

- [x] **Step 5: Add persisted route validation**

In `mobility/persist_snapshot.rs`, add:

```rust
fn validate_active_route(graph: &Graph, route: &crate::mobility::records::PersistedActiveRoute) {
    if (route.destination_node as usize) >= graph.node_count() {
        panic!(
            "apply_into_world: persisted active_route destination node {} is missing",
            route.destination_node
        );
    }
    if route.cursor > route.steps.len() {
        panic!(
            "apply_into_world: persisted active_route cursor {} exceeds {} steps",
            route.cursor,
            route.steps.len()
        );
    }
    for step in &route.steps {
        if (step.edge_id as usize) >= graph.edge_count() {
            panic!(
                "apply_into_world: persisted active_route edge {} is missing",
                step.edge_id
            );
        }
        let edge = graph.edge(EdgeId(step.edge_id));
        let canonical = edge
            .legacy_id
            .clone()
            .unwrap_or_else(|| format!("edge:{}", edge.id.0));
        if canonical != step.canonical_edge_key {
            panic!(
                "apply_into_world: persisted active_route edge {} key mismatch: got {}, expected {}",
                step.edge_id, step.canonical_edge_key, canonical
            );
        }
        if step.length < 0.0 || !step.length.is_finite() {
            panic!(
                "apply_into_world: persisted active_route edge {} has invalid length {}",
                step.edge_id, step.length
            );
        }
    }
}
```

In `apply_into_world`, before spawning agents:

```rust
    {
        let graph = world.resource::<Graph>();
        for agent in snap.agents.values() {
            if let Some(active_route) = &agent.active_route {
                validate_active_route(graph, active_route);
            }
        }
    }
```

- [x] **Step 6: Run targeted tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core mobility::records::route_execution_tests -- --nocapture
cargo test -p sim-core mobility:: -- --nocapture
```

Expected: mobility tests pass.

- [x] **Step 7: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/mobility/components.rs backend/crates/sim-core/src/mobility/records.rs backend/crates/sim-core/src/mobility/api.rs backend/crates/sim-core/src/mobility/persist_snapshot.rs
git commit -m "feat(8e): persist active walking routes"
```

---

## Task 4: Route Assignment and Route Advancement

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/resources.rs`
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`
- Modify: `backend/crates/sim-core/src/mobility/api.rs`

- [x] **Step 1: Add route-assignment stats resource**

In `mobility/resources.rs`, add:

```rust
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RouteAssignmentStats {
    pub assigned: u64,
    pub skipped: u64,
    pub failed: u64,
}
```

In `mobility/api.rs::install_mobility`, insert:

```rust
    world.insert_resource(RouteAssignmentStats::default());
```

- [x] **Step 2: Add failing system tests**

Append to `mobility/systems.rs` tests module if one exists. If the file has no tests module, add this at the bottom:

```rust
#[cfg(test)]
mod route_execution_tests {
    use super::*;
    use crate::ids::AgentId;
    use crate::mobility::api;
    use crate::mobility::records::{AgentMobilityState, AgentRecord, PlanStage};
    use crate::routing::{Edge, EdgeId, EdgeKind, Graph, HpaConfig, HpaIndex, Node, NodeId, NodeKind};
    use abutown_protocol::WorldId;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::world::World;

    fn graph() -> Graph {
        Graph::new(
            vec![
                Node {
                    id: NodeId(0),
                    position: (0.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: Some("activity:home".into()),
                },
                Node {
                    id: NodeId(1),
                    position: (10.0, 0.0),
                    kind: NodeKind::Intersection,
                    legacy_id: None,
                },
                Node {
                    id: NodeId(2),
                    position: (20.0, 0.0),
                    kind: NodeKind::ActivityLocation,
                    legacy_id: Some("activity:work".into()),
                },
            ],
            vec![
                Edge {
                    id: EdgeId(0),
                    from: NodeId(0),
                    to: NodeId(1),
                    polyline: vec![(0.0, 0.0), (10.0, 0.0)],
                    length: 10.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: Some("walk:a".into()),
                },
                Edge {
                    id: EdgeId(1),
                    from: NodeId(1),
                    to: NodeId(2),
                    polyline: vec![(10.0, 0.0), (20.0, 0.0)],
                    length: 10.0,
                    kind: EdgeKind::Footway,
                    speed_limit: 1.0,
                    capacity: 1,
                    legacy_id: Some("walk:b".into()),
                },
            ],
        )
    }

    fn world_with_route_agent() -> (World, Schedule) {
        let (mut world, schedule) = api::empty_world_and_schedule();
        let graph = graph();
        let hpa = HpaIndex::build(
            &graph,
            HpaConfig {
                cluster_size_tiles: 32,
                corridor_margin_clusters: 0,
            },
        )
        .expect("test hpa should build");
        world.insert_resource(graph);
        world.insert_resource(hpa);
        world.insert_resource(crate::routing::FlowFieldCache::with_capacity(8));
        api::force_all_chunks_active_for_test(&mut world);
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId("agent:route".into()),
                AgentMobilityState::Walking {
                    link_id: "walk:a".into(),
                    progress: 0.0,
                },
                vec![PlanStage::WalkToActivity {
                    link_id: "walk:a".into(),
                    activity_id: "activity:work".into(),
                }],
                1.0,
            ),
        );
        (world, schedule)
    }

    #[test]
    fn route_assignment_inserts_active_route() {
        let (mut world, mut schedule) = world_with_route_agent();
        schedule.run(&mut world);

        let record = api::agent(&world, &AgentId("agent:route".into())).unwrap();
        assert!(record.active_route.is_some());
        assert_eq!(record.active_route.unwrap().steps.len(), 2);
        assert_eq!(world.resource::<RouteAssignmentStats>().assigned, 1);
    }

    #[test]
    fn route_advance_crosses_edges_before_finishing_plan() {
        let (mut world, mut schedule) = world_with_route_agent();
        schedule.run(&mut world);
        schedule.run(&mut world);

        let record = api::agent(&world, &AgentId("agent:route".into())).unwrap();
        assert_eq!(record.plan_cursor, 0);
        assert_eq!(
            record.state,
            AgentMobilityState::Walking {
                link_id: "walk:b".into(),
                progress: 0.0,
            }
        );
        assert_eq!(record.active_route.unwrap().cursor, 1);
    }
}
```

- [x] **Step 3: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core mobility::systems::route_execution_tests -- --nocapture
```

Expected: compile fails because route assignment/advance systems are not registered and helpers are missing.

- [x] **Step 4: Add canonical edge helpers**

In `mobility/api.rs`, add:

```rust
pub fn canonical_edge_key(graph: &crate::routing::Graph, edge_id: crate::routing::EdgeId) -> String {
    let edge = graph.edge(edge_id);
    edge.legacy_id
        .clone()
        .unwrap_or_else(|| format!("edge:{}", edge.id.0))
}

pub fn edge_by_canonical_key(
    graph: &crate::routing::Graph,
    key: &str,
) -> Option<crate::routing::EdgeId> {
    if let Some(edge_id) = graph.edge_by_legacy(key) {
        return Some(edge_id);
    }
    let raw = key.strip_prefix("edge:")?;
    let id = raw.parse::<u32>().ok()?;
    ((id as usize) < graph.edge_count()).then_some(crate::routing::EdgeId(id))
}
```

- [x] **Step 5: Add route assignment materialization**

In `mobility/systems.rs`, add imports:

```rust
use crate::routing::{
    FlowFieldCache, FlowFieldCacheKey, FlowFieldRouter, FlowFieldScope, HpaIndex, ModeState,
    RoutingProfile, RoutingProfileKey,
};
```

Add helper functions:

```rust
fn current_route_origin(
    graph: &crate::routing::Graph,
    link_id: &str,
    progress: f32,
) -> Option<crate::routing::NodeId> {
    let edge_id = crate::mobility::api::edge_by_canonical_key(graph, link_id)?;
    let edge = graph.edge(edge_id);
    if progress >= 1.0 {
        Some(edge.to)
    } else {
        Some(edge.from)
    }
}

fn destination_for_stage(
    graph: &crate::routing::Graph,
    spatial: Option<&crate::routing::NodeSpatialIndex>,
    stage: &PlanStage,
) -> Option<crate::routing::NodeId> {
    match stage {
        PlanStage::WalkToStop { stop_id, .. } => graph.node_by_legacy(stop_id),
        PlanStage::WalkToActivity { activity_id, .. } => graph
            .node_by_legacy(activity_id)
            .or_else(|| {
                let coord = crate::mobility_geometry::activity_geometry(activity_id)?.coord;
                spatial.and_then(|index| index.nearest(coord))
            }),
        _ => None,
    }
}

fn materialize_route_steps(
    graph: &crate::routing::Graph,
    field: &crate::routing::FlowField,
    from: crate::routing::NodeId,
    destination: crate::routing::NodeId,
    initial_mode: ModeState,
) -> Option<Vec<RouteStep>> {
    let mut node = from;
    let mut mode = initial_mode;
    let mut steps = Vec::new();
    let mut guard = 0usize;
    while node != destination {
        guard += 1;
        if guard > graph.edge_count().max(1) {
            return None;
        }
        let entry = field.entry(node, mode)?;
        let edge_id = entry.next_edge?;
        let edge = graph.edge(edge_id);
        steps.push(RouteStep {
            edge_id,
            mode: entry.next_mode,
            canonical_edge_key: crate::mobility::api::canonical_edge_key(graph, edge_id),
            length: edge.length,
        });
        node = edge.to;
        mode = entry.next_mode;
    }
    Some(steps)
}
```

- [x] **Step 6: Implement `route_assignment_system`**

Add:

```rust
#[allow(clippy::type_complexity)]
pub fn route_assignment_system(
    mut agents: Query<
        (
            Entity,
            &AgentMobilityStateComponent,
            &WalkPlan,
            Option<&ActiveRoute>,
        ),
        With<AgentMarker>,
    >,
    graph: Res<crate::routing::Graph>,
    hpa: Option<Res<HpaIndex>>,
    spatial: Option<Res<crate::routing::NodeSpatialIndex>>,
    mut cache: Option<ResMut<FlowFieldCache>>,
    mut stats: ResMut<RouteAssignmentStats>,
    mut commands: Commands,
) {
    let Some(hpa) = hpa else {
        return;
    };
    let Some(mut cache) = cache else {
        return;
    };
    for (entity, state, plan, active_route) in agents.iter_mut() {
        if active_route.is_some() {
            stats.skipped += 1;
            continue;
        }
        let AgentMobilityState::Walking { link_id, progress } = &state.0 else {
            stats.skipped += 1;
            continue;
        };
        let Some(stage) = plan.stages.get(plan.cursor) else {
            stats.skipped += 1;
            continue;
        };
        let Some(origin) = current_route_origin(&graph, link_id, *progress) else {
            stats.failed += 1;
            continue;
        };
        let Some(destination) = destination_for_stage(&graph, spatial.as_deref(), stage) else {
            stats.failed += 1;
            continue;
        };
        if origin == destination {
            stats.skipped += 1;
            continue;
        }
        let Some(origin_cluster) = hpa.cluster_of_node(origin) else {
            stats.failed += 1;
            continue;
        };
        let Some(destination_cluster) = hpa.cluster_of_node(destination) else {
            stats.failed += 1;
            continue;
        };
        let corridor_clusters = if origin_cluster == destination_cluster {
            vec![origin_cluster]
        } else {
            vec![origin_cluster, destination_cluster]
        };
        let corridor_set = corridor_clusters.iter().copied().collect();
        let key = FlowFieldCacheKey::new(destination, RoutingProfileKey::Walk, 0, &corridor_clusters);
        let field = match cache.get_or_build(
            &graph,
            key,
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::Corridor(corridor_set),
        ) {
            Ok(field) => field,
            Err(_) => {
                stats.failed += 1;
                continue;
            }
        };
        let Some(steps) =
            materialize_route_steps(&graph, &field, origin, destination, ModeState::Walking)
        else {
            stats.failed += 1;
            continue;
        };
        if steps.is_empty() {
            stats.skipped += 1;
            continue;
        }
        commands.entity(entity).insert(ActiveRoute {
            destination,
            profile: RoutingProfileKey::Walk,
            steps,
            cursor: 0,
        });
        stats.assigned += 1;
    }
}
```

Note: the first implementation uses the same-cluster corridor and direct start/destination clusters. If a test needs a longer corridor, use a small graph where both nodes share one cluster. Do not add a global-routing retry.

- [x] **Step 7: Implement `route_advance_system`**

Add:

```rust
pub fn route_advance_system(
    mut agents: Query<
        (
            Entity,
            &mut AgentMobilityStateComponent,
            &mut WalkPlan,
            &mut ActiveRoute,
        ),
        With<AgentMarker>,
    >,
    mut dirty: ResMut<DirtyAgents>,
    mut commands: Commands,
) {
    for (entity, mut state, mut plan, mut route) in agents.iter_mut() {
        let AgentMobilityState::Walking { progress, .. } = &state.0 else {
            continue;
        };
        if *progress < 1.0 {
            continue;
        }

        let next_cursor = route.cursor + 1;
        if let Some(next_step) = route.steps.get(next_cursor).cloned() {
            route.cursor = next_cursor;
            state.0 = AgentMobilityState::Walking {
                link_id: next_step.canonical_edge_key,
                progress: 0.0,
            };
            dirty.0.insert(entity);
            continue;
        }

        let completed_stage = plan.stages.get(plan.cursor).cloned();
        match completed_stage {
            Some(PlanStage::WalkToStop { stop_id, .. }) => {
                plan.cursor += 1;
                state.0 = AgentMobilityState::WaitingAtStop { stop_id };
                commands.entity(entity).remove::<ActiveRoute>();
                dirty.0.insert(entity);
            }
            Some(PlanStage::WalkToActivity { activity_id, .. }) => {
                plan.cursor += 1;
                state.0 = AgentMobilityState::AtActivity { activity_id };
                commands.entity(entity).remove::<ActiveRoute>();
                dirty.0.insert(entity);
            }
            _ => {}
        }
    }
}
```

- [x] **Step 8: Register systems in order**

In `install_systems`, update the Advance set tuple so route systems run before `walk_advance_system`:

```rust
        route_assignment_system.in_set(MobilitySet::Advance),
        route_advance_system
            .in_set(MobilitySet::Advance)
            .after(route_assignment_system),
        update_link_polyline_cache_system
            .in_set(MobilitySet::Advance)
            .after(route_advance_system),
        walk_advance_system
            .in_set(MobilitySet::Advance)
            .after(update_link_polyline_cache_system),
```

Keep the existing downstream `.after(walk_advance_system)` ordering for boarding, stop arrival, and vehicle advancement.

- [x] **Step 9: Run targeted mobility tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core mobility::systems::route_execution_tests -- --nocapture
cargo test -p sim-core mobility:: -- --nocapture
```

Expected: route execution tests pass and existing mobility tests remain green.

- [x] **Step 10: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-core/src/mobility/resources.rs backend/crates/sim-core/src/mobility/systems.rs backend/crates/sim-core/src/mobility/api.rs
git commit -m "feat(8e): execute walking graph routes"
```

---

## Task 5: Runtime Flow Field Integration

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-core/src/routing/plugin.rs`

- [x] **Step 1: Add failing runtime tests**

In `backend/crates/sim-server/src/runtime.rs`, find the existing runtime tests for HPA/pathfinding resources and add:

```rust
    #[test]
    fn runtime_installs_flow_field_cache() {
        let runtime = SimulationRuntime::new_for_test();
        assert!(runtime.world.contains_resource::<sim_core::routing::FlowFieldCache>());
    }

    #[test]
    fn set_mobility_for_test_refreshes_flow_field_cache() {
        let mut runtime = SimulationRuntime::new_for_test();
        {
            let mut cache = runtime
                .world
                .resource_mut::<sim_core::routing::FlowFieldCache>();
            assert_eq!(cache.len(), 0);
        }

        let (world, schedule) = sim_core::mobility::seed::tiny_world();
        runtime.set_mobility_for_test(world, schedule);

        assert!(runtime.world.contains_resource::<sim_core::routing::HpaIndex>());
        assert!(runtime.world.contains_resource::<sim_core::routing::FlowFieldCache>());
        assert_eq!(
            runtime
                .world
                .resource::<sim_core::routing::FlowFieldCache>()
                .len(),
            0
        );
    }
```

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-server runtime_installs_flow_field_cache set_mobility_for_test_refreshes_flow_field_cache -- --nocapture
```

Expected: compile or assertion failure because runtime does not install/refresh `FlowFieldCache`.

- [x] **Step 3: Install FlowFieldPlugin in runtime construction**

In `runtime.rs`, wherever plugins are installed in runtime setup, ensure order is:

```rust
sim_core::routing::RoutingPlugin {
    seeded_stops,
    seeded_walks,
}
.install(&mut world, &mut schedule);
sim_core::routing::PathfindingPlugin::default().install(&mut world, &mut schedule);
sim_core::routing::HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);
sim_core::routing::FlowFieldPlugin::default().install(&mut world, &mut schedule);
sim_core::mobility::MobilityPlugin.install(&mut world, &mut schedule);
```

Keep existing local variables and world-network setup intact; only insert `FlowFieldPlugin` after HPA and before mobility.

- [x] **Step 4: Refresh caches after graph replacement**

Add a helper near existing HPA refresh helpers:

```rust
fn refresh_flow_field_resources(world: &mut bevy_ecs::world::World) {
    if world.contains_resource::<sim_core::routing::FlowFieldCache>() {
        world.resource_mut::<sim_core::routing::FlowFieldCache>().clear();
    } else {
        world.insert_resource(sim_core::routing::FlowFieldCache::default());
    }
}
```

Call it anywhere `refresh_hpa_index` or equivalent is called after mobility hydration or `set_mobility_for_test` graph replacement:

```rust
refresh_hpa_index(&mut self.world);
refresh_flow_field_resources(&mut self.world);
```

- [x] **Step 5: Run targeted runtime tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-server runtime_ -- --nocapture
```

Expected: runtime tests pass.

- [x] **Step 6: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-core/src/routing/plugin.rs
git commit -m "feat(8e): install flow fields in runtime"
```

---

## Task 6: Strict Frontend Mobility Decode

**Files:**
- Modify: `src/backend/mobilityProtocol.ts`
- Modify: `tests/backend/mobilityProtocol.test.ts`

- [x] **Step 1: Add failing frontend tests**

In `tests/backend/mobilityProtocol.test.ts`, add:

```ts
import { create } from '@bufbuild/protobuf';
import { AgentMobilitySchema, AgentStateSchema, Direction, WalkingSchema } from '../../src/backend/proto/abutown_pb';
import { agentMobilityFromProto } from '../../src/backend/mobilityProtocol';

describe('strict agent proto conversion', () => {
  it('rejects missing agent state', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:bad',
      worldCoord: { x: 1, y: 2 },
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(() => agentMobilityFromProto(proto)).toThrow(/missing AgentState/);
  });

  it('rejects missing world coord', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:bad',
      state: create(AgentStateSchema, {
        state: { case: 'walking', value: create(WalkingSchema, { linkId: 'edge:7', progress: 0.5 }) },
      }),
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(() => agentMobilityFromProto(proto)).toThrow(/missing world_coord/);
  });

  it('accepts graph-native walking edge ids', () => {
    const proto = create(AgentMobilitySchema, {
      id: 'agent:ok',
      state: create(AgentStateSchema, {
        state: { case: 'walking', value: create(WalkingSchema, { linkId: 'edge:7', progress: 0.5 }) },
      }),
      worldCoord: { x: 7, y: 8 },
      direction: Direction.E,
      spriteKey: 'pedestrian:0',
      planCursor: 0,
    });

    expect(agentMobilityFromProto(proto).state).toEqual({
      type: 'walking',
      link_id: 'edge:7',
      progress: 0.5,
    });
  });
});
```

If those imports already exist in the file, merge the new names into the existing import statements instead of duplicating imports.

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false tests/backend/mobilityProtocol.test.ts
```

Expected: tests fail because `agentMobilityFromProto` currently fabricates `at_activity` and `(0, 0)`.

- [x] **Step 3: Make proto conversion strict**

In `src/backend/mobilityProtocol.ts`, replace `agentStateFromProto` with:

```ts
function agentStateFromProto(state: AgentStateProto | undefined): AgentMobilityStateDto {
  if (!state || state.state.case === undefined) {
    throw new Error('missing AgentState');
  }
  switch (state.state.case) {
    case 'walking':
      return { type: 'walking', link_id: state.state.value.linkId, progress: state.state.value.progress };
    case 'waitingAtStop':
      return { type: 'waiting_at_stop', stop_id: state.state.value.stopId };
    case 'inVehicle':
      return { type: 'in_vehicle', vehicle_id: state.state.value.vehicleId, seat_index: state.state.value.seatIndex };
    case 'boarding':
      return { type: 'boarding', vehicle_id: state.state.value.vehicleId, stop_id: state.state.value.stopId };
    case 'alighting':
      return { type: 'alighting', vehicle_id: state.state.value.vehicleId, stop_id: state.state.value.stopId };
    case 'atActivity':
      return { type: 'at_activity', activity_id: state.state.value.activityId };
  }
}
```

Replace the world-coordinate assignment in `agentMobilityFromProto`:

```ts
  if (!p.worldCoord) {
    throw new Error('missing world_coord');
  }
```

Then return:

```ts
    world_coord: { x: p.worldCoord.x, y: p.worldCoord.y },
```

- [x] **Step 4: Run targeted frontend tests**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false tests/backend/mobilityProtocol.test.ts
```

Expected: mobility protocol tests pass.

- [x] **Step 5: Commit**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add src/backend/mobilityProtocol.ts tests/backend/mobilityProtocol.test.ts
git commit -m "fix(8e): reject malformed mobility proto frames"
```

---

## Task 7: Verification, Progress, and Review

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Run full backend verification**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown/backend
cargo test -p sim-core routing::flow_field -- --nocapture
cargo test -p sim-core mobility:: -- --nocapture
cargo test -p sim-server runtime_ -- --nocapture
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
```

Expected:

- All targeted flow-field tests pass.
- All targeted mobility tests pass.
- Runtime tests show `FlowFieldCache` installed and refreshed.
- Full workspace tests pass.
- Clippy exits cleanly with `-D warnings`.

- [x] **Step 2: Run frontend verification**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
./node_modules/.bin/tsc --noEmit --pretty false
./node_modules/.bin/vitest run --passWithNoTests --reporter=dot --pool=forks --fileParallelism=false
```

Expected: TypeScript compiles and Vitest passes.

- [x] **Step 3: Run browser smoke**

Ensure the dev stack is running at `http://127.0.0.1:5175/` and backend at `http://127.0.0.1:8080/`, then run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
node scripts/smoke-7b.mjs
```

Expected: all smoke checks pass, binary mobility frames are received, and console errors are zero. If the Postgres-backed local state is quiet, run the same smoke against a fresh in-memory backend and document both results in `progress.md`.

- [x] **Step 4: Run acceptance greps**

Run:

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
rg -n "fall back|fallback|unwrap_or\\(\\(0\\.0, 0\\.0\\)\\)|at_activity with empty|synthetic link|global A\\*" backend/crates/sim-core/src backend/crates/sim-server/src src tests
rg -n "FlowFieldCache|ActiveRoute|route_assignment_system|route_advance_system" backend/crates/sim-core/src
```

Expected: the first grep returns no newly introduced production fallback behavior. The second grep shows the expected 8e implementation symbols.

- [x] **Step 5: Record progress**

Prepend an entry to `progress.md`:

```text
2026-05-26T18:30:00.000Z - Phase 8e verification pass: graph flow fields, `FlowFieldCache`, and live walking `ActiveRoute` execution are implemented. Walking agents can execute multi-edge graph routes through canonical edge keys while frontend rendering remains driven by authoritative backend coordinates. Mobility snapshots now round-trip active route execution with serde defaults for pre-8e snapshots and explicit validation for invalid route steps. Runtime installs `FlowFieldPlugin` after HPA and clears flow-field cache when graph/HPA resources refresh. Backend targeted tests, full workspace tests, clippy, tsc, Vitest, browser smoke, and no-fallback acceptance greps all pass. Include exact command evidence after this sentence: targeted flow-field test count, targeted mobility test count, runtime test count, full workspace test count, clippy result, TypeScript result, Vitest file/test count, smoke frame/delta counts, and acceptance grep result.
```

Before committing `progress.md`, adjust the timestamp to the actual UTC minute when verification completes and replace the evidence sentence with the real command outputs from Steps 1-4.

- [x] **Step 6: Commit progress**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add progress.md
git commit -m "docs(8e): record flow field verification"
```

- [x] **Step 7: Request code review**

Use `superpowers:requesting-code-review` before merging or pushing. Ask the reviewer to focus on:

- Correctness of reverse Dijkstra against `RoutingProfile::transition`.
- Any accidental global-routing retry or invented route id behavior.
- ActiveRoute persistence/hydration validation.
- Schedule ordering around route assignment, route advancement, and existing stop arrival.
- Frontend strict decode behavior and smoke coverage.

---

## Self-Review

Spec coverage:

- Flow fields and cache are covered by Tasks 1-2.
- Live walking execution is covered by Task 4.
- Persistence and hydration validation are covered by Task 3.
- Runtime plugin and cache refresh are covered by Task 5.
- Frontend strict decode requirements are covered by Task 6.
- Full verification, smoke, progress, and review are covered by Task 7.

Placeholder scan:

- The plan contains no placeholder tokens or deferred implementation items.
- The progress entry step includes a concrete sentence shape and instructs the implementer to replace the evidence sentence with real command output before committing `progress.md`.

Type consistency:

- `FlowField`, `FlowFieldCache`, `FlowFieldCacheKey`, `FlowFieldScope`, `ActiveRoute`, `RouteStep`, `PersistedActiveRoute`, `PersistedRouteStep`, `RouteAssignmentStats`, `route_assignment_system`, and `route_advance_system` names are consistent across tasks.
- `RoutingProfileKey::Walk`, `ModeState::Walking`, `NodeId`, `EdgeId`, and `ClusterId` match existing routing names.
