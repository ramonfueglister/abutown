# Sidewalk Footway Simulation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Abutopia pedestrians move on authored sidewalk/footway geometry in the backend, so agents near roads walk on sidewalks instead of the road centerline.

**Architecture:** Keep roads, buildings, terrain, and world tiles as integer tile infrastructure. Add floating-point transport polyline points for movement geometry, author two sidewalk corridors inside the existing road row, and let the existing routing graph turn those corridors into `Footway` edges. Remove the frontend-only pedestrian lane nudge so rendered agents appear at the backend-provided sidewalk coordinates.

**Tech Stack:** Rust (`sim-core`, `sim-server`, Bevy ECS), TypeScript/Vite/Vitest, Playwright, Node base-world generator, `scripts/cargo-serial.sh` for every Cargo command.

---

## Current System Facts

- Roads and buildings are valid integer tile infrastructure. Do not remove or regenerate road/building/world layout while implementing this plan.
- Current Abutopia transport data has one pedestrian corridor, `corridor:main`, with every point at `y: 3`, the same centerline as the road.
- Backend walking already uses `EdgeKind::Footway` only. The bug is that the authored footway geometry is on the road centerline.
- `TransportPath.points` and `CityNetwork.pedestrian_corridors` currently use integer `NetworkCoord`, which cannot represent sidewalk bands inside a road tile.
- The renderer currently adds a hard-coded pedestrian screen offset in `pedestrianRenderStyle`; after backend sidewalk coordinates exist, that visual offset must not move pedestrians again.

## File Structure

- Modify `backend/crates/sim-core/src/city_network.rs`: add floating-point movement points and update `CityNetwork` transport polylines.
- Modify `backend/crates/sim-core/src/base_world.rs`: make `TransportPath.points` floating-point, validate path points with float bounds, and preserve them in `to_city_network`.
- Modify `backend/crates/sim-core/src/routing/builder.rs`: key graph nodes by quantized floating-point coordinates instead of rounded integer coordinates.
- Modify `backend/crates/sim-core/src/mobility/seed.rs`: preserve fractional sidewalk geometry when creating `SeededWalk`s and assert Abutopia spawns on a sidewalk corridor.
- Modify `backend/crates/sim-server/src/runtime.rs`: assert runtime footway edges keep sidewalk coordinates.
- Modify `scripts/generate-abutopia-world.mjs`: author north and south sidewalk corridors beside the existing road centerline.
- Modify generated data under `data/worlds/abutopia/layers/transport.json` and `data/worlds/abutopia/layers/spawns.json` by running `npm run generate:abutopia`.
- Modify `src/backend/baseWorldClient.ts`, `tests/app/baseWorldBundle.test.ts`, and `tests/app/appRuntime.test.ts`: update frontend base-world contract from one centerline corridor to two sidewalk corridors.
- Modify `src/render/entityRenderStyle.ts`, `tests/render/entityRenderStyle.test.ts`, and `tests/render/backendMobilityDrawables.test.ts`: render pedestrians at backend sidewalk coordinates without a default visual lane nudge.
- Modify `tests/e2e/render-smoke.spec.ts`: browser-smoke the real backend-to-render sidewalk coordinate.

## Execution Setup

- [ ] **Step 1: Create an isolated worktree**

Run:

```bash
git fetch origin
git worktree add ../abutown-sidewalk -b codex/sidewalk-footway-simulation origin/main
cd ../abutown-sidewalk
export CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target
```

Expected: new branch `codex/sidewalk-footway-simulation` starts at `origin/main`.

- [ ] **Step 2: Confirm a clean baseline**

Run:

```bash
git status --short --branch
```

Expected: the branch line names `codex/sidewalk-footway-simulation`, and no file lines appear below it.

---

### Task 1: Support Floating Transport Path Coordinates

**Files:**
- Modify: `backend/crates/sim-core/src/city_network.rs`
- Modify: `backend/crates/sim-core/src/base_world.rs`

- [x] **Step 1: Add failing tests for fractional movement points**

