# Traffic Simulation Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove tram/transit mobility runtime code and make cars move through a backend-authoritative road-only traffic route catalog.

**Architecture:** Replace `TransitLines` with `TrafficRoutes`, a road-only catalog built from `EdgeKind::Road` graph edges. Keep passive rail visuals in the frontend map, but remove all tram vehicles, tram DTO success paths, tram renderer drawables, and transit-interest WebSocket code. Preserve backend-required startup, Mini Metro-style vector rendering, and deterministic smoke-test diagnostics.

**Tech Stack:** Rust 2024, Bevy ECS, prost protobuf DTOs, Axum runtime, TypeScript, Vite, Vitest, Playwright.

---

## File Structure

- Create `backend/crates/sim-core/src/routing/traffic.rs`
  - Owns `TrafficRouteId`, `TrafficRoute`, and `TrafficRoutes`.
  - Provides legacy route lookup for `route:arterial:*`.
- Modify `backend/crates/sim-core/src/routing/mod.rs`
  - Exports `traffic`.
  - Stops exporting transit runtime types.
- Modify `backend/crates/sim-core/src/routing/builder.rs`
  - Returns `(Graph, TrafficRoutes, NodeSpatialIndex)`.
  - Builds road edges and road traffic routes only.
  - Removes `SeededTransitLine` and transit line construction.
- Modify `backend/crates/sim-core/src/routing/plugin.rs`
  - Installs `TrafficRoutes`.
  - Removes `seeded_transit_lines`.
- Modify `backend/crates/sim-core/src/mobility/components.rs`
  - Changes `RoutePosition` from `LineId` to `TrafficRouteId`.
- Modify `backend/crates/sim-core/src/mobility/mod.rs`
  - Changes `vehicle_world_coord` to resolve `TrafficRoutes`.
  - Removes transit arguments from agent coordinate helpers.
- Modify `backend/crates/sim-core/src/mobility/api.rs`
  - Installs `TrafficRoutes` defaults.
  - Spawns vehicles via traffic routes.
  - Emits car DTOs with road route ids.
  - Removes stop records derived from transit lines.
- Modify `backend/crates/sim-core/src/mobility/systems.rs`
  - Vehicle cache, advance, coordinate, and direction systems use `TrafficRoutes`.
  - Removes tram LOD bypass and transit boarding/alighting from the installed schedule.
- Modify `backend/crates/sim-core/src/mobility/seed.rs`
  - Removes tram seeding and tram-line seed validation.
  - Seeds only pedestrians, cars, and car driver agents.
- Modify `backend/crates/sim-core/src/mobility/persist_snapshot.rs`
  - Persists and restores route records through `TrafficRoutes`.
- Modify `backend/crates/sim-core/src/mobility/records.rs`
  - `VehicleKind` becomes car-only.
- Modify `backend/crates/protocol/src/lib.rs`
  - `VehicleKindDto` becomes car-only.
  - Proto conversion rejects unsupported tram wire values at the boundary.
- Modify `backend/crates/sim-server/src/app.rs`
  - Vehicle proto emission maps only car DTOs.
- Modify `backend/crates/sim-server/src/runtime.rs`
  - Removes tram snapshot freshness and test expectations.
- Modify `src/backend/mobilityProtocol.ts`
  - `VehicleKindDto` becomes `'car'`.
  - Proto conversion throws for `VehicleKind.TRAM`.
- Modify `src/backend/mobilityState.ts`
  - Removes tram left-vehicle retention.
  - Adds traffic diagnostics derived from backend car state.
- Modify `src/backend/mobilityClient.ts`
  - Removes transit-interest chunk expansion.
- Modify `src/render/backendMobilityDrawables.ts`
  - Removes `BackendTram` and `tramsFromMobilityState`.
- Modify `src/render/minimalMapRenderer.ts`
  - Removes moving train drawables; keeps passive rail path drawing.
- Modify `src/app/runtimeDiagnostics.ts`
  - Removes `mobilityTrams`, `train`, and runtime train count.
  - Adds `traffic`.
- Modify `src/main.ts`
  - Removes runtime calls to `tramsFromMobilityState`.
- Modify tests under `tests/backend`, `tests/render`, `tests/app`, and `tests/e2e`
  - Enforces car-only runtime and traffic diagnostics.

## Task 1: Add Road-Only `TrafficRoutes`

**Files:**
- Create: `backend/crates/sim-core/src/routing/traffic.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`
- Modify: `backend/crates/sim-core/src/routing/builder.rs`
- Modify: `backend/crates/sim-core/src/routing/plugin.rs`

- [ ] **Step 1: Write the failing routing-builder tests**

Add these tests to `backend/crates/sim-core/src/routing/builder.rs` inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn builder_creates_traffic_routes_from_road_edges_only() {
    let (graph, traffic_routes, _) = build_graph_from_city_network(&simple_network(), &[], &[]);

    assert_eq!(traffic_routes.count(), 2);
    assert!(traffic_routes.route_by_legacy("route:arterial:0").is_some());
    assert!(traffic_routes.route_by_legacy("route:arterial:1").is_some());

    for route in traffic_routes.iter() {
        assert!(
            route.edges.len() >= 2,
            "traffic routes include forward and reverse road edges so route-end looping is physical"
        );
        for edge_id in &route.edges {
            assert_eq!(graph.edge(*edge_id).kind, EdgeKind::Road);
        }
    }
}

#[test]
fn builder_does_not_create_tram_track_edges_for_runtime_routes() {
    let (graph, traffic_routes, _) = build_graph_from_city_network(&simple_network(), &[], &[]);

    assert!(
        graph.edges().all(|edge| edge.kind != EdgeKind::TramTrack),
        "tram-track edges are not part of the mobility runtime graph"
    );
    assert_eq!(traffic_routes.count(), simple_network().arterial_paths.len());
}
```

- [ ] **Step 2: Run the routing-builder tests and verify failure**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core routing::builder::tests::builder_creates_traffic_routes_from_road_edges_only routing::builder::tests::builder_does_not_create_tram_track_edges_for_runtime_routes
```

