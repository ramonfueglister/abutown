# Base World Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current split demo world with one canonical versioned Base World Bundle for `zurich-river-city-v1`, remove production fallbacks and seeded demo chunks, and make the backend and frontend consume the same authored world data.

**Architecture:** Generate a versioned bundle under `data/worlds/zurich-river-city-v1/`, load and validate it in Rust at startup, materialize simulation chunks from it, derive routing and initial mobility from its transport/spawn layers, expose the loaded render layers through the backend, and make the frontend render only backend/bundle-derived layers.

**Tech Stack:** Rust `sim-core` + `sim-server`, Bevy ECS resources, Axum routes, JSON bundle files with serde validation, TypeScript/Vite frontend, Vitest, Playwright render smoke tests, existing `tsx` tooling for generation.

---

## Progress

- [ ] Task 1: Add failing cutover guards
- [ ] Task 2: Add Base World Bundle schema and loader in `sim-core`
- [ ] Task 3: Generate the canonical `zurich-river-city-v1` bundle
- [ ] Task 4: Materialize backend chunks from the bundle
- [ ] Task 5: Route and seed mobility from bundle data only
- [ ] Task 6: Serve bundle-derived render layers to the frontend
- [ ] Task 7: Remove retired/demo production paths
- [ ] Task 8: Add snapshot compatibility metadata
- [ ] Task 9: Run full verification and commit in reviewable slices

---

## Current State To Preserve

The frontend already has a richer Mini-Metro-style Zurich world in `src/city/zurichWorld.ts` and its surrounding builders. The first cutover does not discard that authored shape; it moves it out of runtime authority and into a generated static bundle.

Known current counts from existing tests:

- Roads: `3396`
- Rails: `256`
- Buildings: `2268`
- Trees: `4325`
- Network fixture: `data/city/zurich-network.json`
- Network fixture world id: `zurich-river-city-v1`
- Network fixture world size: `256 x 256`, chunk size `32`
- Network fixture arterials: `3`
- Network fixture pedestrian corridors: `160`

The backend currently uses `WORLD_ID = "abutown-main"` and materializes only three seeded chunks. The cutover changes backend world identity to `zurich-river-city-v1`.

---

## Task 1: Add Failing Cutover Guards

**Purpose:** Lock the target behavior before touching production code. These tests intentionally fail on the current codebase.

### 1.1 Add frontend production authority guard