In `backend/crates/sim-core/src/city_network.rs`, add this test inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn parses_fractional_transport_points() {
    let fixture = r#"{
        "version": 1,
        "world_id": "abutopia",
        "chunk_size": 32,
        "world_tiles": { "width": 16, "height": 8 },
        "arterial_paths": [
            [{"x": 3, "y": 3}, {"x": 12, "y": 3}]
        ],
        "pedestrian_corridors": [
            [{"x": 2, "y": 2.49}, {"x": 13, "y": 2.49}],
            [{"x": 2, "y": 3.51}, {"x": 13, "y": 3.51}]
        ]
    }"#;

    let network = CityNetwork::from_json(fixture).expect("parses fractional points");

    assert_eq!(network.arterial_paths[0][0], NetworkPoint { x: 3.0, y: 3.0 });
    assert_eq!(
        network.pedestrian_corridors[0][0],
        NetworkPoint { x: 2.0, y: 2.49 },
    );
    assert_eq!(
        network.pedestrian_corridors[1][1],
        NetworkPoint { x: 13.0, y: 3.51 },
    );
}
```

In `backend/crates/sim-core/src/base_world.rs`, add this test block at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_with_pedestrian_points(points: Vec<NetworkPoint>) -> BaseWorldBundle {
        BaseWorldBundle {
            manifest: BaseWorldManifest {
                schema_version: 1,
                world_id: "test".into(),
                display_name: "Test".into(),
                chunk_size: 32,
                world_tiles: WorldTiles { width: 16, height: 8 },
                layers: BaseWorldLayerFiles {
                    terrain: "terrain.json".into(),
                    transport: "transport.json".into(),
                    buildings: "buildings.json".into(),
                    decorations: "decorations.json".into(),
                    spawns: "spawns.json".into(),
                },
            },
            terrain: TerrainLayer {
                schema_version: 1,
                world_id: "test".into(),
                tiles: Vec::new(),
            },
            transport: TransportLayer {
                schema_version: 1,
                world_id: "test".into(),
                roads: vec![RoadTile {
                    x: 3,
                    y: 3,
                    kind: "street".into(),
                    mask: 10,
                }],
                rails: Vec::new(),
                arterial_paths: Vec::new(),
                rail_paths: Vec::new(),
                pedestrian_corridors: vec![TransportPath {
                    id: "corridor:test".into(),
                    points,
                }],
            },
            buildings: BuildingLayer {
                schema_version: 1,
                world_id: "test".into(),
                footprints: vec![BuildingFootprint {
                    id: "building:test".into(),
                    tiles: vec![NetworkCoord { x: 2, y: 3 }],
                    sheet: None,
                    frame: None,
                    district: None,
                }],
            },
            decorations: DecorationLayer {
                schema_version: 1,
                world_id: "test".into(),
                trees: Vec::new(),
                details: Vec::new(),
            },
            spawns: SpawnLayer {
                schema_version: 1,
                world_id: "test".into(),
                pedestrian_groups: Vec::new(),
                car_groups: Vec::new(),
                tram_lines: Vec::new(),
            },
        }
    }

    #[test]
    fn base_world_preserves_fractional_pedestrian_corridor_points() {
        let bundle = bundle_with_pedestrian_points(vec![
            NetworkPoint { x: 2.0, y: 2.49 },
            NetworkPoint { x: 13.0, y: 2.49 },
        ]);

        bundle.validate().expect("fractional transport points are valid");
        let network = bundle.to_city_network();

        assert_eq!(network.pedestrian_corridors.len(), 1);
        assert_eq!(
            network.pedestrian_corridors[0],
            vec![NetworkPoint { x: 2.0, y: 2.49 }, NetworkPoint { x: 13.0, y: 2.49 }],
        );
    }
}
```

- [x] **Step 2: Run the focused failing tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core parses_fractional_transport_points
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core base_world_preserves_fractional_pedestrian_corridor_points
```

Expected: both commands fail because `NetworkPoint` does not exist and `TransportPath.points` still requires integer `NetworkCoord`.

- [x] **Step 3: Add `NetworkPoint` and update `CityNetwork`**

In `backend/crates/sim-core/src/city_network.rs`, replace the top structs with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NetworkPoint {
    pub x: f32,
    pub y: f32,
}

impl From<NetworkCoord> for NetworkPoint {
    fn from(coord: NetworkCoord) -> Self {
        Self {
            x: coord.x as f32,
            y: coord.y as f32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldTiles {
    pub width: u32,
    pub height: u32,
}

#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CityNetwork {
    pub version: u32,
    pub world_id: String,
    pub chunk_size: u16,
    pub world_tiles: WorldTiles,
    pub arterial_paths: Vec<Vec<NetworkPoint>>,
    pub pedestrian_corridors: Vec<Vec<NetworkPoint>>,
}
```