Expected: FAIL because `build_graph_from_city_network` still returns `TransitLines`, accepts `seeded_transit_lines`, and `TrafficRoutes` does not exist.

- [ ] **Step 3: Create `TrafficRoutes`**

Create `backend/crates/sim-core/src/routing/traffic.rs`:

```rust
use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::routing::graph::EdgeId;

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct TrafficRouteId(pub u32);

#[derive(Debug, Clone)]
pub struct TrafficRoute {
    pub id: TrafficRouteId,
    pub name: String,
    pub edges: Vec<EdgeId>,
    pub legacy_route_id: String,
}

#[derive(Resource, Debug, Default)]
pub struct TrafficRoutes {
    routes: Vec<TrafficRoute>,
    by_legacy_route_id: HashMap<String, TrafficRouteId>,
}

impl TrafficRoutes {
    pub fn new(routes: Vec<TrafficRoute>) -> Self {
        let mut by_legacy_route_id = HashMap::new();
        for route in &routes {
            by_legacy_route_id.insert(route.legacy_route_id.clone(), route.id);
        }
        Self {
            routes,
            by_legacy_route_id,
        }
    }

    pub fn route(&self, id: TrafficRouteId) -> &TrafficRoute {
        &self.routes[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &TrafficRoute> {
        self.routes.iter()
    }

    pub fn count(&self) -> usize {
        self.routes.len()
    }

    pub fn route_by_legacy(&self, legacy_id: &str) -> Option<TrafficRouteId> {
        self.by_legacy_route_id.get(legacy_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_routes_lookup_by_legacy_id() {
        let routes = TrafficRoutes::new(vec![TrafficRoute {
            id: TrafficRouteId(0),
            name: "arterial_0".into(),
            edges: vec![EdgeId(3)],
            legacy_route_id: "route:arterial:0".into(),
        }]);

        assert_eq!(routes.count(), 1);
        assert_eq!(routes.route_by_legacy("route:arterial:0"), Some(TrafficRouteId(0)));
        assert!(routes.route_by_legacy("tram:rail:0").is_none());
    }
}
```

- [ ] **Step 4: Export traffic routes**

In `backend/crates/sim-core/src/routing/mod.rs`, add:

```rust
pub mod traffic;
```

Replace the transit export line with:

```rust
pub use traffic::{TrafficRoute, TrafficRouteId, TrafficRoutes};
```

Keep `pub mod transit;` through Task 3 while mobility compile errors are being migrated. Delete `transit.rs` in Task 4 after the search gate proves no code references it.

- [ ] **Step 5: Rewrite builder return type and road route construction**

In `backend/crates/sim-core/src/routing/builder.rs`:

Replace the imports:

```rust
use crate::routing::graph::{Edge, EdgeId, EdgeKind, Graph, Node, NodeId, NodeKind};
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::traffic::{TrafficRoute, TrafficRouteId, TrafficRoutes};
```

Delete `SPEED_TRAM` and delete `SeededTransitLine`.

Change the function signature:

```rust
pub fn build_graph_from_city_network(
    network: &CityNetwork,
    seeded_stops: &[SeededStop],
    seeded_walks: &[SeededWalk],
) -> (Graph, TrafficRoutes, NodeSpatialIndex) {
```

Inside arterial edge emission, replace the `TramTrack` and `Road` four-edge block with this road-only block:

```rust
let fwd_id = EdgeId(edges.len() as u32);
edges.push(Edge {
    id: fwd_id,
    from,
    to,
    polyline: polyline.clone(),
    length,
    kind: EdgeKind::Road,
    speed_limit: SPEED_ROAD,
    capacity: 1,
    legacy_id: Some(format!("link:road:{index}:{}_{},fwd", segment[0].0, segment[0].1)),
});
road_forward_by_arterial.entry(index).or_default().push(fwd_id);

let rev_id = EdgeId(edges.len() as u32);
edges.push(Edge {
    id: rev_id,
    from: to,
    to: from,
    polyline: polyline.iter().rev().copied().collect(),
    length,
    kind: EdgeKind::Road,
    speed_limit: SPEED_ROAD,
    capacity: 1,
    legacy_id: Some(format!("link:road:{index}:{}_{},rev", segment[0].0, segment[0].1)),
});
road_reverse_by_arterial.entry(index).or_default().push(rev_id);
```

At the start of Phase 4, define:

```rust
let mut road_forward_by_arterial: HashMap<usize, Vec<EdgeId>> = HashMap::new();
let mut road_reverse_by_arterial: HashMap<usize, Vec<EdgeId>> = HashMap::new();
```

After `let spatial_index = NodeSpatialIndex::from_nodes(graph.nodes());`, replace transit-line construction with:

```rust
let mut routes: Vec<TrafficRoute> = Vec::new();
let mut arterial_indices: Vec<usize> = road_forward_by_arterial.keys().copied().collect();
arterial_indices.sort();
for arterial_idx in arterial_indices {
    let mut route_edges = road_forward_by_arterial.remove(&arterial_idx).unwrap_or_default();
    if let Some(reverse_edges) = road_reverse_by_arterial.remove(&arterial_idx) {
        route_edges.extend(reverse_edges.into_iter().rev());
    }
    if route_edges.is_empty() {
        continue;
    }
    routes.push(TrafficRoute {
        id: TrafficRouteId(routes.len() as u32),
        name: format!("arterial_{arterial_idx}"),
        edges: route_edges,
        legacy_route_id: format!("route:arterial:{arterial_idx}"),
    });
}

let traffic_routes = TrafficRoutes::new(routes);
(graph, traffic_routes, spatial_index)
```

- [ ] **Step 6: Update routing plugin**

In `backend/crates/sim-core/src/routing/plugin.rs`, remove `SeededTransitLine` and `TransitLines` imports. Import `TrafficRoutes`.