Create `tests/app/noDemoWorldAuthority.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const repoRoot = process.cwd();

function read(path: string): string {
  return readFileSync(join(repoRoot, path), "utf8");
}

describe("base world cutover guards", () => {
  it("does not use procedural Zurich builders as runtime map authority", () => {
    const runtimeEntrypoints = [
      "src/main.ts",
      "src/app/appRuntime.ts",
      "src/app/zurichRuntimeContext.ts",
    ];

    const forbidden = [
      "createZurichRuntimeContext(",
      "buildZurichWorld(",
      "buildZurichTransport(",
      "buildZurichPlacement(",
    ];

    const hits = runtimeEntrypoints.flatMap((file) => {
      const source = read(file);
      return forbidden
        .filter((pattern) => source.includes(pattern))
        .map((pattern) => `${file}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });

  it("does not reference retired pak or simutrans assets from runtime code", () => {
    const runtimeFiles = [
      "src/main.ts",
      "src/render/minimalMap.ts",
      "src/app/appRuntime.ts",
    ];

    const forbidden = [/pak128/i, /simutrans/i, /opengfx/i];

    const hits = runtimeFiles.flatMap((file) => {
      const source = read(file);
      return forbidden
        .filter((pattern) => pattern.test(source))
        .map((pattern) => `${file}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });
});
```

Expected result before implementation:

```bash
npm test -- tests/app/noDemoWorldAuthority.test.ts
```

The first test fails because `src/main.ts` still imports and calls `createZurichRuntimeContext`.

### 1.2 Add backend production fallback guard

Add this unit test near the existing runtime tests in `backend/crates/sim-server/src/runtime.rs`:

```rust
#[test]
fn runtime_materializes_base_world_instead_of_demo_chunks() {
    let fixture_root = workspace_root().join("data/worlds/zurich-river-city-v1");
    let runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
        .expect("base world fixture must load");
    let summary = runtime.summary();

    assert_eq!(summary.world_id, "zurich-river-city-v1");
    assert_eq!(summary.chunk_size, 32);
    assert!(
        summary.loaded_chunks.len() > 3,
        "base world must not be the old three seeded chunks"
    );
    assert!(
        summary.loaded_chunks.iter().any(|coord| coord.x == 4 && coord.y == 4),
        "central Zurich chunk remains available"
    );
}
```

Add this helper in the same test module if there is no shared workspace helper:

```rust
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-server crate lives under backend/crates/sim-server")
        .to_path_buf()
}
```

Expected result before implementation:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_materializes_base_world_instead_of_demo_chunks
```

The test fails to compile because `new_from_base_world_dir` does not exist yet.

### 1.3 Add production no-fallback grep guard

Create `tests/app/noProductionFallbacks.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const checks = [
  {
    file: "backend/crates/sim-server/src/runtime.rs",
    patterns: [
      "SEEDED_CHUNKS",
      "CityNetwork::empty_for_world",
      "tiny_world()",
      "TileKind::BuildingFootprint; }",
    ],
  },
  {
    file: "backend/crates/sim-server/src/app.rs",
    patterns: [
      "SimulationRuntime::new()",
      "Err(_) =>",
      "empty_for_world",
    ],
  },
  {
    file: "src/main.ts",
    patterns: [
      "createZurichRuntimeContext",
      "zurichContext.runtime",
    ],
  },
];

describe("production fallback removal", () => {
  it("does not keep demo world fallbacks in production entrypoints", () => {
    const hits = checks.flatMap(({ file, patterns }) => {
      const source = readFileSync(join(process.cwd(), file), "utf8");
      return patterns
        .filter((pattern) => source.includes(pattern))
        .map((pattern) => `${file}: ${pattern}`);
    });

    expect(hits).toEqual([]);
  });
});
```

Expected result before implementation:

```bash
npm test -- tests/app/noProductionFallbacks.test.ts
```

This fails on `SEEDED_CHUNKS`, `CityNetwork::empty_for_world`, and the frontend procedural runtime context.

---

## Task 2: Add Base World Bundle Schema And Loader In `sim-core`

**Purpose:** Create a strict data boundary for authored world state. Missing files, unknown schema versions, invalid bounds, and empty required layers return errors.

### 2.1 Add module export

Edit `backend/crates/sim-core/src/lib.rs`:

```rust
pub mod base_world;
```

### 2.2 Add `backend/crates/sim-core/src/base_world.rs`

Implement the data model with serde derives:

```rust
use crate::city_network::{
    ArterialPath, CityNetwork, PedestrianCorridor, Point, WorldTiles,
};
use crate::world::{ChunkCoord, TileKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum BaseWorldError {
    #[error("base world manifest missing at {0}")]
    MissingManifest(PathBuf),
    #[error("failed to read base world file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse base world file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported base world schema version {0}")]
    UnsupportedSchema(u32),
    #[error("base world id mismatch: manifest has {manifest}, layer has {layer}")]
    WorldIdMismatch { manifest: String, layer: String },
    #[error("base world layer {0} is empty")]
    EmptyLayer(&'static str),
    #[error("base world coordinate {x},{y} is outside {width}x{height}")]
    OutOfBounds {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BaseWorldManifest {
    pub schema_version: u32,
    pub world_id: String,
    pub display_name: String,
    pub chunk_size: u32,
    pub world_tiles: WorldTiles,
    pub layers: BaseWorldLayerFiles,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BaseWorldLayerFiles {
    pub terrain: String,
    pub transport: String,
    pub buildings: String,
    pub spawns: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TerrainLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub tiles: Vec<TerrainTile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TerrainTile {
    pub x: u32,
    pub y: u32,
    pub kind: TerrainKind,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TerrainKind {
    Grass,
    Water,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub roads: Vec<TransportPath>,
    pub rails: Vec<TransportPath>,
    pub pedestrian_corridors: Vec<TransportPath>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportPath {
    pub id: String,
    pub points: Vec<Point>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildingLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub footprints: Vec<BuildingFootprint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildingFootprint {
    pub id: String,
    pub tiles: Vec<Point>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpawnLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub pedestrian_groups: Vec<PedestrianSpawnGroup>,
    pub car_groups: Vec<CarSpawnGroup>,
    pub tram_lines: Vec<TramLineSpawn>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PedestrianSpawnGroup {
    pub id: String,
    pub corridor_id: String,
    pub agents_per_corridor: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CarSpawnGroup {
    pub id: String,
    pub arterial_id: String,
    pub cars_per_arterial: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TramLineSpawn {
    pub id: String,
    pub rail_path_ids: Vec<String>,
    pub trams: u32,
}

#[derive(Debug, Clone)]
pub struct BaseWorldBundle {
    pub manifest: BaseWorldManifest,
    pub terrain: TerrainLayer,
    pub transport: TransportLayer,
    pub buildings: BuildingLayer,
    pub spawns: SpawnLayer,
}
```

Add loader helpers:

```rust
impl BaseWorldBundle {
    pub fn load_from_dir(root: impl AsRef<Path>) -> Result<Self, BaseWorldError> {
        let root = root.as_ref();
        let manifest_path = root.join("manifest.json");
        if !manifest_path.exists() {
            return Err(BaseWorldError::MissingManifest(manifest_path));
        }

        let manifest: BaseWorldManifest = read_json(&manifest_path)?;
        let terrain: TerrainLayer = read_json(&root.join(&manifest.layers.terrain))?;
        let transport: TransportLayer = read_json(&root.join(&manifest.layers.transport))?;
        let buildings: BuildingLayer = read_json(&root.join(&manifest.layers.buildings))?;
        let spawns: SpawnLayer = read_json(&root.join(&manifest.layers.spawns))?;

        let bundle = Self {
            manifest,
            terrain,
            transport,
            buildings,
            spawns,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn validate(&self) -> Result<(), BaseWorldError> {
        validate_schema(self.manifest.schema_version)?;
        validate_schema(self.terrain.schema_version)?;
        validate_schema(self.transport.schema_version)?;
        validate_schema(self.buildings.schema_version)?;
        validate_schema(self.spawns.schema_version)?;

        for layer_world_id in [
            &self.terrain.world_id,
            &self.transport.world_id,
            &self.buildings.world_id,
            &self.spawns.world_id,
        ] {
            if layer_world_id != &self.manifest.world_id {
                return Err(BaseWorldError::WorldIdMismatch {
                    manifest: self.manifest.world_id.clone(),
                    layer: layer_world_id.clone(),
                });
            }
        }

        if self.transport.roads.is_empty() {
            return Err(BaseWorldError::EmptyLayer("transport.roads"));
        }
        if self.transport.pedestrian_corridors.is_empty() {
            return Err(BaseWorldError::EmptyLayer("transport.pedestrian_corridors"));
        }
        if self.buildings.footprints.is_empty() {
            return Err(BaseWorldError::EmptyLayer("buildings.footprints"));
        }

        self.validate_points()?;
        Ok(())
    }

    pub fn world_id(&self) -> &str {
        &self.manifest.world_id
    }

    pub fn chunk_size(&self) -> u32 {
        self.manifest.chunk_size
    }

    pub fn world_tiles(&self) -> WorldTiles {
        self.manifest.world_tiles
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, BaseWorldError> {
    let bytes = fs::read(path).map_err(|source| BaseWorldError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| BaseWorldError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn validate_schema(version: u32) -> Result<(), BaseWorldError> {
    if version == SUPPORTED_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(BaseWorldError::UnsupportedSchema(version))
    }
}
```

Add point validation and CityNetwork bridge:

```rust
impl BaseWorldBundle {
    fn validate_points(&self) -> Result<(), BaseWorldError> {
        let width = self.manifest.world_tiles.width;
        let height = self.manifest.world_tiles.height;

        for tile in &self.terrain.tiles {
            self.require_in_bounds(tile.x, tile.y)?;
        }
        for path in self
            .transport
            .roads
            .iter()
            .chain(self.transport.rails.iter())
            .chain(self.transport.pedestrian_corridors.iter())
        {
            for point in &path.points {
                self.require_in_bounds(point.x, point.y)?;
            }
        }
        for footprint in &self.buildings.footprints {
            for point in &footprint.tiles {
                self.require_in_bounds(point.x, point.y)?;
            }
        }

        if width == 0 || height == 0 || self.manifest.chunk_size == 0 {
            return Err(BaseWorldError::OutOfBounds {
                x: width,
                y: height,
                width,
                height,
            });
        }
        Ok(())
    }

    fn require_in_bounds(&self, x: u32, y: u32) -> Result<(), BaseWorldError> {
        let width = self.manifest.world_tiles.width;
        let height = self.manifest.world_tiles.height;
        if x < width && y < height {
            Ok(())
        } else {
            Err(BaseWorldError::OutOfBounds { x, y, width, height })
        }
    }

    pub fn to_city_network(&self) -> CityNetwork {
        CityNetwork {
            version: self.manifest.schema_version,
            world_id: self.manifest.world_id.clone(),
            chunk_size: self.manifest.chunk_size,
            world_tiles: self.manifest.world_tiles,
            arterial_paths: self
                .transport
                .roads
                .iter()
                .map(|path| ArterialPath {
                    id: path.id.clone(),
                    points: path.points.clone(),
                })
                .collect(),
            pedestrian_corridors: self
                .transport
                .pedestrian_corridors
                .iter()
                .map(|path| PedestrianCorridor {
                    id: path.id.clone(),
                    points: path.points.clone(),
                })
                .collect(),
        }
    }
}
```

Add tile helpers:

```rust
impl BaseWorldBundle {
    pub fn chunk_coords(&self) -> Vec<ChunkCoord> {
        let chunk_size = self.manifest.chunk_size;
        let chunks_x = self.manifest.world_tiles.width.div_ceil(chunk_size);
        let chunks_y = self.manifest.world_tiles.height.div_ceil(chunk_size);

        (0..chunks_y)
            .flat_map(|y| (0..chunks_x).map(move |x| ChunkCoord { x, y }))
            .collect()
    }

    pub fn tile_kind_at(&self, x: u32, y: u32) -> TileKind {
        let terrain_water = self
            .terrain
            .tiles
            .iter()
            .any(|tile| tile.x == x && tile.y == y && tile.kind == TerrainKind::Water);
        if terrain_water {
            return TileKind::Water;
        }

        let road = self
            .transport
            .roads
            .iter()
            .any(|path| path.points.iter().any(|point| point.x == x && point.y == y));
        if road {
            return TileKind::Road;
        }

        let building = self
            .buildings
            .footprints
            .iter()
            .any(|footprint| footprint.tiles.iter().any(|point| point.x == x && point.y == y));
        if building {
            return TileKind::BuildingFootprint;
        }

        TileKind::Grass
    }

    pub fn occupied_tiles(&self) -> BTreeSet<(u32, u32)> {
        let mut occupied = BTreeSet::new();
        for tile in &self.terrain.tiles {
            occupied.insert((tile.x, tile.y));
        }
        for path in self
            .transport
            .roads
            .iter()
            .chain(self.transport.rails.iter())
            .chain(self.transport.pedestrian_corridors.iter())
        {
            for point in &path.points {
                occupied.insert((point.x, point.y));
            }
        }
        for footprint in &self.buildings.footprints {
            for point in &footprint.tiles {
                occupied.insert((point.x, point.y));
            }
        }
        occupied
    }
}
```

### 2.3 Add loader tests

Create `backend/crates/sim-core/tests/base_world_bundle.rs`:

```rust
use sim_core::base_world::{BaseWorldBundle, BaseWorldError};
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-core crate lives under backend/crates/sim-core")
        .join("data/worlds/zurich-river-city-v1")
}

#[test]
fn loads_zurich_base_world_fixture() {
    let bundle = BaseWorldBundle::load_from_dir(fixture_root()).expect("bundle loads");

    assert_eq!(bundle.world_id(), "zurich-river-city-v1");
    assert_eq!(bundle.chunk_size(), 32);
    assert_eq!(bundle.world_tiles().width, 256);
    assert_eq!(bundle.world_tiles().height, 256);
    assert!(bundle.transport.roads.len() >= 3);
    assert!(bundle.transport.pedestrian_corridors.len() >= 160);
    assert!(bundle.buildings.footprints.len() >= 2_200);
}

#[test]
fn missing_manifest_fails_closed() {
    let err = BaseWorldBundle::load_from_dir(fixture_root().join("missing"))
        .expect_err("missing manifest is fatal");

    assert!(matches!(err, BaseWorldError::MissingManifest(_)));
}
```

Expected result after Task 2 and before Task 3:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core base_world_bundle
```

The loader compiles. The fixture load test fails because the bundle files do not exist yet.

---

## Task 3: Generate The Canonical `zurich-river-city-v1` Bundle

**Purpose:** Promote the current authored Mini-Metro-style Zurich map into stable data. The TypeScript city builders stay as generation tooling, not runtime map authority.

### 3.1 Add package script

Edit `package.json`:

```json
{
  "scripts": {
    "generate:base-world": "node --import tsx/esm scripts/generate-base-world.mjs"
  }
}
```

Keep the existing scripts unchanged.

### 3.2 Add generator script

Create `scripts/generate-base-world.mjs`:

```js
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import {
  createZurichRuntimeContext,
} from "../src/app/zurichRuntimeContext.ts";

const worldId = "zurich-river-city-v1";
const schemaVersion = 1;
const chunkSize = 32;
const worldTiles = { width: 256, height: 256 };
const root = resolve("data/worlds", worldId);

function pointKey(point) {
  return `${point.x},${point.y}`;
}

function sortedPoints(points) {
  return [...points].sort((a, b) => a.y - b.y || a.x - b.x);
}

function tileFromKey(key) {
  const [x, y] = key.split(",").map((value) => Number.parseInt(value, 10));
  return { x, y };
}

function assertPoint(point, label) {
  if (
    !Number.isInteger(point.x) ||
    !Number.isInteger(point.y) ||
    point.x < 0 ||
    point.y < 0 ||
    point.x >= worldTiles.width ||
    point.y >= worldTiles.height
  ) {
    throw new Error(`${label} point out of bounds: ${JSON.stringify(point)}`);
  }
}

function toPath(id, points) {
  const deduped = new Map();
  for (const point of points) {
    assertPoint(point, id);
    deduped.set(pointKey(point), { x: point.x, y: point.y });
  }
  return { id, points: sortedPoints(deduped.values()) };
}

function singleTileFootprints(tiles) {
  return sortedPoints(tiles).map((tile, index) => ({
    id: `building:${String(index).padStart(5, "0")}`,
    tiles: [{ x: tile.x, y: tile.y }],
  }));
}

async function writeJson(relativePath, value) {
  const file = resolve(root, relativePath);
  await mkdir(dirname(file), { recursive: true });
  await writeFile(`${file}.tmp`, `${JSON.stringify(value, null, 2)}\n`);
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
}

const context = createZurichRuntimeContext({ seed: 1848 });
const runtime = context.runtime;

const roadTiles = sortedPoints(runtime.roads);
const railTiles = sortedPoints(runtime.rails);
const waterTiles = sortedPoints(runtime.terrain.water);
const buildingTiles = sortedPoints(runtime.buildings);

const pedestrianCorridors = context.network.pedestrian_corridors.map((corridor, index) =>
  toPath(corridor.id ?? `pedestrian:${index}`, corridor.points),
);

const arterialPaths = context.network.arterial_paths.map((arterial, index) =>
  toPath(arterial.id ?? `arterial:${index}`, arterial.points),
);

const terrain = {
  schema_version: schemaVersion,
  world_id: worldId,
  tiles: waterTiles.map((tile) => ({ ...tile, kind: "water" })),
};

const transport = {
  schema_version: schemaVersion,
  world_id: worldId,
  roads: arterialPaths.length > 0 ? arterialPaths : [toPath("road:all", roadTiles)],
  rails: [toPath("rail:all", railTiles)],
  pedestrian_corridors: pedestrianCorridors,
};

const buildings = {
  schema_version: schemaVersion,
  world_id: worldId,
  footprints: singleTileFootprints(buildingTiles),
};

const spawns = {
  schema_version: schemaVersion,
  world_id: worldId,
  pedestrian_groups: pedestrianCorridors.map((corridor) => ({
    id: `spawn:ped:${corridor.id}`,
    corridor_id: corridor.id,
    agents_per_corridor: 6,
  })),
  car_groups: arterialPaths.map((arterial) => ({
    id: `spawn:car:${arterial.id}`,
    arterial_id: arterial.id,
    cars_per_arterial: 17,
  })),
  tram_lines: [
    {
      id: "tram:lake-loop",
      rail_path_ids: ["rail:all"],
      trams: 4,
    },
  ],
};

const manifest = {
  schema_version: schemaVersion,
  world_id: worldId,
  display_name: "Zurich River City",
  chunk_size: chunkSize,
  world_tiles: worldTiles,
  layers: {
    terrain: "layers/terrain.json",
    transport: "layers/transport.json",
    buildings: "layers/buildings.json",
    spawns: "layers/spawns.json",
  },
};

await writeJson("manifest.json", manifest);
await writeJson("layers/terrain.json", terrain);
await writeJson("layers/transport.json", transport);
await writeJson("layers/buildings.json", buildings);
await writeJson("layers/spawns.json", spawns);

console.log(
  JSON.stringify(
    {
      worldId,
      roads: roadTiles.length,
      rails: railTiles.length,
      water: waterTiles.length,
      buildings: buildingTiles.length,
      pedestrianCorridors: pedestrianCorridors.length,
      arterials: arterialPaths.length,
    },
    null,
    2,
  ),
);
```

After the first run, remove the `.tmp` write if it creates unused files. The final script writes only the target JSON files.

### 3.3 Generate bundle

Run:

```bash
npm run generate:base-world
```

Expected output includes:

```json
{
  "worldId": "zurich-river-city-v1",
  "roads": 3396,
  "rails": 256,
  "buildings": 2268,
  "pedestrianCorridors": 160,
  "arterials": 3
}
```

### 3.4 Add bundle tests

Create `tests/app/baseWorldBundle.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

function loadJson<T>(path: string): T {
  return JSON.parse(readFileSync(join(process.cwd(), path), "utf8")) as T;
}

describe("generated base world bundle", () => {
  it("contains the authored Zurich layers", () => {
    const manifest = loadJson<{
      world_id: string;
      chunk_size: number;
      world_tiles: { width: number; height: number };
      layers: Record<string, string>;
    }>("data/worlds/zurich-river-city-v1/manifest.json");
    const terrain = loadJson<{ tiles: unknown[] }>(
      `data/worlds/zurich-river-city-v1/${manifest.layers.terrain}`,
    );
    const transport = loadJson<{
      roads: unknown[];
      rails: unknown[];
      pedestrian_corridors: unknown[];
    }>(`data/worlds/zurich-river-city-v1/${manifest.layers.transport}`);
    const buildings = loadJson<{ footprints: unknown[] }>(
      `data/worlds/zurich-river-city-v1/${manifest.layers.buildings}`,
    );

    expect(manifest.world_id).toBe("zurich-river-city-v1");
    expect(manifest.chunk_size).toBe(32);
    expect(manifest.world_tiles).toEqual({ width: 256, height: 256 });
    expect(terrain.tiles.length).toBeGreaterThan(0);
    expect(transport.roads.length).toBe(3);
    expect(transport.rails.length).toBe(1);
    expect(transport.pedestrian_corridors.length).toBe(160);
    expect(buildings.footprints.length).toBeGreaterThanOrEqual(2268);
  });
});
```

Run:

```bash
npm test -- tests/app/baseWorldBundle.test.ts
cargo test --manifest-path backend/Cargo.toml -p sim-core base_world_bundle
```

Expected result after Task 3: both pass.

Commit slice:

```bash
git add package.json scripts/generate-base-world.mjs data/worlds/zurich-river-city-v1 tests/app/baseWorldBundle.test.ts backend/crates/sim-core/src/lib.rs backend/crates/sim-core/src/base_world.rs backend/crates/sim-core/tests/base_world_bundle.rs
git commit -m "Add canonical base world bundle"
```

---

## Task 4: Materialize Backend Chunks From The Bundle

**Purpose:** Replace the three seeded demo chunks with deterministic chunks derived from the bundle.

### 4.1 Add materializer in `sim-core`

Add to `backend/crates/sim-core/src/base_world.rs`:

```rust
use crate::world::systems::spawn_chunk_entity;
use crate::world::{ChunkActivity, ChunkCoord, LocalTileIndex, TileData};
use bevy_ecs::world::World;

impl BaseWorldBundle {
    pub fn spawn_all_chunks(&self, world: &mut World, initial_version: u64) {
        let chunk_size = self.manifest.chunk_size;

        for coord in self.chunk_coords() {
            let tiles = self.tiles_for_chunk(coord);
            spawn_chunk_entity(
                world,
                coord,
                chunk_size,
                tiles,
                initial_version,
                ChunkActivity::Warm,
            );
        }
    }

    pub fn tiles_for_chunk(&self, coord: ChunkCoord) -> Vec<TileData> {
        let chunk_size = self.manifest.chunk_size;
        let mut tiles = Vec::with_capacity((chunk_size * chunk_size) as usize);

        for local_y in 0..chunk_size {
            for local_x in 0..chunk_size {
                let x = coord.x * chunk_size + local_x;
                let y = coord.y * chunk_size + local_y;
                let local_index = LocalTileIndex(local_y * chunk_size + local_x);

                let kind = if x < self.manifest.world_tiles.width
                    && y < self.manifest.world_tiles.height
                {
                    self.tile_kind_at(x, y)
                } else {
                    TileKind::Grass
                };

                tiles.push(TileData { local_index, kind });
            }
        }

        tiles
    }
}
```

If `TileData` fields are private in the current code, add a constructor in the existing world module instead of exposing fields:

```rust
impl TileData {
    pub fn new(local_index: LocalTileIndex, kind: TileKind) -> Self {
        Self { local_index, kind }
    }
}
```

Use the existing local struct shape from `src/world`.

### 4.2 Add runtime constructor

Edit `backend/crates/sim-server/src/runtime.rs`:

```rust
use sim_core::base_world::{BaseWorldBundle, BaseWorldError};
```

Add constructors:

```rust
impl SimulationRuntime {
    pub fn new_from_base_world_dir(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let bundle = BaseWorldBundle::load_from_dir(path)?;
        Self::new_from_base_world(bundle)
    }

    pub fn new_from_base_world(bundle: BaseWorldBundle) -> anyhow::Result<Self> {
        Self::new_with_event_store_and_base_world(
            Arc::new(InMemoryEventStore::default()),
            bundle,
        )
    }

    pub fn new_with_event_store_and_base_world(
        event_store: Arc<dyn EventStore>,
        bundle: BaseWorldBundle,
    ) -> anyhow::Result<Self> {
        let network = bundle.to_city_network();
        let mut app = App::new();

        app.add_plugins((
            CorePlugin::default(),
            RoutingPlugin {
                seeded_stops: seeded_stops_from_base_world(&bundle),
                seeded_walks: seeded_walks_from_base_world(&bundle),
            },
            PathfindingPlugin::default(),
            HpaPlugin::default(),
            FlowFieldPlugin::default(),
            MobilityPlugin::default(),
            PersistencePlugin::default(),
        ));

        app.insert_resource(EventStoreResource(event_store.clone()));
        app.insert_resource(network.clone());
        bundle.spawn_all_chunks(app.world_mut(), 0);

        let snapshot = seeded_mobility_snapshot_for_base_world(&bundle)
            .expect("validated base world must produce mobility snapshot");
        hydrate_from_stores(app.world_mut(), event_store.as_ref(), &bundle, snapshot);

        Ok(Self {
            world_id: bundle.world_id().to_owned(),
            app: Arc::new(Mutex::new(app)),
            command_log: Arc::new(Mutex::new(Vec::new())),
            event_store,
        })
    }
}
```

Adjust the exact constructor code to match existing `SimulationRuntime` fields and plugin order.

### 4.3 Remove demo chunk seeding from production constructor

Delete from `backend/crates/sim-server/src/runtime.rs`:

- `WORLD_ID = "abutown-main"` production usage
- `SEEDED_CHUNKS`
- `SEED_DENSITY` as a production constant
- the loop that creates chunks with offset `Road`, `Water`, `BuildingFootprint`
- the `CityNetwork::empty_for_world("abutown-main")` branch

Keep test-only utilities under `#[cfg(test)]` if existing unit tests still need small fixtures.

### 4.4 Update app startup to fail closed

Edit `backend/crates/sim-server/src/app.rs`:

Add:

```rust
const BASE_WORLD_DEFAULT_PATH: &str = "data/worlds/zurich-river-city-v1";

fn resolve_base_world_path() -> PathBuf {
    std::env::var("ABUTOWN_BASE_WORLD_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(BASE_WORLD_DEFAULT_PATH))
}
```

Change `build_app()`:

```rust
pub fn build_app() -> Router {
    let runtime = SimulationRuntime::new_from_base_world_dir(resolve_base_world_path())
        .expect("base world bundle is required for app startup");
    build_app_from_runtime(runtime)
}
```

Change `build_app_from_config(config: AppConfig)` to load the same bundle and propagate the error:

```rust
let bundle = BaseWorldBundle::load_from_dir(resolve_base_world_path())?;
let runtime = SimulationRuntime::new_with_event_store_and_base_world(event_store, bundle)?;
```

Remove the branch:

```rust
Err(_) => SimulationRuntime::new(),
```

### 4.5 Update backend expectations

Update tests that assert backend world id:

- Replace `"abutown-main"` with `"zurich-river-city-v1"`.
- Replace tests that expect exactly three loaded chunks with assertions on `> 3` or the full chunk grid count when stable.

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core base_world_bundle
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_materializes_base_world_instead_of_demo_chunks
cargo test --manifest-path backend/Cargo.toml -p sim-server
```

Expected result after Task 4: backend tests pass and app startup fails if the bundle is missing.

Commit slice:

```bash
git add backend/crates/sim-core backend/crates/sim-server
git commit -m "Load simulation chunks from base world"
```

---

## Task 5: Route And Seed Mobility From Bundle Data Only

**Purpose:** Remove production dependency on tiny/demo mobility seeds and derive pedestrians, cars, and trams from bundle spawn layers.

### 5.1 Add base-world routing adapters

Create `backend/crates/sim-server/src/base_world_adapters.rs`:

```rust
use sim_core::base_world::BaseWorldBundle;
use sim_core::routing::builder::{SeededStop, SeededWalk};

pub fn seeded_walks_from_base_world(bundle: &BaseWorldBundle) -> Vec<SeededWalk> {
    bundle
        .transport
        .pedestrian_corridors
        .iter()
        .map(|corridor| SeededWalk {
            legacy_link_id: corridor.id.clone(),
            polyline: corridor.points.clone(),
        })
        .collect()
}

pub fn seeded_stops_from_base_world(bundle: &BaseWorldBundle) -> Vec<SeededStop> {
    bundle
        .transport
        .rails
        .iter()
        .enumerate()
        .flat_map(|(rail_index, rail)| {
            let first = rail.points.first().cloned();
            let middle = rail.points.get(rail.points.len() / 2).cloned();
            let last = rail.points.last().cloned();
            [first, middle, last]
                .into_iter()
                .flatten()
                .enumerate()
                .map(move |(stop_index, point)| SeededStop {
                    id: format!("stop:rail:{rail_index}:{stop_index}"),
                    name: format!("Rail Stop {rail_index}-{stop_index}"),
                    position: point,
                })
        })
        .collect()
}
```

Adjust field names to match the current `SeededStop` and `SeededWalk` definitions.

Register the module in `backend/crates/sim-server/src/lib.rs` or `main.rs` module tree according to existing structure:

```rust
mod base_world_adapters;
```

### 5.2 Add base-world mobility seed function

Edit `backend/crates/sim-core/src/mobility/seed.rs`:

Add:

```rust
use crate::base_world::BaseWorldBundle;

pub fn from_base_world(bundle: &BaseWorldBundle) -> Result<MobilitySnapshot, MobilitySeedError> {
    let network = bundle.to_city_network();

    let pedestrian_density = bundle
        .spawns
        .pedestrian_groups
        .first()
        .map(|group| group.agents_per_corridor)
        .unwrap_or(0);
    let car_density = bundle
        .spawns
        .car_groups
        .first()
        .map(|group| group.cars_per_arterial)
        .unwrap_or(0);
    let tram_count = bundle
        .spawns
        .tram_lines
        .iter()
        .map(|line| line.trams)
        .sum();

    from_network(
        &network,
        SeedDensity {
            pedestrians_per_corridor: pedestrian_density,
            cars_per_arterial: car_density,
            trams_total: tram_count,
        },
    )
}
```

If `from_network` returns a snapshot directly, keep the return type aligned with the current implementation.

### 5.3 Remove production tiny-world branch

Edit `backend/crates/sim-server/src/runtime.rs`.

Replace:

```rust
seeded_mobility_snapshot_for_network(network)
```

with:

```rust
seeded_mobility_snapshot_for_base_world(&bundle)
```

Implement:

```rust
fn seeded_mobility_snapshot_for_base_world(
    bundle: &BaseWorldBundle,
) -> anyhow::Result<MobilityPersistSnapshot> {
    let snapshot = sim_core::mobility::seed::from_base_world(bundle)?;
    Ok(MobilityPersistSnapshot::from_runtime_snapshot(
        bundle.world_id(),
        snapshot,
    ))
}
```

Adjust constructor arguments to the actual snapshot types. The key behavior is:

- Missing persisted mobility snapshot creates a fresh seed from the base world.
- Persisted mobility snapshot with mismatched base world id is rejected and replaced with a fresh seed from the base world.
- Empty city network never creates `tiny_world()` in production.

Move `tiny_world()` and `legacy_seeded_*()` behind test-only usage if existing unit tests still need them. Keep the existing function bodies unchanged and add `#[cfg(test)]` to the function declarations:

```rust
#[cfg(test)]
pub fn tiny_world() -> MobilitySnapshot
```

For `legacy_seeded_walks` and `legacy_seeded_stops`, either delete production use or rename to `test_seeded_walks` and `test_seeded_stops` under `#[cfg(test)]`.

### 5.4 Update mobility tests

Add a test in `backend/crates/sim-core/tests/base_world_bundle.rs` or a new `backend/crates/sim-core/tests/base_world_mobility.rs`:

```rust
use sim_core::base_world::BaseWorldBundle;
use sim_core::mobility::seed::from_base_world;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-core crate lives under backend/crates/sim-core")
        .join("data/worlds/zurich-river-city-v1")
}

#[test]
fn seeds_mobility_from_base_world_spawns() {
    let bundle = BaseWorldBundle::load_from_dir(fixture_root()).expect("bundle loads");
    let snapshot = from_base_world(&bundle).expect("base world mobility seed");

    assert!(snapshot.agents.len() >= 900);
    assert!(snapshot.vehicles.len() >= 50);
    assert!(snapshot.transit_vehicles.len() >= 4);
}
```

Use the current snapshot field names.

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core base_world_mobility
cargo test --manifest-path backend/Cargo.toml -p sim-server mobility
npm test -- tests/app/noProductionFallbacks.test.ts
```

Expected result after Task 5: no production `tiny_world()` branch remains and mobility still moves in browser smoke.

Commit slice:

```bash
git add backend/crates/sim-core backend/crates/sim-server tests/app/noProductionFallbacks.test.ts
git commit -m "Seed mobility from base world"
```

---

## Task 6: Serve Bundle-Derived Render Layers To The Frontend

**Purpose:** Stop the frontend from building a separate runtime world. The browser renders the same loaded base world that the backend validated.

### 6.1 Add backend response type

Edit `backend/crates/sim-server/src/app.rs` or add `backend/crates/sim-server/src/base_world_api.rs`:

```rust
use axum::Json;
use serde::Serialize;
use sim_core::base_world::BaseWorldBundle;

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldResponse {
    pub world_id: String,
    pub chunk_size: u32,
    pub world_tiles: WorldTilesResponse,
    pub terrain: TerrainRenderLayer,
    pub transport: TransportRenderLayer,
    pub buildings: BuildingRenderLayer,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldTilesResponse {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TerrainRenderLayer {
    pub water: Vec<PointResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransportRenderLayer {
    pub roads: Vec<PointResponse>,
    pub rails: Vec<PointResponse>,
    pub road_paths: Vec<PathResponse>,
    pub rail_paths: Vec<PathResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildingRenderLayer {
    pub footprints: Vec<Vec<PointResponse>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathResponse {
    pub id: String,
    pub points: Vec<PointResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PointResponse {
    pub x: u32,
    pub y: u32,
}
```

Add conversion:

```rust
impl From<&BaseWorldBundle> for BaseWorldResponse {
    fn from(bundle: &BaseWorldBundle) -> Self {
        Self {
            world_id: bundle.world_id().to_owned(),
            chunk_size: bundle.chunk_size(),
            world_tiles: WorldTilesResponse {
                width: bundle.world_tiles().width,
                height: bundle.world_tiles().height,
            },
            terrain: TerrainRenderLayer {
                water: bundle
                    .terrain
                    .tiles
                    .iter()
                    .filter(|tile| tile.kind == TerrainKind::Water)
                    .map(|tile| PointResponse { x: tile.x, y: tile.y })
                    .collect(),
            },
            transport: TransportRenderLayer {
                roads: flatten_paths(&bundle.transport.roads),
                rails: flatten_paths(&bundle.transport.rails),
                road_paths: to_paths(&bundle.transport.roads),
                rail_paths: to_paths(&bundle.transport.rails),
            },
            buildings: BuildingRenderLayer {
                footprints: bundle
                    .buildings
                    .footprints
                    .iter()
                    .map(|footprint| {
                        footprint
                            .tiles
                            .iter()
                            .map(|point| PointResponse { x: point.x, y: point.y })
                            .collect()
                    })
                    .collect(),
            },
        }
    }
}
```

Store this in app state:

```rust
#[derive(Clone)]
struct AppState {
    runtime: SimulationRuntime,
    read_view: Arc<RuntimeReadView>,
    base_world: Arc<BaseWorldResponse>,
}
```

When constructing the app, create the response from the same `BaseWorldBundle` used by the runtime. The cleanest path is to load the bundle in `build_app_from_config`, clone it into `SimulationRuntime::new_with_event_store_and_base_world`, and store `BaseWorldResponse::from(&bundle)` in state before moving.

Add route:

```rust
.route("/base-world", get(base_world_handler))
```

Handler:

```rust
async fn base_world_handler(State(state): State<AppState>) -> Json<BaseWorldResponse> {
    Json((*state.base_world).clone())
}
```

### 6.2 Add backend route test

Add to `backend/crates/sim-server/tests/http.rs` or existing HTTP test file:

```rust
#[tokio::test]
async fn base_world_endpoint_serves_canonical_layers() {
    let app = sim_server::app::build_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/base-world")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json["world_id"], "zurich-river-city-v1");
    assert!(json["transport"]["roads"].as_array().unwrap().len() > 1_800);
    assert_eq!(json["transport"]["rails"].as_array().unwrap().len(), 256);
    assert!(json["buildings"]["footprints"].as_array().unwrap().len() >= 2_268);
}
```

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server base_world_endpoint_serves_canonical_layers
```

### 6.3 Add frontend client

Create `src/backend/baseWorldClient.ts`:

```ts
import { getBackendBaseUrl } from "./backendConfig";

export type BaseWorldPoint = {
  readonly x: number;
  readonly y: number;
};

export type BaseWorldResponse = {
  readonly world_id: string;
  readonly chunk_size: number;
  readonly world_tiles: { readonly width: number; readonly height: number };
  readonly terrain: { readonly water: readonly BaseWorldPoint[] };
  readonly transport: {
    readonly roads: readonly BaseWorldPoint[];
    readonly rails: readonly BaseWorldPoint[];
    readonly road_paths: readonly { readonly id: string; readonly points: readonly BaseWorldPoint[] }[];
    readonly rail_paths: readonly { readonly id: string; readonly points: readonly BaseWorldPoint[] }[];
  };
  readonly buildings: { readonly footprints: readonly (readonly BaseWorldPoint[])[] };
};

export async function requireBaseWorld(): Promise<BaseWorldResponse> {
  const response = await fetch(`${getBackendBaseUrl()}/base-world`);
  if (!response.ok) {
    throw new Error(`base world request failed: ${response.status}`);
  }
  const payload = (await response.json()) as BaseWorldResponse;
  validateBaseWorld(payload);
  return payload;
}

function validateBaseWorld(payload: BaseWorldResponse): void {
  if (payload.world_id !== "zurich-river-city-v1") {
    throw new Error(`unexpected base world id: ${payload.world_id}`);
  }
  if (payload.chunk_size !== 32) {
    throw new Error(`unexpected base world chunk size: ${payload.chunk_size}`);
  }
  if (payload.transport.roads.length < 1_800) {
    throw new Error("base world roads layer is incomplete");
  }
  if (payload.transport.rails.length !== 256) {
    throw new Error("base world rails layer is incomplete");
  }
  if (payload.buildings.footprints.length < 2_268) {
    throw new Error("base world buildings layer is incomplete");
  }
}
```

### 6.4 Replace `src/main.ts` runtime world authority

In `src/main.ts`:

Remove:

```ts
import { createZurichRuntimeContext } from "./app/zurichRuntimeContext";
```

Add:

```ts
import { requireBaseWorld } from "./backend/baseWorldClient";
```

Replace:

```ts
const zurichContext = createZurichRuntimeContext({ seed: 1848 });
```

with:

```ts
const baseWorld = await requireBaseWorld();
```

Replace usages:

```ts
zurichContext.runtime.terrain
zurichContext.runtime.roads
zurichContext.runtime.rails
zurichContext.runtime.buildings
zurichContext.runtime.railPaths
zurichWorld.id
```

with:

```ts
{
  water: new Set(baseWorld.terrain.water.map(pointKey)),
}
new Set(baseWorld.transport.roads.map(pointKey))
new Set(baseWorld.transport.rails.map(pointKey))
new Set(baseWorld.buildings.footprints.flat().map(pointKey))
baseWorld.transport.rail_paths.map((path) => path.points)
baseWorld.world_id
```

Add local conversion helper:

```ts
function pointKey(point: { readonly x: number; readonly y: number }): string {
  return `${point.x},${point.y}`;
}
```

If the renderer expects `MapTileCoord` objects instead of string keys, use the existing conversion helper from renderer types. Keep the renderer input shape unchanged.

### 6.5 Keep `src/app/zurichRuntimeContext.ts` as generation-only

Move it under `src/tools/zurichRuntimeContext.ts` if imports allow, or leave it in place with this file-level comment:

```ts
// Generation-only context used by scripts/generate-base-world.mjs.
// Runtime rendering must consume /base-world instead.
```

Update the guard test from Task 1 so it allows this file only as generation tooling and still forbids imports from `src/main.ts` and app runtime entrypoints.

### 6.6 Update render smoke

Edit `tests/e2e/render-smoke.spec.ts`:

- Backend status world id becomes `zurich-river-city-v1`.
- The app state still asserts:
  - road tiles above `1800`
  - rail tiles exactly `256`
  - buildings above `2250`
  - no retired requests
  - mobility connected and moving

Run:

```bash
npm test -- tests/app/noDemoWorldAuthority.test.ts tests/app/noProductionFallbacks.test.ts
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
cargo test --manifest-path backend/Cargo.toml -p sim-server base_world_endpoint_serves_canonical_layers
```

Expected result after Task 6: browser map comes from `/base-world`; deleting or corrupting bundle files prevents app startup or initial render instead of silently using procedural data.

Commit slice:

```bash
git add backend/crates/sim-server src tests
git commit -m "Render frontend from base world"
```

---

## Task 7: Remove Retired/Demo Production Paths

**Purpose:** Delete stale assets and production code paths that confuse the current runtime.

### 7.1 Remove old Pak/Simutrans files from runtime-visible locations

Run:

```bash
rg -n "pak128|simutrans|opengfx|OpenGFX|Simutrans" public src tests scripts package.json
```

For each hit:

- Delete public/runtime assets that are not referenced by the canonical bundle.
- Keep historical references only in docs/specs if they explain the removal.
- Keep tests that assert retired assets are absent.

Use `git rm` for deleted files.

### 7.2 Expand retired asset guard

Edit `tests/render/noRetiredAssets.test.ts` to include:

```ts
const retiredPatterns = [
  /pak128/i,
  /simutrans/i,
  /opengfx/i,
  /SEEDED_CHUNKS/,
  /CityNetwork::empty_for_world/,
  /tiny_world\(\)/,
];
```

Scope source scanning to production directories:

```ts
const productionRoots = [
  "src",
  "backend/crates/sim-server/src",
  "backend/crates/sim-core/src",
  "public",
];
```

Exclude:

- `docs/superpowers/specs`
- `docs/superpowers/plans`
- test files that intentionally name the retired patterns

### 7.3 Delete obsolete runtime constants and imports

Use:

```bash
rg -n "SEEDED_CHUNKS|empty_for_world|tiny_world|legacy_seeded|createZurichRuntimeContext|pak128|simutrans|opengfx" backend/crates src public tests
```

Acceptable remaining hits:

- `scripts/generate-base-world.mjs` importing the generation-only Zurich context
- test fixtures under `#[cfg(test)]`
- guard tests that name forbidden patterns
- docs

No acceptable hits in production entrypoints:

- `src/main.ts`
- `src/app/appRuntime.ts`
- `backend/crates/sim-server/src/app.rs`
- `backend/crates/sim-server/src/runtime.rs`
- public runtime asset paths

Run:

```bash
npm test -- tests/render/noRetiredAssets.test.ts tests/app/noProductionFallbacks.test.ts tests/app/noDemoWorldAuthority.test.ts
cargo test --manifest-path backend/Cargo.toml -p sim-server
```

Commit slice:

```bash
git add -A
git commit -m "Remove retired demo world paths"
```

---

## Task 8: Add Snapshot Compatibility Metadata

**Purpose:** Prevent old demo snapshots from being hydrated into the new base world.

### 8.1 Add base world metadata to persisted snapshot structs

Find `MobilityPersistSnapshot` and chunk snapshot persistence structs:

```bash
rg -n "MobilityPersistSnapshot|ChunkSnapshot|snapshot" backend/crates/sim-core/src backend/crates/sim-server/src
```

Add metadata:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotWorldMetadata {
    pub base_world_id: String,
    pub base_world_schema_version: u32,
}
```

Embed it in mobility snapshots by adding this field next to the existing persisted fields:

```rust
pub world: Option<SnapshotWorldMetadata>,
```

If existing persisted JSON lacks this field, deserialize with an explicit legacy state:

```rust
#[serde(default)]
pub world: Option<SnapshotWorldMetadata>,
```

Then convert immediately after reading:

```rust
fn require_compatible_snapshot(
    snapshot: MobilityPersistSnapshot,
    bundle: &BaseWorldBundle,
) -> Option<MobilityPersistSnapshot> {
    let Some(world) = snapshot.world.as_ref() else {
        tracing::warn!("discarding legacy mobility snapshot without base world metadata");
        return None;
    };
    if world.base_world_id != bundle.world_id()
        || world.base_world_schema_version != bundle.manifest.schema_version
    {
        tracing::warn!(
            snapshot_world_id = %world.base_world_id,
            expected_world_id = %bundle.world_id(),
            "discarding incompatible mobility snapshot"
        );
        return None;
    }
    Some(snapshot)
}
```

The replacement behavior after rejection is a fresh seed from the validated base world. It is not a demo fallback.

### 8.2 Add chunk snapshot base-world key

Update persistence schema/migrations for Postgres and in-memory stores:

- Add `base_world_id`
- Add `base_world_schema_version`
- Reads filter by the active base world id and schema version
- Writes include the active base world id and schema version

Search migration location:

```bash
rg -n "CREATE TABLE|chunk_snapshots|mobility_snapshots|migrations" backend
```

Expected SQL direction:

```sql
ALTER TABLE chunk_snapshots
  ADD COLUMN IF NOT EXISTS base_world_id text,
  ADD COLUMN IF NOT EXISTS base_world_schema_version integer;

ALTER TABLE mobility_snapshots
  ADD COLUMN IF NOT EXISTS base_world_id text,
  ADD COLUMN IF NOT EXISTS base_world_schema_version integer;

CREATE INDEX IF NOT EXISTS chunk_snapshots_world_schema_idx
  ON chunk_snapshots (world_id, base_world_id, base_world_schema_version, chunk_x, chunk_y);
```

Do not mark legacy rows as current. Rows with null metadata are ignored by current startup.

### 8.3 Add compatibility tests

Add backend tests:

```rust
#[test]
fn rejects_legacy_mobility_snapshot_without_base_world_metadata() {
    let bundle = load_fixture_bundle();
    let legacy = legacy_snapshot_without_metadata();

    assert!(require_compatible_snapshot(legacy, &bundle).is_none());
}

#[test]
fn accepts_snapshot_for_current_base_world() {
    let bundle = load_fixture_bundle();
    let snapshot = snapshot_with_metadata(bundle.world_id(), bundle.manifest.schema_version);

    assert!(require_compatible_snapshot(snapshot, &bundle).is_some());
}
```

Use existing snapshot test helpers where available.

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server snapshot
cargo test --manifest-path backend/Cargo.toml -p sim-core snapshot
```

Commit slice:

```bash
git add backend
git commit -m "Gate snapshots by base world metadata"
```

---

## Task 9: Full Verification And Final Commit

**Purpose:** Verify the cutover end-to-end, including browser behavior.

### 9.1 Generate protocol and run full test suite

Run:

```bash
npm run generate:proto
npm run generate:base-world
npm test
cargo test --manifest-path backend/Cargo.toml
```

Expected:

- All Vitest suites pass.
- All Rust crates pass.
- Generated base world files are stable after regeneration.

Check stability:

```bash
git diff -- data/worlds/zurich-river-city-v1
```

Expected: no diff after `npm run generate:base-world`.

### 9.2 Run backend and frontend locally

Start the dev server using the existing script:

```bash
npm run dev
```

If backend is a separate process in this repo, start it with the existing documented command from `package.json` or `README.md`.

Open:

```text
http://127.0.0.1:5175/
```

Verify in the browser:

- Login button is visible if the app has no active session.
- Map renders the Mini-Metro-style Zurich base world.
- No old Pak/Simutrans graphics or paths appear in network requests.
- Cars are visible on roads.
- Pedestrians and cars move after a few seconds.
- Backend `/health` reports `world_id = "zurich-river-city-v1"`.
- Backend `/base-world` returns road, rail, terrain, building layers.

### 9.3 Run Playwright smoke

Run:

```bash
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected:

- Road count remains above threshold.
- Rail count is `256`.
- Building count remains above threshold.
- Retired requests list is empty.
- Mobility connection is live.
- Movement assertion passes.

### 9.4 Inspect final forbidden references

Run:

```bash
rg -n "SEEDED_CHUNKS|CityNetwork::empty_for_world|tiny_world\(\)|createZurichRuntimeContext|pak128|simutrans|opengfx" backend/crates src public tests scripts docs
```

Acceptable hits:

- `docs/superpowers/specs/2026-05-27-base-world-cutover-design.md`
- `docs/superpowers/plans/2026-05-27-base-world-cutover.md`
- guard tests that name the forbidden strings
- `scripts/generate-base-world.mjs`
- generation-only Zurich context file
- `#[cfg(test)]` utilities

No acceptable hits in production runtime entrypoints.

### 9.5 Final commit

If prior slice commits were not made, create one final commit:

```bash
git add -A
git commit -m "Cut over runtime to canonical base world"
```

Final status:

```bash
git status --short
git log --oneline --decorate -5
```

Expected:

- Working tree clean except intentionally untracked local files.
- Branch contains the spec commit, plan commit, and implementation commits.

---

## Review Checklist

- [ ] Bundle loading is fail-closed: missing manifest, missing layer, schema mismatch, empty required layers, or out-of-bounds data prevents startup.
- [ ] Backend world id is `zurich-river-city-v1`.
- [ ] Backend chunks are materialized from bundle data, not from `SEEDED_CHUNKS`.
- [ ] Runtime mobility seeds from bundle spawn data, not from `tiny_world()`.
- [ ] Frontend render layers come from `/base-world`, not from `createZurichRuntimeContext`.
- [ ] Old Pak/Simutrans/OpenGFX paths are absent from runtime-visible assets and requests.
- [ ] Snapshot reads are gated by base world id and schema version.
- [ ] `npm test`, Rust tests, and Playwright smoke pass.