Update the existing city-network fixture assertions to compare with `NetworkPoint { x: 2.0, y: 3.0 }`.

- [x] **Step 4: Update `TransportPath` and float bounds validation**

In `backend/crates/sim-core/src/base_world.rs`, update the import:

```rust
use crate::city_network::{CityNetwork, NetworkCoord, NetworkPoint, WorldTiles};
```

Change `TransportPath`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportPath {
    pub id: String,
    pub points: Vec<NetworkPoint>,
}
```

In `BaseWorldBundle::validate`, replace transport path point validation with:

```rust
for path in self.transport_paths() {
    if path.points.is_empty() {
        return Err(BaseWorldError::EmptyLayer("transport.path.points"));
    }
    for point in &path.points {
        self.require_point_in_bounds(point.x, point.y)?;
    }
}
```

Add these methods next to `require_in_bounds` and `point_in_bounds`:

```rust
fn require_point_in_bounds(&self, x: f32, y: f32) -> Result<(), BaseWorldError> {
    if self.point_in_bounds_f32(x, y) {
        Ok(())
    } else {
        Err(BaseWorldError::OutOfBounds {
            x: x.floor() as i32,
            y: y.floor() as i32,
            width: self.manifest.world_tiles.width,
            height: self.manifest.world_tiles.height,
        })
    }
}

fn point_in_bounds_f32(&self, x: f32, y: f32) -> bool {
    x.is_finite()
        && y.is_finite()
        && x >= 0.0
        && y >= 0.0
        && x < self.manifest.world_tiles.width as f32
        && y < self.manifest.world_tiles.height as f32
}
```

Leave road tiles, building footprints, trees, and decoration details on integer `NetworkCoord`.

- [x] **Step 5: Update Rust constructors that now need `NetworkPoint`**

Run:

```bash
rg -n "NetworkCoord \\{|Vec<Vec<NetworkCoord>>|arterial_paths: vec!|pedestrian_corridors: vec!" backend/crates/sim-core/src backend/crates/sim-server/src
```

For `CityNetwork` path arrays, use `NetworkPoint { x: <n>.0, y: <n>.0 }` or `NetworkCoord { x: <n>, y: <n> }.into()`. Keep non-path tile data as `NetworkCoord`.

Example replacement in Rust tests:

```rust
use crate::city_network::{CityNetwork, NetworkPoint, WorldTiles};

fn np(x: f32, y: f32) -> NetworkPoint {
    NetworkPoint { x, y }
}
```

Then path values become:

```rust
arterial_paths: vec![vec![np(0.0, 0.0), np(10.0, 0.0)]],
pedestrian_corridors: vec![vec![np(0.0, 3.51), np(10.0, 3.51)]],
```

- [x] **Step 6: Re-run focused tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core parses_fractional_transport_points
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core base_world_preserves_fractional_pedestrian_corridor_points
```

Expected: both commands pass.

- [x] **Step 7: Commit data model change**

Run:

```bash
git add backend/crates/sim-core/src/city_network.rs backend/crates/sim-core/src/base_world.rs
git commit -m "feat(mobility): support fractional transport paths"
```

Expected: commit succeeds.

---

### Task 2: Preserve Fractional Footway Nodes in the Routing Graph

**Files:**
- Modify: `backend/crates/sim-core/src/routing/builder.rs`

- [ ] **Step 1: Add a failing routing-builder test**