Change `RoutingPlugin`:

```rust
#[derive(Default)]
pub struct RoutingPlugin {
    pub seeded_stops: Vec<SeededStop>,
    pub seeded_walks: Vec<SeededWalk>,
}
```

Change install:

```rust
let (graph, traffic_routes, spatial_index) = match world.get_resource::<CityNetwork>() {
    Some(network) => build_graph_from_city_network(network, &self.seeded_stops, &self.seeded_walks),
    None => (Graph::default(), TrafficRoutes::default(), NodeSpatialIndex::default()),
};
world.insert_resource(graph);
world.insert_resource(traffic_routes);
world.insert_resource(spatial_index);
world.insert_resource(WaitingAgents::default());
```

Update the plugin test assertion:

```rust
assert!(world.contains_resource::<TrafficRoutes>());
```

- [ ] **Step 7: Run routing tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core routing::traffic routing::builder routing::plugin
```

Expected: PASS for touched routing tests. Other modules may still fail because mobility still references `TransitLines`.

- [ ] **Step 8: Commit routing route catalog**

```bash
git add backend/crates/sim-core/src/routing
git commit -m "refactor: add road traffic route catalog"
```

## Task 2: Move Mobility Vehicle Positions To `TrafficRoutes`

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/components.rs`
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`
- Modify: `backend/crates/sim-core/src/mobility/api.rs`
- Modify: `backend/crates/sim-core/src/mobility/records.rs`

- [ ] **Step 1: Write failing vehicle coordinate tests**

In `backend/crates/sim-core/src/mobility/mod.rs`, update the test helper `install_test_routing` to install `TrafficRoutes` instead of `TransitLines`, then add this test:

```rust
#[test]
fn vehicle_world_coord_resolves_traffic_route_edges() {
    let mut world = World::new();
    install_test_routing(&mut world);
    let traffic_routes = world.resource::<crate::routing::TrafficRoutes>();
    let graph = world.resource::<crate::routing::Graph>();
    let route_id = traffic_routes.route_by_legacy("route:old-town-loop").unwrap();
    let rp = components::RoutePosition {
        route_id,
        edge_index: 0,
        progress: 0.5,
        speed: 0.1,
    };

    assert_eq!(
        vehicle_world_coord(&rp, traffic_routes, graph),
        Some((15.0, 0.0))
    );
}
```

- [ ] **Step 2: Run the mobility coordinate test and verify failure**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility::tests::vehicle_world_coord_resolves_traffic_route_edges
```

Expected: FAIL because `RoutePosition` still uses `LineId` and `vehicle_world_coord` still expects `TransitLines`.

- [ ] **Step 3: Update mobility records to car-only**

In `backend/crates/sim-core/src/mobility/records.rs`, replace `VehicleKind` and conversion with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VehicleKind {
    Car,
}