In `backend/crates/sim-core/src/routing/builder.rs`, add this test inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn builder_preserves_fractional_seeded_walk_nodes() {
    let walks = vec![
        SeededWalk {
            legacy_link_id: "link:walk:corridor:north".into(),
            polyline: vec![(2.0, 2.49), (13.0, 2.49)],
        },
        SeededWalk {
            legacy_link_id: "link:walk:corridor:south".into(),
            polyline: vec![(2.0, 3.51), (13.0, 3.51)],
        },
    ];

    let (graph, _, _) = build_graph_from_city_network(&simple_network(), &[], &walks);
    let north = graph.edge(
        graph
            .edge_by_legacy("link:walk:corridor:north")
            .expect("north sidewalk edge exists"),
    );
    let south = graph.edge(
        graph
            .edge_by_legacy("link:walk:corridor:south")
            .expect("south sidewalk edge exists"),
    );

    assert_eq!(north.polyline, vec![(2.0, 2.49), (13.0, 2.49)]);
    assert_eq!(south.polyline, vec![(2.0, 3.51), (13.0, 3.51)]);
    assert!(graph.nodes().iter().any(|node| node.position == (2.0, 2.49)));
    assert!(graph.nodes().iter().any(|node| node.position == (2.0, 3.51)));
}
```

- [ ] **Step 2: Run the failing routing test**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core builder_preserves_fractional_seeded_walk_nodes
```

Expected: FAIL because walk endpoint nodes are still rounded to integer coordinates.

- [ ] **Step 3: Add quantized graph-node keys**

In `backend/crates/sim-core/src/routing/builder.rs`, add these helpers near `polyline_length`:

```rust
type CoordKey = (i32, i32);
const COORD_KEY_SCALE: f32 = 1000.0;

fn coord_key(point: (f32, f32)) -> CoordKey {
    (
        (point.0 * COORD_KEY_SCALE).round() as i32,
        (point.1 * COORD_KEY_SCALE).round() as i32,
    )
}

fn remember_point(points: &mut HashMap<CoordKey, (f32, f32)>, point: (f32, f32)) -> CoordKey {
    let key = coord_key(point);
    points.entry(key).or_insert(point);
    key
}
```

- [ ] **Step 4: Replace rounded integer node collection**

Inside `build_graph_from_city_network`, use `CoordKey` for node identity and keep the original `(f32, f32)` for node positions:

```rust
let mut coord_use_count: HashMap<CoordKey, u32> = HashMap::new();
let mut point_by_key: HashMap<CoordKey, (f32, f32)> = HashMap::new();
let mut endpoint_coords: Vec<CoordKey> = Vec::new();
let mut polyline_coords: Vec<Vec<(f32, f32)>> = Vec::new();
let mut polyline_kinds: Vec<PolylineKind> = Vec::new();

for (idx, path) in network.arterial_paths.iter().enumerate() {
    let coords = path.iter().map(|point| (point.x, point.y)).collect::<Vec<_>>();
    if coords.is_empty() {
        continue;
    }
    endpoint_coords.push(remember_point(&mut point_by_key, coords[0]));
    endpoint_coords.push(remember_point(&mut point_by_key, *coords.last().unwrap()));
    for coord in &coords {
        let key = remember_point(&mut point_by_key, *coord);
        *coord_use_count.entry(key).or_insert(0) += 1;
    }
    polyline_coords.push(coords);
    polyline_kinds.push(PolylineKind::Arterial { index: idx });
}
```

For seeded walks:

```rust
let mut walk_coords: Vec<CoordKey> = Vec::new();
for walk in seeded_walks {
    if walk.polyline.len() < 2 {
        continue;
    }
    let from = remember_point(&mut point_by_key, *walk.polyline.first().unwrap());
    let to = remember_point(&mut point_by_key, *walk.polyline.last().unwrap());
    walk_coords.push(from);
    walk_coords.push(to);
}
```

For stops:

```rust
for stop in seeded_stops {
    is_node.insert(remember_point(&mut point_by_key, stop.coord), true);
}
```

When creating nodes:

```rust
let mut node_keys: Vec<CoordKey> = is_node.keys().copied().collect();
node_keys.sort();
let mut nodes: Vec<Node> = Vec::with_capacity(node_keys.len());
let mut node_id_by_coord: HashMap<CoordKey, NodeId> = HashMap::new();
for (idx, key) in node_keys.iter().enumerate() {
    let id = NodeId(idx as u32);
    node_id_by_coord.insert(*key, id);
    nodes.push(Node {
        id,
        position: point_by_key[key],
        kind: NodeKind::Intersection,
        legacy_id: None,
    });
}
```

In split and footway lookups, call `coord_key(point)` instead of rounding to `(i32, i32)`. For road legacy IDs, use the quantized key to keep stable strings:

```rust
let legacy_key = coord_key(segment[0]);
legacy_id: Some(format!("link:road:{index}:{}_{},fwd", legacy_key.0, legacy_key.1)),
```

- [ ] **Step 5: Run routing tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core routing::builder
```

Expected: PASS.

- [ ] **Step 6: Commit routing graph change**

Run:

```bash
git add backend/crates/sim-core/src/routing/builder.rs
git commit -m "fix(routing): preserve fractional footway geometry"
```

Expected: commit succeeds.

---

### Task 3: Author Real Abutopia Sidewalk Corridors

**Files:**
- Modify: `scripts/generate-abutopia-world.mjs`
- Modify generated: `data/worlds/abutopia/layers/transport.json`
- Modify generated: `data/worlds/abutopia/layers/spawns.json`
- Modify: `src/backend/baseWorldClient.ts`
- Modify: `tests/app/baseWorldBundle.test.ts`
- Modify: `tests/app/appRuntime.test.ts`

- [ ] **Step 1: Add failing frontend/base-world contract tests**

In `tests/app/baseWorldBundle.test.ts`, change the transport type and assertions:

```ts
type TransportPath = {
  id: string;
  points: { x: number; y: number }[];
};
```

Use this transport load type:

```ts
const transport = loadJson<{
  roads: unknown[];
  rails: unknown[];
  arterial_paths: unknown[];
  rail_paths: unknown[];
  pedestrian_corridors: TransportPath[];
}>(`data/worlds/abutopia/${manifest.layers.transport}`);
```

Replace the pedestrian corridor assertion with:

```ts
expect(transport.pedestrian_corridors.map((path) => path.id)).toEqual([
  'corridor:sidewalk:north',
  'corridor:sidewalk:south',
]);
expect(transport.pedestrian_corridors[0].points).toHaveLength(12);
expect(transport.pedestrian_corridors[1].points).toHaveLength(12);
expect(transport.pedestrian_corridors[0].points[0]).toEqual({ x: 2, y: 2.49 });
expect(transport.pedestrian_corridors[1].points[0]).toEqual({ x: 2, y: 3.51 });
expect(
  transport.pedestrian_corridors.flatMap((path) => path.points).some((point) => point.y === 3),
).toBe(false);
```

In `tests/app/appRuntime.test.ts`, update `createBaseWorld()` so the fixture contains both sidewalk corridors:

```ts
pedestrian_corridors: [
  { id: 'corridor:sidewalk:north', points: Array.from({ length: 12 }, (_, index) => ({ x: index + 2, y: 2.49 })) },
  { id: 'corridor:sidewalk:south', points: Array.from({ length: 12 }, (_, index) => ({ x: index + 2, y: 3.51 })) },
],
```

- [ ] **Step 2: Run failing JS tests**

Run:

```bash
npm test -- tests/app/baseWorldBundle.test.ts tests/app/appRuntime.test.ts
```

Expected: `baseWorldBundle` fails because generated Abutopia still has one `corridor:main` at `y: 3`.

- [ ] **Step 3: Generate north and south sidewalk corridors**

In `scripts/generate-abutopia-world.mjs`, replace the current `corridorPoints` block with:

```js
const sidewalkOffset = 0.51;
const sidewalkNorthY = Number((roadY - sidewalkOffset).toFixed(2));
const sidewalkSouthY = Number((roadY + sidewalkOffset).toFixed(2));