impl From<VehicleKind> for abutown_protocol::VehicleKindDto {
    fn from(value: VehicleKind) -> Self {
        match value {
            VehicleKind::Car => abutown_protocol::VehicleKindDto::Car,
        }
    }
}
```

- [ ] **Step 4: Update `RoutePosition`**

In `backend/crates/sim-core/src/mobility/components.rs`, replace `RoutePosition` with:

```rust
/// Vehicle position along its current road traffic route edge. The wire shape
/// still uses `route_id: String` + `link_index: usize`; conversion happens at
/// the emission boundary through `TrafficRoutes::route(route_id)`.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct RoutePosition {
    pub route_id: crate::routing::TrafficRouteId,
    pub edge_index: usize,
    pub progress: f32,
    pub speed: f32,
}
```

- [ ] **Step 5: Update coordinate helpers**

In `backend/crates/sim-core/src/mobility/mod.rs`, change `agent_world_coord` signature:

```rust
pub fn agent_world_coord(
    state: &AgentMobilityState,
    graph: &crate::routing::Graph,
) -> Option<(f32, f32)> {
```

Remove the unused transit argument from its callers.

Replace `vehicle_world_coord`:

```rust
pub fn vehicle_world_coord(
    route_position: &components::RoutePosition,
    traffic_routes: &crate::routing::TrafficRoutes,
    graph: &crate::routing::Graph,
) -> Option<(f32, f32)> {
    if (route_position.route_id.0 as usize) >= traffic_routes.count() {
        return None;
    }
    let route = traffic_routes.route(route_position.route_id);
    let edge_id = *route.edges.get(route_position.edge_index)?;
    let edge = graph.edge(edge_id);
    Some(crate::mobility_geometry::world_coord_at_progress_slice(
        &edge.polyline,
        route_position.progress,
    ))
}
```

- [ ] **Step 6: Update API route lookup**

In `backend/crates/sim-core/src/mobility/api.rs`, replace the `TransitLines` default insert with:

```rust
if !world.contains_resource::<crate::routing::TrafficRoutes>() {
    world.insert_resource(crate::routing::TrafficRoutes::default());
}
```

Change `compute_vehicle_sprite_key`:

```rust
fn compute_vehicle_sprite_key(id: &VehicleId) -> String {
    format!("vehicle:{}", stable_index(&id.0) % 8)
}
```

In `spawn_vehicle_from_record`, replace route resolution with:

```rust
let route_id = world
    .resource::<crate::routing::TrafficRoutes>()
    .route_by_legacy(&record.route_id)
    .unwrap_or_else(|| panic!("unknown traffic route_id {}", record.route_id));
let edge_index = record.link_index;
let (px, py) = {
    let traffic_routes = world.resource::<crate::routing::TrafficRoutes>();
    let graph = world.resource::<crate::routing::Graph>();
    let rp = RoutePosition {
        route_id,
        edge_index,
        progress: record.progress,
        speed: record.speed_per_tick,
    };
    crate::mobility::vehicle_world_coord(&rp, traffic_routes, graph)
        .expect("vehicle route position must resolve through traffic routes")
};
```

Store `RoutePosition { route_id, edge_index, progress: record.progress, speed: record.speed_per_tick }`.

Replace `legacy_route_id_for` with:

```rust
fn legacy_route_id_for(world: &World, route_id: crate::routing::TrafficRouteId) -> String {
    let routes = world.resource::<crate::routing::TrafficRoutes>();
    if (route_id.0 as usize) < routes.count() {
        return routes.route(route_id).legacy_route_id.clone();
    }
    panic!("unknown traffic route_id {}", route_id.0)
}
```

Update vehicle record/DTO callers to use `pos.route_id`.

- [ ] **Step 7: Run focused mobility tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility::tests::vehicle_world_coord_resolves_traffic_route_edges
```

Expected: PASS.

- [ ] **Step 8: Commit mobility route position migration**

```bash
git add backend/crates/sim-core/src/mobility/components.rs backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-core/src/mobility/api.rs backend/crates/sim-core/src/mobility/records.rs
git commit -m "refactor: move vehicles to traffic routes"
```

## Task 3: Migrate Mobility Systems And Remove Transit Advance Paths

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/systems.rs`
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`

- [ ] **Step 1: Write failing vehicle advance tests**

In `backend/crates/sim-core/src/mobility/systems.rs`, add or update tests so a car route advances through road edges and loops:

```rust
#[test]
fn vehicle_advance_loops_car_over_traffic_route_edges() {
    use crate::mobility::records::VehicleKind;

    let mut world = World::new();
    let route_id = insert_test_routing(&mut world);
    world.insert_resource(SimulatedChunks(std::iter::once(crate::ids::ChunkCoord { x: 0, y: 0 }).collect()));
    world.insert_resource(DirtyVehicles::default());

    let entity = world
        .spawn((
            VehicleMarker,
            VehicleKindComponent(VehicleKind::Car),
            Position { x: 10.0, y: 0.0 },
            RoutePosition {
                route_id,
                edge_index: 0,
                progress: 1.0,
                speed: 0.1,
            },
            DwellTicksRemaining(0),
        ))
        .id();

    let mut schedule = Schedule::default();
    schedule.add_systems(vehicle_advance_system);
    schedule.run(&mut world);

    let pos = world.get::<RoutePosition>(entity).unwrap();
    assert_eq!(pos.edge_index, 1);
    assert_eq!(pos.progress, 0.0);
}
```

Update the existing `insert_test_routing` helper so it returns `TrafficRouteId`.

- [ ] **Step 2: Run the systems test and verify failure**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility::systems::tests::vehicle_advance_loops_car_over_traffic_route_edges
```

Expected: FAIL because systems still use `TransitLines` and tram-specific LOD bypass.

- [ ] **Step 3: Add traffic route edge helper**

Near the top of `backend/crates/sim-core/src/mobility/systems.rs`, add:

```rust
fn traffic_route_edge<'a>(
    graph: &'a crate::routing::Graph,
    traffic_routes: &crate::routing::TrafficRoutes,
    route_position: &RoutePosition,
) -> Option<&'a crate::routing::Edge> {
    if (route_position.route_id.0 as usize) >= traffic_routes.count() {
        return None;
    }
    let route = traffic_routes.route(route_position.route_id);
    let edge_id = *route.edges.get(route_position.edge_index)?;
    Some(graph.edge(edge_id))
}
```

- [ ] **Step 4: Update `update_link_polyline_cache_system`**

Replace the vehicle half of `update_link_polyline_cache_system` so it takes `traffic_routes: Res<crate::routing::TrafficRoutes>` and uses:

```rust
let resolved: Option<(String, Vec<(f32, f32)>)> =
    traffic_route_edge(&graph, &traffic_routes, rp).map(|edge| {
        let lid = edge
            .legacy_id
            .clone()
            .unwrap_or_else(|| format!("edge:{}", edge.id.0));
        (lid, edge.polyline.clone())
    });