function corridorPointsFor(y) {
  const points = [];
  for (let x = houseAX; x <= houseBX; x += 1) points.push({ x, y });
  return points;
}
```

Replace `pedestrian_corridors` with:

```js
pedestrian_corridors: [
  { id: 'corridor:sidewalk:north', points: corridorPointsFor(sidewalkNorthY) },
  { id: 'corridor:sidewalk:south', points: corridorPointsFor(sidewalkSouthY) },
],
```

Replace the pedestrian spawn group with:

```js
pedestrian_groups: [
  { id: 'spawn:ped:sidewalk-south', corridor_id: 'corridor:sidewalk:south', agents_per_corridor: 1 },
],
```

- [ ] **Step 4: Regenerate Abutopia data**

Run:

```bash
npm run generate:abutopia
```

Expected: `data/worlds/abutopia/layers/transport.json` contains two sidewalk corridors and `data/worlds/abutopia/layers/spawns.json` references `corridor:sidewalk:south`.

- [ ] **Step 5: Update frontend base-world validation**

In `src/backend/baseWorldClient.ts`, replace:

```ts
if (payload.transport.pedestrian_corridors.length !== 1) throw new Error('Base world pedestrian layer is incomplete');
```

with:

```ts
if (payload.transport.pedestrian_corridors.length !== 2) throw new Error('Base world pedestrian layer is incomplete');
const pedestrianIds = payload.transport.pedestrian_corridors.map((path) => path.id);
if (!pedestrianIds.includes('corridor:sidewalk:north') || !pedestrianIds.includes('corridor:sidewalk:south')) {
  throw new Error('Base world pedestrian sidewalks are incomplete');
}
if (payload.transport.pedestrian_corridors.some((path) => path.points.some((point) => point.y === 3))) {
  throw new Error('Base world pedestrian sidewalks must not use the road centerline');
}
```

- [ ] **Step 6: Run base-world tests**

Run:

```bash
npm test -- tests/app/baseWorldBundle.test.ts tests/app/appRuntime.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit Abutopia sidewalk data**

Run:

```bash
git add scripts/generate-abutopia-world.mjs data/worlds/abutopia/layers/transport.json data/worlds/abutopia/layers/spawns.json src/backend/baseWorldClient.ts tests/app/baseWorldBundle.test.ts tests/app/appRuntime.test.ts
git commit -m "feat(world): author abutopia sidewalk corridors"
```

Expected: commit succeeds.

---

### Task 4: Seed and Verify Backend Pedestrians on Sidewalk Footways

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/seed.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Add failing seed tests**

In `backend/crates/sim-core/src/mobility/seed.rs`, add these tests inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn seeded_walks_from_network_preserve_fractional_sidewalk_geometry() {
    use crate::city_network::{CityNetwork, NetworkPoint, WorldTiles};

    let network = CityNetwork {
        version: 1,
        world_id: "test".into(),
        chunk_size: 32,
        world_tiles: WorldTiles {
            width: 16,
            height: 8,
        },
        arterial_paths: vec![],
        pedestrian_corridors: vec![
            vec![NetworkPoint { x: 2.0, y: 2.49 }, NetworkPoint { x: 13.0, y: 2.49 }],
            vec![NetworkPoint { x: 2.0, y: 3.51 }, NetworkPoint { x: 13.0, y: 3.51 }],
        ],
    };

    let walks = seeded_walks_from_network(&network);

    assert_eq!(walks.len(), 2);
    assert_eq!(walks[0].polyline, vec![(2.0, 2.49), (13.0, 2.49)]);
    assert_eq!(walks[1].polyline, vec![(2.0, 3.51), (13.0, 3.51)]);
}

#[test]
fn from_base_world_bundle_seeds_pedestrian_on_sidewalk_corridor() {
    use crate::ids::AgentId;
    use crate::mobility::components::Position;
    use crate::mobility::resources::AgentIdIndex;

    let bundle = crate::base_world::BaseWorldBundle::load_from_dir(
        workspace_root().join("data/worlds/abutopia"),
    )
    .expect("base world bundle should load");

    let (world, _) = from_base_world_bundle(&bundle).expect("base world should seed");
    let agents = crate::mobility::api::agents(&world);
    let agent = agents
        .iter()
        .find(|agent| agent.id == AgentId("agent:walk:0".into()))
        .expect("abutopia pedestrian is seeded");

    assert!(matches!(
        &agent.state,
        AgentMobilityState::Walking { link_id, .. } if link_id == "link:walk:corridor:1"
    ));

    let entity = *world
        .resource::<AgentIdIndex>()
        .0
        .get(&AgentId("agent:walk:0".into()))
        .expect("agent index contains spawned pedestrian");
    let position = world.entity(entity).get::<Position>().expect("agent has position");
    assert!((position.y - 3.51).abs() < 0.001);
}
```

- [ ] **Step 2: Update `seeded_walks_from_network` implementation**

In `backend/crates/sim-core/src/mobility/seed.rs`, replace the integer mapping in `seeded_walks_from_network` with:

```rust
let polyline: Vec<(f32, f32)> = corridor.iter().map(|point| (point.x, point.y)).collect();
```

Remove the now-unused local `use crate::city_network::NetworkCoord;`.

- [ ] **Step 3: Add runtime footway geometry test**

In `backend/crates/sim-server/src/runtime.rs`, add this test near `runtime_can_find_seeded_walk_path`:

```rust
#[test]
fn runtime_uses_sidewalk_footway_geometry_from_base_world() {
    let network = base_world_fixture().to_city_network();
    let runtime = SimulationRuntime::new_from_network(&network);
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let edge = graph.edge(
        graph
            .edge_by_legacy("link:walk:corridor:1")
            .expect("south sidewalk footway exists"),
    );

    assert_eq!(edge.kind, sim_core::routing::EdgeKind::Footway);
    assert_eq!(edge.polyline.first().copied(), Some((2.0, 3.51)));
    assert_eq!(edge.polyline.last().copied(), Some((13.0, 3.51)));
    assert!(edge.polyline.iter().all(|(_, y)| (*y - 3.0).abs() > 0.001));
}
```

- [ ] **Step 4: Run backend seed/runtime tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core seeded_walks_from_network_preserve_fractional_sidewalk_geometry
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core from_base_world_bundle_seeds_pedestrian_on_sidewalk_corridor
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server runtime_uses_sidewalk_footway_geometry_from_base_world
```

Expected: PASS.

- [ ] **Step 5: Commit backend sidewalk seeding**

Run:

```bash
git add backend/crates/sim-core/src/mobility/seed.rs backend/crates/sim-server/src/runtime.rs
git commit -m "test(mobility): verify pedestrians seed on sidewalks"
```

Expected: commit succeeds.

---

### Task 5: Render Pedestrians at Backend Sidewalk Coordinates

**Files:**
- Modify: `src/render/entityRenderStyle.ts`
- Modify: `tests/render/entityRenderStyle.test.ts`
- Modify: `tests/render/backendMobilityDrawables.test.ts`

- [ ] **Step 1: Add failing render-style tests**

In `tests/render/entityRenderStyle.test.ts`, replace the pedestrian lane test with:

```ts
it('does not add a default visual lane offset for backend pedestrians', () => {
  expect(pedestrianRenderStyle({ x: 0, y: 0 }, { x: 10, y: 0 }, 0.5, 0)).toEqual({
    lane: { x: 0, y: 0 },
    selectedRadius: 9,
    radius: 3.2,
  });
});

it('can still apply an explicit pedestrian lane offset', () => {
  expect(pedestrianRenderStyle({ x: 0, y: 0 }, { x: 10, y: 0 }, 0.5, 2)).toEqual({
    lane: { x: 0, y: 4 },
    selectedRadius: 9,
    radius: 3.2,
  });
});
```

In `tests/render/backendMobilityDrawables.test.ts`, add:

```ts
it('passes backend pedestrian sidewalk coordinates through without visual lane offset', () => {
  const state = makeStateWith(
    [
      {
        id: 'agent:walk:0',
        state: { type: 'walking', link_id: 'link:walk:corridor:1', progress: 0 },
        plan_cursor: 0,
        world_coord: { x: 2, y: 3.51 },
        direction: 'e',
        sprite_key: 'pedestrian:0',
      },
    ],
    [],
  );

  const pedestrians = pedestriansFromMobilityState(state, pedestrianSprites, 0, 100);

  expect(pedestrians).toHaveLength(1);
  expect(pedestrians[0].path[0]).toEqual({ x: 2, y: 3.51 });
  expect(pedestrians[0].laneOffset).toBe(0);
});
```

- [ ] **Step 2: Run failing render tests**

Run:

```bash
npm test -- tests/render/entityRenderStyle.test.ts tests/render/backendMobilityDrawables.test.ts
```

Expected: `entityRenderStyle.test.ts` fails because `laneOffset: 0` still produces a hard-coded lane offset.

- [ ] **Step 3: Remove default pedestrian lane nudge**