```

- [ ] **Step 5: Update `vehicle_advance_system`**

Replace the system parameters with:

```rust
pub fn vehicle_advance_system(
    mut query: Query<
        (
            Entity,
            &Position,
            &mut RoutePosition,
            &mut DwellTicksRemaining,
        ),
        With<VehicleMarker>,
    >,
    simulated: Res<SimulatedChunks>,
    traffic_routes: Res<crate::routing::TrafficRoutes>,
    mut dirty: ResMut<DirtyVehicles>,
) {
```

Use this body:

```rust
for (entity, world_pos, mut pos, mut dwell) in query.iter_mut() {
    if !chunk_is_simulated(world_pos, &simulated) {
        continue;
    }
    if dwell.0 > 0 {
        dwell.0 -= 1;
        dirty.0.insert(entity);
        continue;
    }
    if (pos.route_id.0 as usize) >= traffic_routes.count()
        || traffic_routes.route(pos.route_id).edges.is_empty()
    {
        continue;
    }
    if pos.progress >= 1.0 {
        let route = traffic_routes.route(pos.route_id);
        pos.edge_index = (pos.edge_index + 1) % route.edges.len();
        pos.progress = 0.0;
        dirty.0.insert(entity);
        continue;
    }
    let next = (pos.progress + pos.speed).min(1.0);
    if next != pos.progress {
        pos.progress = next;
        dirty.0.insert(entity);
    }
}
```

- [ ] **Step 6: Update output systems**

In `compute_world_coord_system`, replace `transit_lines` with `traffic_routes` and call:

```rust
crate::mobility::vehicle_world_coord(rp, &traffic_routes, &graph)
```

For agents, call:

```rust
crate::mobility::agent_world_coord(&state.0, &graph)
```

In `compute_direction_system`, replace the vehicle fallback with:

```rust
if let Some(edge) = traffic_route_edge(&graph, &traffic_routes, rp) {
    dir.0 = dir_at_progress(&edge.polyline, rp.progress);
}
```

For walking agents, keep the cached-link fast path and use `edge_by_canonical_key` for uncached links.

- [ ] **Step 7: Remove transit boarding/alighting from the installed schedule**

In `install_systems`, remove `boarding_alighting_system` from the schedule. Keep `stop_arrival_system` only for walking state completion, but do not install any system that matches vehicles to stops.

Change the ordering comment to:

```rust
//   1. route_assignment    — assign graph routes to un-routed walkers.
//   2. route_advance       — move completed route edges to the next edge.
//   3. update_link_cache   — refresh edge polylines after route changes.
//   4. walk_advance        — push Walking agents along their link.
//   5. stop_arrival        — convert progress=1.0 walkers into terminal states.
//   6. vehicle_advance     — decrement dwell or push cars along road routes.
```

- [ ] **Step 8: Remove transit arguments from LOD helpers**

Change `promote_warm_to_active_system`, `demote_active_to_warm_system`, and `agent_destination_chunk` to call `agent_world_coord(&state.0, &graph)` without `TransitLines`.

- [ ] **Step 9: Run focused system tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility::systems
```

Expected: PASS after updating tests that intentionally created `VehicleKind::Tram` to use `VehicleKind::Car` or deleting tests that only covered transit boarding.

- [ ] **Step 10: Commit systems migration**

```bash
git add backend/crates/sim-core/src/mobility/systems.rs backend/crates/sim-core/src/mobility/mod.rs
git commit -m "refactor: advance vehicles on traffic routes"
```

## Task 4: Remove Tram Seeding And Runtime Transit Records

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/seed.rs`
- Modify: `backend/crates/sim-core/src/mobility/persist_snapshot.rs`
- Modify: `backend/crates/sim-core/src/routing/mod.rs`
- Delete: `backend/crates/sim-core/src/routing/transit.rs`

- [ ] **Step 1: Write failing seed tests**

In `backend/crates/sim-core/src/mobility/seed.rs`, add:

```rust
#[test]
fn from_base_world_bundle_seeds_no_trams() {
    let bundle = crate::base_world::load_base_world_bundle(
        &workspace_root().join("data/base-world/zurich-river-city-v1.json"),
    )
    .expect("base world bundle should load");

    let (world, _) = from_base_world_bundle(&bundle).expect("base world should seed");
    let vehicles = crate::mobility::api::vehicles(&world);

    assert!(vehicles.iter().all(|vehicle| vehicle.kind == VehicleKind::Car));
    assert!(vehicles.iter().any(|vehicle| vehicle.id.0.starts_with("vehicle:car:")));
}
```

Use the existing `workspace_root()` helper if present in the module; otherwise add the same helper used in `mobility/mod.rs`.

- [ ] **Step 2: Run the seed test and verify failure**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility::seed::tests::from_base_world_bundle_seeds_no_trams
```

Expected: FAIL while `seed_trams_from_bundle` still creates tram vehicles.

- [ ] **Step 3: Remove tram seed types and calls**

In `backend/crates/sim-core/src/mobility/seed.rs`:

- Delete `seeded_transit_lines_from_base_world`.
- Delete `SeedError::MissingRailPath` and `SeedError::EmptyTramLine`.
- Delete `seed_trams_from_bundle`.
- Remove `seed_trams_from_bundle(&mut world, bundle)?;`.
- Change `RoutingPlugin` construction to:

```rust
crate::routing::RoutingPlugin {
    seeded_stops: Vec::new(),
    seeded_walks: seeded_walks_from_network(&bundle.to_city_network()),
}
.install(&mut world, &mut schedule);
```

In `test_seed_world`, change seeded test vehicles to `VehicleKind::Car` and route ids that resolve through `TrafficRoutes`.

- [ ] **Step 4: Migrate persistence to `TrafficRoutes`**

In `backend/crates/sim-core/src/mobility/persist_snapshot.rs`, replace imports of `TransitLine` and `TransitLines` with `TrafficRoute`, `TrafficRouteId`, and `TrafficRoutes`.

Where extraction currently iterates transit lines, use:

```rust
let traffic_routes = world.resource::<TrafficRoutes>();
for route in traffic_routes.iter() {
    let route_id = route.legacy_route_id.clone();
    let links = route
        .edges
        .iter()
        .map(|edge_id| {
            let edge = graph.edge(*edge_id);
            PersistedRouteLink {
                id: edge
                    .legacy_id
                    .clone()
                    .unwrap_or_else(|| format!("edge:{}", edge.id.0)),
                polyline: edge.polyline.clone(),
            }
        })
        .collect();
    routes.insert(route_id.clone(), PersistedRoute { id: route_id, links });
}
```

Where restore currently inserts `TransitLines`, construct `TrafficRoutes`:

```rust
let traffic_routes = routes
    .values()
    .enumerate()
    .map(|(index, route)| TrafficRoute {
        id: TrafficRouteId(index as u32),
        name: route.id.clone(),
        edges: route
            .links
            .iter()
            .filter_map(|link| graph.edge_by_legacy(&link.id))
            .collect(),
        legacy_route_id: route.id.clone(),
    })
    .collect::<Vec<_>>();
world.insert_resource(TrafficRoutes::new(traffic_routes));
```

- [ ] **Step 5: Delete transit module export**

After all Rust references to `crate::routing::TransitLines`, `TransitLine`, and `LineId` are gone:

```bash
rm backend/crates/sim-core/src/routing/transit.rs
```

Then remove `pub mod transit;` and any `pub use transit::...` line from `backend/crates/sim-core/src/routing/mod.rs`.

- [ ] **Step 6: Run backend search gate**

Run:

```bash
rg -n "TransitLines|TransitLine|SeededTransitLine|seed_trams|VehicleKind::Tram|LineId" backend/crates/sim-core backend/crates/sim-server
```

Expected: no matches in runtime code. Test names may mention removed behavior only if the test asserts rejection of old tram data.

- [ ] **Step 7: Run sim-core tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core
```

Expected: PASS.

- [ ] **Step 8: Commit tram seeding removal**

```bash
git add backend/crates/sim-core/src/mobility/seed.rs backend/crates/sim-core/src/mobility/persist_snapshot.rs backend/crates/sim-core/src/routing
git commit -m "refactor: remove transit runtime seeding"
```

## Task 5: Make Protocol And Server Runtime Car-Only

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Keep: `backend/crates/protocol/proto/abutown.proto`

- [ ] **Step 1: Write failing protocol/server tests**

In `backend/crates/protocol/src/lib.rs`, add a test near proto conversion tests:

```rust
#[test]
fn vehicle_kind_dto_is_car_only() {
    assert_eq!(VehicleKindDto::Car, VehicleKindDto::Car);
}
```

In `backend/crates/sim-server/src/runtime.rs`, update the base-world seed test to assert zero tram vehicles and at least one car:

```rust
let vehicles = sim_core::mobility::api::vehicles(&runtime.world);
assert!(vehicles.iter().all(|vehicle| vehicle.kind == VehicleKind::Car));
assert!(vehicles.iter().any(|vehicle| vehicle.id.0.starts_with("vehicle:car:")));
```

- [ ] **Step 2: Run focused server tests and verify failure**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol vehicle_kind_dto_is_car_only
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_
```

Expected: protocol may pass before implementation, server tests fail until tram expectations are removed.

- [ ] **Step 3: Make `VehicleKindDto` car-only**

In `backend/crates/protocol/src/lib.rs`, replace:

```rust
pub enum VehicleKindDto {
    Car,
    Tram,
}
```

with:

```rust
pub enum VehicleKindDto {
    Car,
}
```

Do not remove `VEHICLE_KIND_TRAM = 2` from `backend/crates/protocol/proto/abutown.proto` in this branch. The wire enum value remains as a legacy invalid value; conversion boundaries reject it by not mapping it to DTO/runtime state.

- [ ] **Step 4: Update server proto emission**

In `backend/crates/sim-server/src/app.rs`, replace:

```rust
let kind = match dto.kind {
    VehicleKindDto::Car => w::VehicleKind::Car,
    VehicleKindDto::Tram => w::VehicleKind::Tram,
};
```

with:

```rust
let kind = match dto.kind {
    VehicleKindDto::Car => w::VehicleKind::Car,
};
```

- [ ] **Step 5: Remove runtime tram freshness checks**

In `backend/crates/sim-server/src/runtime.rs`, remove functions and assertions that:

- call `expected_base_world_tram_ids`
- filter vehicles with `VehicleKind::Tram`
- filter DTOs with `VehicleKindDto::Tram`
- read `sim_core::routing::TransitLines`

Replace freshness checks with car-route checks:

```rust
fn mobility_snapshot_matches_base_world(
    snapshot: &MobilityPersistSnapshot,
    base_world: &BaseWorldBundle,
) -> bool {
    let expected_car_count: usize = base_world
        .spawns
        .car_groups
        .iter()
        .map(|group| group.cars_per_arterial as usize)
        .sum();
    let cars = snapshot
        .vehicles
        .values()
        .filter(|vehicle| vehicle.kind == sim_core::mobility::VehicleKind::Car)
        .count();
    cars == expected_car_count
}
```

- [ ] **Step 6: Run protocol and server tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol
cargo test --manifest-path backend/Cargo.toml -p sim-server
```

Expected: PASS.

- [ ] **Step 7: Commit car-only protocol/server runtime**

```bash
git add backend/crates/protocol/src/lib.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs
git commit -m "refactor: make vehicle protocol car-only"
```

## Task 6: Remove Frontend Tram DTOs, Drawables, And Transit Interest

**Files:**
- Modify: `src/backend/mobilityProtocol.ts`
- Modify: `src/backend/mobilityState.ts`
- Modify: `src/backend/mobilityClient.ts`
- Modify: `src/render/backendMobilityDrawables.ts`
- Modify: `tests/backend/mobilityProtocol.test.ts`
- Modify: `tests/backend/mobilityState.test.ts`
- Modify: `tests/render/backendMobilityDrawables.test.ts`
- Delete: `tests/render/backendTransitDrawables.test.ts`

- [ ] **Step 1: Write failing frontend protocol tests**

In `tests/backend/mobilityProtocol.test.ts`, change the fixture vehicle to a car:

```ts
{
  id: 'vehicle:car:0:0',
  kind: 'car' as const,
  route_id: 'route:arterial:0',
  link_index: 0,
  progress: 0.25,
  capacity: 1,
  occupants: [],
  dwell_ticks_remaining: 0,
  world_coord: { x: 0, y: 0 },
  direction: 'e',
  sprite_key: 'vehicle:0',
}
```

Add this test:

```ts
it('rejects tram vehicle proto values at the DTO boundary', () => {
  const proto = create(VehicleMobilitySchema, {
    id: 'vehicle:tram:0:0',
    kind: VehicleKind.TRAM,
    routeId: 'tram:rail:0',
    linkIndex: 0,
    progress: 0.5,
    capacity: 80,
    occupants: [],
    dwellTicksRemaining: 0,
    worldCoord: create(WorldCoordSchema, { x: 1, y: 2 }),
    direction: Direction.E,
    spriteKey: 'tram:0',
  });

  expect(() => vehicleMobilityFromProto(proto)).toThrow(/unsupported vehicle kind/);
});
```

- [ ] **Step 2: Run frontend protocol tests and verify failure**

Run:

```bash
npx vitest run tests/backend/mobilityProtocol.test.ts --passWithNoTests
```

Expected: FAIL because `VehicleKindDto` still includes `'tram'` and proto conversion maps tram to DTO.

- [ ] **Step 3: Make DTO validation car-only**

In `src/backend/mobilityProtocol.ts`, replace:

```ts
export type VehicleKindDto = 'car' | 'tram';
const VEHICLE_KINDS: ReadonlySet<VehicleKindDto> = new Set(['car', 'tram']);
```

with:

```ts
export type VehicleKindDto = 'car';
const VEHICLE_KINDS: ReadonlySet<VehicleKindDto> = new Set(['car']);
```

Replace `vehicleKindFromProto`:

```ts
function vehicleKindFromProto(value: VehicleKindProto): VehicleKindDto {
  if (value === VehicleKindProto.CAR) return 'car';
  throw new Error('unsupported vehicle kind');
}
```

- [ ] **Step 4: Remove tram state exceptions**

In `src/backend/mobilityState.ts`, replace the left-vehicle loop with:

```ts
for (const id of msg.left_vehicles) {
  vehicles.delete(id);
}
```

Add traffic diagnostics:

```ts
export type TrafficDiagnostics = {
  routes: number;
  cars: number;
  movingCars: number;
  stuckCars: number;
  invalidRouteCars: number;
};

function moved(entry: InterpolatedEntry<VehicleMobilityDto>): boolean {
  return (
    Math.abs(entry.current.world_coord.x - entry.prev.world_coord.x) > 0.001 ||
    Math.abs(entry.current.world_coord.y - entry.prev.world_coord.y) > 0.001
  );
}

export function trafficDiagnostics(state: MobilityOverlayState): TrafficDiagnostics {
  const cars = [...state.vehicles.values()].filter((entry) => entry.current.kind === 'car');
  const movingCars = cars.filter(moved).length;
  return {
    routes: new Set(cars.map((entry) => entry.current.route_id)).size,
    cars: cars.length,
    movingCars,
    stuckCars: cars.length - movingCars,
    invalidRouteCars: state.invalidMessages,
  };
}
```

- [ ] **Step 5: Remove transit-interest subscription expansion**

In `src/backend/mobilityClient.ts`, replace:

```ts
subscription?.update(withTransitInterestChunks(visible, currentState, world));
```

with:

```ts
subscription?.update(visible);
```

Delete `withTransitInterestChunks`, `chunkKey`, and the `WorldDims` type if unused after this change.

- [ ] **Step 6: Remove tram drawables**

In `src/render/backendMobilityDrawables.ts`:

- Delete `BackendTram`.
- Delete `tramsFromMobilityState`.
- Keep `BackendCar` and `carsFromMobilityState`.

Delete `tests/render/backendTransitDrawables.test.ts`.

- [ ] **Step 7: Run frontend unit tests for state/protocol/drawables**

Run:

```bash
npx vitest run tests/backend/mobilityProtocol.test.ts tests/backend/mobilityState.test.ts tests/render/backendMobilityDrawables.test.ts --passWithNoTests
```

Expected: PASS.

- [ ] **Step 8: Commit frontend tram DTO removal**

```bash
git add src/backend/mobilityProtocol.ts src/backend/mobilityState.ts src/backend/mobilityClient.ts src/render/backendMobilityDrawables.ts tests/backend/mobilityProtocol.test.ts tests/backend/mobilityState.test.ts tests/render/backendMobilityDrawables.test.ts
git rm tests/render/backendTransitDrawables.test.ts
git commit -m "refactor: remove frontend tram mobility paths"
```

## Task 7: Remove Runtime Tram Diagnostics And Moving Train Rendering

**Files:**
- Modify: `src/app/runtimeDiagnostics.ts`
- Modify: `src/render/minimalMapRenderer.ts`
- Modify: `src/main.ts`
- Modify: `tests/app/runtimeDiagnostics.test.ts`
- Modify: `tests/render/minimalGlyphScale.test.ts` only if renderer import changes require a test import update

- [ ] **Step 1: Write failing diagnostics tests**

In `tests/app/runtimeDiagnostics.test.ts`, add:

```ts
it('reports car traffic diagnostics without tram runtime diagnostics', () => {
  const payload = buildRuntimeDiagnosticsPayload(baseOptions());

  expect(payload.city.mobilityTrams).toBeUndefined();
  expect(payload.city.train).toBeUndefined();
  expect(payload.city.trains).toBeUndefined();
  expect(payload.city.traffic).toEqual({
    routes: 0,
    cars: 0,
    movingCars: 0,
    stuckCars: 0,
    invalidRouteCars: 0,
  });
});
```

- [ ] **Step 2: Run diagnostics tests and verify failure**

Run:

```bash
npx vitest run tests/app/runtimeDiagnostics.test.ts --passWithNoTests
```

Expected: FAIL because `mobilityTrams`, `train`, and `trains` still exist.

- [ ] **Step 3: Update runtime diagnostics**

In `src/app/runtimeDiagnostics.ts`:

- Remove imports of `tramsFromMobilityState` and `BackendTram`.
- Import `trafficDiagnostics` from `../backend/mobilityState`.
- Remove `RuntimeTrainDiagnostics`.
- Remove `getTrain` from `RuntimeDiagnosticsOptions`.
- Remove `trains` from `RuntimeCounts`.
- Remove `projectedTrams`, `mobilityTramEntries`, `mobilityTrams`, `trains`, and `train`.
- Add:

```ts
const traffic = trafficDiagnostics(mobilityState);
```

Inside `city`, add:

```ts
traffic,
```

Delete `mobilityTramEntry`.

- [ ] **Step 4: Update minimal renderer**

In `src/render/minimalMapRenderer.ts`:

- Remove `tramsFromMobilityState` and `BackendTram` imports.
- Delete `type TrainDrawable`.
- Remove `TrainDrawable` from `Drawable`.
- Delete `TRAIN_CORE`.
- Delete creation and drawing of `trainDrawables`.
- Delete `drawTrain` if it becomes unused.
- Keep `drawRailPath`, `drawRail`, and passive rail station rendering.

- [ ] **Step 5: Update main diagnostics options**

In `src/main.ts`:

- Remove import of `tramsFromMobilityState`.
- In `getCounts`, remove the `trains` property.
- Remove the whole `getTrain` option.

- [ ] **Step 6: Run frontend type/unit tests**

Run:

```bash
npm test -- --run tests/app/runtimeDiagnostics.test.ts tests/render/backendMobilityDrawables.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit runtime diagnostics cleanup**

```bash
git add src/app/runtimeDiagnostics.ts src/render/minimalMapRenderer.ts src/main.ts tests/app/runtimeDiagnostics.test.ts
git commit -m "refactor: remove tram runtime diagnostics"
```

## Task 8: Update Browser Smoke For Road Traffic Only

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Update smoke assertions**

In `tests/e2e/render-smoke.spec.ts`:

Remove the poll:

```ts
await expect.poll(async () => {
  const state = await readCityState(page);
  return state.city.mobilityTrams.trams.length;
}, { timeout: 10_000 }).toBe(4);
```

Replace tram/train expectations:

```ts
expect(state.city.trains).toBe(4);
...
expect(state.city.mobilityTrams.count).toBe(4);
...
expect(state.city.train).toEqual(expect.objectContaining({
  id: expect.stringMatching(/^vehicle:tram:/),
  position: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
}));
...
await expect.poll(movementObserver(page, (sample) => sample.city.mobilityTrams.trams), {
  timeout: 10_000,
}).toBeGreaterThan(0);
```

with:

```ts
expect(state.city.trains).toBeUndefined();
expect(state.city.train).toBeUndefined();
expect(state.city.mobilityTrams).toBeUndefined();
expect(state.city.traffic).toEqual(expect.objectContaining({
  routes: expect.any(Number),
  cars: expect.any(Number),
  movingCars: expect.any(Number),
  stuckCars: expect.any(Number),
  invalidRouteCars: expect.any(Number),
}));
expect(state.city.traffic.routes).toBeGreaterThanOrEqual(1);
expect(state.city.traffic.cars).toBe(state.city.mobilityVehicles.vehicles.length);
```

Remove `...interactionState.city.mobilityTrams.trams` from the click-candidate list.

Add:

```ts
expect(
  state.city.mobilityVehicles.vehicles.every((vehicle: { kind: string }) => vehicle.kind === 'car'),
).toBe(true);
```

- [ ] **Step 2: Run smoke test and verify failure before backend is complete**

Run:

```bash
npm run test:e2e -- --project=chromium tests/e2e/render-smoke.spec.ts
```

Expected before all backend changes are complete: FAIL. Expected after Tasks 1-7: PASS.

- [ ] **Step 3: Commit smoke update**

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test: smoke road traffic without trams"
```

## Task 9: Full Verification And Old-Code Search Gate

**Files:**
- No planned source edits unless a verification command identifies a defect.

- [ ] **Step 1: Generate protobuf TypeScript**

Run:

```bash
npm run generate:proto
```

Expected: generated TS updates cleanly. If generated files are ignored, `git status` remains unchanged.

- [ ] **Step 2: Run Rust formatting**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

Expected: PASS. If it fails, run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
```

then re-run the check.

- [ ] **Step 3: Run Rust tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: PASS.

- [ ] **Step 4: Run Rust clippy**

Run:

```bash
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 5: Run frontend tests**

Run:

```bash
npm test
```

Expected: PASS.

- [ ] **Step 6: Run production build**

Run:

```bash
npm run build
```

Expected: PASS.

- [ ] **Step 7: Run browser smoke**

Run:

```bash
npm run test:e2e
```

Expected: PASS.

- [ ] **Step 8: Search for removed runtime paths**

Run:

```bash
rg -n "TransitLines|TransitLine|SeededTransitLine|seed_trams|VehicleKind::Tram|mobilityTrams|tramsFromMobilityState|withTransitInterestChunks|vehicle:tram|tram:0" backend/crates src tests
```

Expected: no runtime or test success-path matches. Acceptable matches are comments documenting rejected legacy wire values in protocol tests.

- [ ] **Step 9: Verify app in the browser**

With the dev stack running at `http://127.0.0.1:5175/`, run a Playwright probe:

```bash
node --input-type=module <<'EOF'
import { chromium } from '@playwright/test';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 409, height: 519 } });
await page.goto('http://127.0.0.1:5175/');
await page.waitForFunction(() => document.querySelector('#game')?.getAttribute('data-ready') === 'true', null, { timeout: 10000 });
await page.waitForFunction(() => typeof window.render_game_to_text === 'function', null, { timeout: 10000 });
const state = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? '{}'));
console.log(JSON.stringify({
  status: state.city.mobility.status,
  cars: state.city.mobilityVehicles.vehicles.length,
  traffic: state.city.traffic,
  mobilityTrams: state.city.mobilityTrams,
  train: state.city.train,
}, null, 2));
await browser.close();
EOF
```

Expected output:

```json
{
  "status": "connected",
  "cars": 1,
  "traffic": {
    "routes": 1,
    "cars": 1,
    "movingCars": 1,
    "stuckCars": 0,
    "invalidRouteCars": 0
  }
}
```

The exact car and route counts may be higher than `1`, but `mobilityTrams` and `train` must be absent.

- [ ] **Step 10: Confirm no unplanned verification edits remain**

Run:

```bash
git status --short
```

Expected: no output. If a verification command changed generated files or formatting, return to the task that owns those files, add the exact generated or formatted files there, and commit under that task's commit message.

## Plan Self-Review

- Spec coverage: Tasks 1-4 remove `TransitLines`, tram seeding, and tram runtime movement; Tasks 5-8 remove tram DTO/render/smoke success paths; Task 9 verifies no fallback entities and no old runtime code remain.
- Type consistency: `TrafficRouteId`, `TrafficRoute`, and `TrafficRoutes` are introduced in Task 1 and used by `RoutePosition` in Task 2, systems in Task 3, persistence in Task 4, and diagnostics in Tasks 6-8.
- Scope: This is one coherent branch because backend route catalog, DTO shape, frontend renderer, and smoke expectations must change together to keep the app bootable at every completed task boundary.