In `src/render/entityRenderStyle.ts`, replace `pedestrianRenderStyle` with:

```ts
export function pedestrianRenderStyle(
  currentPoint: Coord,
  nextPoint: Coord,
  cameraScale: number,
  laneOffset: number,
): PedestrianRenderStyle {
  const lanePixels = laneOffset <= 0
    ? 0
    : screenStableWorldSize(laneOffset, cameraScale, { minWorld: 0, maxWorld: 14 });
  return {
    lane: lanePixels === 0 ? { x: 0, y: 0 } : screenRightLaneOffset(currentPoint, nextPoint, lanePixels),
    selectedRadius: screenStableWorldSize(4.5, cameraScale, { minWorld: 3.2, maxWorld: 10 }),
    radius: screenStableWorldSize(1.6, cameraScale, { minWorld: 1.2, maxWorld: 3.2 }),
  };
}
```

- [ ] **Step 4: Run render tests**

Run:

```bash
npm test -- tests/render/entityRenderStyle.test.ts tests/render/backendMobilityDrawables.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit frontend render change**

Run:

```bash
git add src/render/entityRenderStyle.ts tests/render/entityRenderStyle.test.ts tests/render/backendMobilityDrawables.test.ts
git commit -m "fix(render): draw pedestrians at backend sidewalk coords"
```

Expected: commit succeeds.

---

### Task 6: Browser Smoke and Full Verification

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Add sidewalk assertion to browser smoke**

In `tests/e2e/render-smoke.spec.ts`, after the existing mobility agent object expectation block, add:

```ts
const agent = state.city.mobilityAgents.agents[0];
expect(agent.coord.x).toBeGreaterThanOrEqual(2);
expect(agent.coord.x).toBeLessThanOrEqual(13);
expect(agent.coord.y).toBeGreaterThan(3.45);
expect(agent.coord.y).toBeLessThan(3.57);
expect(agent.coord.y).not.toBe(3);
```

- [ ] **Step 2: Run TypeScript and unit tests**

Run:

```bash
npm run typecheck
npm test
npm run build
```

Expected: all commands exit 0.

- [ ] **Step 3: Run Rust formatting, clippy, and tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml -- --check
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
```

Expected: all commands exit 0.

- [ ] **Step 4: Stop existing local dev servers before Playwright**

Run:

```bash
for port in 8080 5173; do
  pids=$(lsof -ti tcp:$port -sTCP:LISTEN || true)
  if [ -n "$pids" ]; then kill $pids; fi
done
```

Expected: ports `8080` and `5173` are free for Playwright web servers.

- [ ] **Step 5: Run mandatory browser smoke**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target npx playwright test tests/e2e/render-smoke.spec.ts --project=chromium
```

Expected: PASS. The smoke must confirm one backend-driven pedestrian, no retired transit assets, no rail/tram diagnostics, and `coord.y` in the sidewalk band around `3.51`.

- [ ] **Step 6: Commit e2e assertion**

Run:

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test(e2e): assert pedestrians render on sidewalks"
```

Expected: commit succeeds.

- [ ] **Step 7: Final branch status**

Run:

```bash
git status --short --branch
git log --oneline origin/main..HEAD
```

Expected: worktree is clean and the branch contains the sidewalk commits from Tasks 1 through 6.

---

### Task 7: Publish for Review

**Files:**
- No source changes.

- [ ] **Step 1: Push the branch**

Run:

```bash
git push -u origin codex/sidewalk-footway-simulation
```

Expected: push succeeds.

- [ ] **Step 2: Prepare PR summary**

Use this PR body:

```markdown
## Summary
- add floating-point transport path coordinates for backend movement geometry
- author Abutopia north/south sidewalk corridors while preserving roads and buildings
- seed and render pedestrians at backend sidewalk coordinates
- add browser smoke coverage for sidewalk-positioned backend pedestrians

## Verification
- npm run typecheck
- npm test
- npm run build
- CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml -- --check
- CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
- CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
- CARGO_TARGET_DIR=/tmp/abutown-sidewalk-target npx playwright test tests/e2e/render-smoke.spec.ts --project=chromium
```

Expected: summary clearly states that roads/buildings stay intact and only pedestrian footway geometry changes.
