# Layered Terrain Seed Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the physical 256x256 map backend-authoritative and persistent through a clean layered terrain seed, without introducing economy/domain semantics.

**Architecture:** Generate a deterministic dense Zurich terrain seed from the existing frontend world/transport/placement builders, then load that artifact into the Rust backend as layered tile records. Replace the runtime-facing tile shape with `base + surface + cover + display/zone/masks`, expose it through protobuf chunk snapshots, and switch the Mini Motorways renderer to draw backend tile data instead of frontend-local Zurich builders.

**Tech Stack:** TypeScript, Node/tsx, Vitest, Rust, Bevy ECS, Prost/protobuf, Axum, Playwright.

---

## File Structure

- Create `src/city/layeredTerrainSeed.ts` — canonical TypeScript seed builder and validator from existing Zurich world, transport, and placement data.
- Create `tests/city/layeredTerrainSeed.test.ts` — seed shape and physical invariant coverage.
- Create `scripts/generate-layered-terrain-seed.mjs` — writes deterministic `data/city/zurich-layered-terrain-seed.json`.
- Modify `package.json` — add `generate:terrain-seed`.
- Modify `backend/crates/protocol/proto/abutown.proto` — replace one-dimensional tile wire shape with layered physical tile enums/messages.
- Modify `backend/crates/protocol/src/lib.rs` — add layered DTO enums and replace `TileMutationDto` with `LayeredTileDto`.
- Modify `backend/crates/sim-core/src/tile.rs` — replace durable `TileKind` target model with `LayeredTileRecord` and validation.
- Modify `backend/crates/sim-core/src/persistence.rs` — emit layered chunk snapshots.
- Create `backend/crates/sim-core/src/terrain_seed.rs` — Rust seed JSON loader and validator.
- Modify `backend/crates/sim-core/src/lib.rs` — export `terrain_seed`.
- Modify `backend/crates/sim-server/src/runtime.rs` — hydrate fresh chunk tiles from the layered seed.
- Modify `backend/crates/sim-server/src/app.rs` — map layered chunk snapshots to protobuf.
- Modify `backend/crates/sim-server/tests/http.rs` and `backend/crates/sim-server/tests/websocket.rs` — update chunk snapshot and tile command expectations.
- Modify `src/backend/mobilityProtocol.ts` or create `src/backend/terrainProtocol.ts` — decode layered chunk snapshots from generated protobuf.
- Modify `src/main.ts` — derive rendered terrain, roads, rails, buildings, trees, and details from backend chunk snapshots.
- Modify `tests/e2e/render-smoke.spec.ts` — assert layered backend terrain is used.
- Modify `progress.md` — record the migration after verification.

## Guardrails

- Do not add company, workplace, housing, owner, job, production, storage, money, market, ledger, or resource-extraction fields.
- Do not make an expanded `TileKind` enum the target model.
- Do not keep frontend Zurich builders as runtime rendering fallback.
- If a temporary adapter is unavoidable to get tests through a transition, isolate it in one module and add a task that removes or acceptance-greps it.

## Tasks

### Task 1: TypeScript Layered Seed Builder

**Files:**
- Create: `src/city/layeredTerrainSeed.ts`
- Create: `tests/city/layeredTerrainSeed.test.ts`

- [ ] **Step 1: Write the failing seed-builder tests**

Create `tests/city/layeredTerrainSeed.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { buildLayeredTerrainSeed, validateLayeredTerrainSeed } from '../../src/city/layeredTerrainSeed';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichWorld } from '../../src/city/zurichWorld';

describe('layered terrain seed', () => {
  it('builds one physical layered tile for every Zurich coordinate', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    const seed = buildLayeredTerrainSeed({ world, transport, placement });

    expect(seed.version).toBe(1);
    expect(seed.world_id).toBe('zurich-river-city-v1');
    expect(seed.width).toBe(256);
    expect(seed.height).toBe(256);
    expect(seed.chunk_size).toBe(32);
    expect(seed.tiles).toHaveLength(256 * 256);
    expect(new Set(seed.tiles.map((tile) => `${tile.x}:${tile.y}`))).toHaveLength(256 * 256);
  });

  it('separates base, surface, and cover layers', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const seed = buildLayeredTerrainSeed({ world, transport, placement });

    const bridgeTile = seed.tiles.find((tile) => tile.surface === 'Bridge');
    expect(bridgeTile).toEqual(expect.objectContaining({
      base: expect.stringMatching(/Water|Riverbank/),
      surface: 'Bridge',
      cover: 'None',
      road_mask: expect.any(Number),
    }));

    const buildingTile = seed.tiles.find((tile) => tile.cover === 'Building');
    expect(buildingTile).toEqual(expect.objectContaining({
      surface: 'None',
      display: expect.any(String),
      zone_id: expect.stringMatching(/^zone:/),
    }));
  });

  it('rejects invalid physical layer combinations', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const seed = buildLayeredTerrainSeed({ world, transport, placement });
    const invalid = {
      ...seed,
      tiles: seed.tiles.map((tile, index) =>
        index === 0 ? { ...tile, base: 'Water' as const, surface: 'Street' as const, cover: 'Building' as const } : tile,
      ),
    };

    expect(validateLayeredTerrainSeed(seed)).toEqual([]);
    expect(validateLayeredTerrainSeed(invalid)).toContain('tile:0:0:building_on_water');
    expect(validateLayeredTerrainSeed(invalid)).toContain('tile:0:0:cover_on_transport_surface');
  });
});
```

- [ ] **Step 2: Run the test to verify RED**

Run:

```bash
npm test -- tests/city/layeredTerrainSeed.test.ts
```

Expected: FAIL because `../../src/city/layeredTerrainSeed` does not exist.

- [ ] **Step 3: Implement the seed builder**

Create `src/city/layeredTerrainSeed.ts`:

```ts
import { key, type Coord, type ZurichBuilding, type ZurichDetail, type ZurichTerrainKind, type ZurichWorld } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

export type LayeredBaseKind = 'Grass' | 'Water' | 'Riverbank' | 'Forest' | 'Park' | 'Reserve' | 'Plaza';
export type LayeredSurfaceKind = 'None' | 'Street' | 'Bridge' | 'Rail' | 'RailCrossing';
export type LayeredCoverKind = 'None' | 'Building' | 'Tree' | 'Detail';

export type LayeredTerrainTile = {
  x: number;
  y: number;
  base: LayeredBaseKind;
  surface: LayeredSurfaceKind;
  cover: LayeredCoverKind;
  display: string | null;
  zone_id: string | null;
  road_mask: number | null;
  rail_mask: number | null;
  version: number;
};

export type LayeredTerrainSeed = {
  version: 1;
  world_id: string;
  width: number;
  height: number;
  chunk_size: number;
  tiles: LayeredTerrainTile[];
};

export function buildLayeredTerrainSeed(input: {
  world: ZurichWorld;
  transport: ZurichTransport;
  placement: ZurichPlacement;
}): LayeredTerrainSeed {
  const buildingsByKey = new Map(input.placement.buildings.map((building) => [key(building.coord), building]));
  const treesByKey = new Set(input.placement.trees.map(key));
  const detailsByKey = new Map(input.placement.details.map((detail) => [key(detail.coord), detail]));
  const tiles: LayeredTerrainTile[] = [];

  for (let y = 0; y < input.world.height; y += 1) {
    for (let x = 0; x < input.world.width; x += 1) {
      const coord = { x, y };
      const tileKey = key(coord);
      const terrain = input.world.terrain.get(tileKey);
      if (!terrain) throw new Error(`missing terrain tile ${tileKey}`);
      const road = input.transport.roads.get(tileKey);
      const rail = input.transport.rails.get(tileKey);
      const building = buildingsByKey.get(tileKey);
      const detail = detailsByKey.get(tileKey);
      const surface = surfaceFor({ roadKind: road?.kind, hasRail: Boolean(rail), isRailCrossing: input.transport.railCrossings.has(tileKey) });
      const cover = coverFor({ building, hasTree: treesByKey.has(tileKey), detail, surface });

      tiles.push({
        x,
        y,
        base: baseFor(terrain.kind),
        surface,
        cover,
        display: displayFor({ building, detail }),
        zone_id: terrain.zoneId ?? null,
        road_mask: road ? road.mask : null,
        rail_mask: rail ? rail.mask : null,
        version: 0,
      });
    }
  }

  return {
    version: 1,
    world_id: input.world.id,
    width: input.world.width,
    height: input.world.height,
    chunk_size: input.world.chunkSize,
    tiles,
  };
}

export function validateLayeredTerrainSeed(seed: LayeredTerrainSeed): string[] {
  const errors: string[] = [];
  const seen = new Set<string>();
  if (seed.tiles.length !== seed.width * seed.height) errors.push(`tile_count:${seed.tiles.length}`);
  if (seed.width % seed.chunk_size !== 0 || seed.height % seed.chunk_size !== 0) errors.push('chunk_size:does_not_partition_world');

  for (const tile of seed.tiles) {
    const tileKey = `${tile.x}:${tile.y}`;
    if (tile.x < 0 || tile.y < 0 || tile.x >= seed.width || tile.y >= seed.height) errors.push(`tile:${tileKey}:out_of_bounds`);
    if (seen.has(tileKey)) errors.push(`tile:${tileKey}:duplicate`);
    seen.add(tileKey);
    if (tile.surface === 'Bridge' && tile.base !== 'Water' && tile.base !== 'Riverbank') errors.push(`tile:${tileKey}:bridge_without_water`);
    if (tile.cover === 'Building' && tile.base === 'Water') errors.push(`tile:${tileKey}:building_on_water`);
    if ((tile.cover === 'Building' || tile.cover === 'Tree') && tile.surface !== 'None') errors.push(`tile:${tileKey}:cover_on_transport_surface`);
    if (tile.road_mask !== null && tile.surface !== 'Street' && tile.surface !== 'Bridge' && tile.surface !== 'RailCrossing') errors.push(`tile:${tileKey}:road_mask_without_road_surface`);
    if (tile.rail_mask !== null && tile.surface !== 'Rail' && tile.surface !== 'RailCrossing') errors.push(`tile:${tileKey}:rail_mask_without_rail_surface`);
  }

  return errors;
}

function baseFor(kind: ZurichTerrainKind): LayeredBaseKind {
  const mapping: Record<ZurichTerrainKind, LayeredBaseKind> = {
    grass: 'Grass',
    water: 'Water',
    riverbank: 'Riverbank',
    forest: 'Forest',
    park: 'Park',
    reserve: 'Reserve',
    plaza: 'Plaza',
  };
  return mapping[kind];
}

function surfaceFor(input: { roadKind?: 'street' | 'bridge'; hasRail: boolean; isRailCrossing: boolean }): LayeredSurfaceKind {
  if (input.isRailCrossing) return 'RailCrossing';
  if (input.roadKind === 'bridge') return 'Bridge';
  if (input.roadKind === 'street') return 'Street';
  if (input.hasRail) return 'Rail';
  return 'None';
}

function coverFor(input: {
  building?: ZurichBuilding;
  hasTree: boolean;
  detail?: ZurichDetail;
  surface: LayeredSurfaceKind;
}): LayeredCoverKind {
  if (input.surface !== 'None') return 'None';
  if (input.building) return 'Building';
  if (input.hasTree) return 'Tree';
  if (input.detail) return 'Detail';
  return 'None';
}

function displayFor(input: { building?: ZurichBuilding; detail?: ZurichDetail }): string | null {
  if (input.building) return input.building.sheet;
  if (input.detail) return input.detail.assetCategory;
  return null;
}
```

- [ ] **Step 4: Verify GREEN**

Run:

```bash
npm test -- tests/city/layeredTerrainSeed.test.ts
```

Expected: PASS with 3 tests.

- [ ] **Step 5: Commit**

```bash
git add src/city/layeredTerrainSeed.ts tests/city/layeredTerrainSeed.test.ts
git commit -m "feat: add layered terrain seed builder"
```

### Task 2: Deterministic Seed Artifact

**Files:**
- Create: `scripts/generate-layered-terrain-seed.mjs`
- Create: `data/city/zurich-layered-terrain-seed.json`
- Modify: `package.json`

- [ ] **Step 1: Add the npm script**

Modify `package.json` scripts:

```json
"generate:terrain-seed": "node --import tsx/esm scripts/generate-layered-terrain-seed.mjs"
```

- [ ] **Step 2: Create the seed generation script**

Create `scripts/generate-layered-terrain-seed.mjs`:

```js
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { buildLayeredTerrainSeed, validateLayeredTerrainSeed } from '../src/city/layeredTerrainSeed.ts';
import { buildZurichPlacement } from '../src/city/zurichPlacement.ts';
import { buildZurichTransport } from '../src/city/zurichTransport.ts';
import { buildZurichWorld } from '../src/city/zurichWorld.ts';

const outPath = path.resolve('data/city/zurich-layered-terrain-seed.json');
const world = buildZurichWorld({ seed: 1848 });
const transport = buildZurichTransport(world);
const placement = buildZurichPlacement(world, transport);
const seed = buildLayeredTerrainSeed({ world, transport, placement });
const errors = validateLayeredTerrainSeed(seed);

if (errors.length > 0) {
  throw new Error(`layered terrain seed failed validation:\\n${errors.join('\\n')}`);
}

await mkdir(path.dirname(outPath), { recursive: true });
await writeFile(outPath, `${JSON.stringify(seed, null, 2)}\\n`, 'utf8');
console.log(`layered terrain seed complete -> ${outPath} (${seed.tiles.length} tiles)`);
```

- [ ] **Step 3: Generate the artifact**

Run:

```bash
npm run generate:terrain-seed
```

Expected output includes:

```text
layered terrain seed complete -> .../data/city/zurich-layered-terrain-seed.json (65536 tiles)
```

- [ ] **Step 4: Verify deterministic output**

Run:

```bash
git diff -- data/city/zurich-layered-terrain-seed.json
npm run generate:terrain-seed
git diff --exit-code -- data/city/zurich-layered-terrain-seed.json
```

Expected: second `git diff --exit-code` exits 0.

- [ ] **Step 5: Commit**

```bash
git add package.json scripts/generate-layered-terrain-seed.mjs data/city/zurich-layered-terrain-seed.json
git commit -m "feat: generate layered terrain seed artifact"
```

### Task 3: Protobuf and Protocol DTOs

**Files:**
- Modify: `backend/crates/protocol/proto/abutown.proto`
- Modify: `backend/crates/protocol/src/lib.rs`
- Modify: `backend/crates/protocol/src/lib.rs` tests near proto conversion tests

- [ ] **Step 1: Write failing protocol tests**

Add to the protocol test module in `backend/crates/protocol/src/lib.rs`:

```rust
#[test]
fn layered_tile_proto_round_trips() {
    let tile = LayeredTileDto {
        local_index: 7,
        base: TileBaseDto::Riverbank,
        surface: TileSurfaceDto::Bridge,
        cover: TileCoverDto::None,
        display: None,
        zone_id: Some("zone:limmat-river".to_string()),
        road_mask: Some(10),
        rail_mask: None,
        version: 3,
    };

    let proto: v1::LayeredTile = tile.clone().into();
    let back = LayeredTileDto::try_from(proto).expect("valid layered tile");

    assert_eq!(back, tile);
}

#[test]
fn chunk_snapshot_uses_layered_tiles() {
    let snapshot = ChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        coord: ChunkCoordDto { x: 1, y: 2 },
        chunk_state: ChunkStateDto::Active,
        chunk_version: 5,
        tile_count: 1024,
        tiles: vec![LayeredTileDto {
            local_index: 0,
            base: TileBaseDto::Grass,
            surface: TileSurfaceDto::Street,
            cover: TileCoverDto::None,
            display: None,
            zone_id: None,
            road_mask: Some(5),
            rail_mask: None,
            version: 1,
        }],
    };

    assert_eq!(snapshot.tiles[0].surface, TileSurfaceDto::Street);
}
```

- [ ] **Step 2: Run protocol tests to verify RED**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol layered_tile_proto_round_trips chunk_snapshot_uses_layered_tiles
```

Expected: FAIL because DTOs and proto messages do not exist.

- [ ] **Step 3: Replace tile wire schema with layered schema**

Modify `backend/crates/protocol/proto/abutown.proto`:

```proto
enum TileBase {
  reserved 100 to max;
  TILE_BASE_UNSPECIFIED = 0;
  TILE_BASE_GRASS = 1;
  TILE_BASE_WATER = 2;
  TILE_BASE_RIVERBANK = 3;
  TILE_BASE_FOREST = 4;
  TILE_BASE_PARK = 5;
  TILE_BASE_RESERVE = 6;
  TILE_BASE_PLAZA = 7;
}

enum TileSurface {
  reserved 100 to max;
  TILE_SURFACE_UNSPECIFIED = 0;
  TILE_SURFACE_NONE = 1;
  TILE_SURFACE_STREET = 2;
  TILE_SURFACE_BRIDGE = 3;
  TILE_SURFACE_RAIL = 4;
  TILE_SURFACE_RAIL_CROSSING = 5;
}

enum TileCover {
  reserved 100 to max;
  TILE_COVER_UNSPECIFIED = 0;
  TILE_COVER_NONE = 1;
  TILE_COVER_BUILDING = 2;
  TILE_COVER_TREE = 3;
  TILE_COVER_DETAIL = 4;
}

message ChunkSnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  ChunkCoord coord = 3;
  uint64 chunk_version = 4;
  ChunkState chunk_state = 5;
  uint32 tile_count = 6;
  repeated LayeredTile tiles = 7;
}

message LayeredTile {
  uint32 local_index = 1;
  TileBase base = 2;
  TileSurface surface = 3;
  TileCover cover = 4;
  optional string display = 5;
  optional string zone_id = 6;
  optional uint32 road_mask = 7;
  optional uint32 rail_mask = 8;
  uint64 version = 9;
}
```

Remove `enum TileKind`, `message TileMutation`, `SetTileKindCommand`, and `TileKindSetEvent` from the target schema. If deletion breaks old command tests, update those tests in Task 7 rather than keeping the legacy command as a target model.

- [ ] **Step 4: Add protocol DTOs**

Modify `backend/crates/protocol/src/lib.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileBaseDto {
    Grass,
    Water,
    Riverbank,
    Forest,
    Park,
    Reserve,
    Plaza,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileSurfaceDto {
    None,
    Street,
    Bridge,
    Rail,
    RailCrossing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileCoverDto {
    None,
    Building,
    Tree,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayeredTileDto {
    pub local_index: u16,
    pub base: TileBaseDto,
    pub surface: TileSurfaceDto,
    pub cover: TileCoverDto,
    pub display: Option<String>,
    pub zone_id: Option<String>,
    pub road_mask: Option<u8>,
    pub rail_mask: Option<u8>,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkSnapshotDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub coord: ChunkCoordDto,
    pub chunk_state: ChunkStateDto,
    pub chunk_version: u64,
    pub tile_count: u16,
    pub tiles: Vec<LayeredTileDto>,
}
```

Add `From`/`TryFrom` implementations for the new enums and `LayeredTileDto`. Use explicit `TryFrom<i32>` handling so unspecified enum values reject with a string error.

- [ ] **Step 5: Generate protobuf and verify**

Run:

```bash
npm run generate:proto
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol layered_tile_proto_round_trips chunk_snapshot_uses_layered_tiles
```

Expected: protocol tests pass.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/protocol/proto/abutown.proto backend/crates/protocol/src/lib.rs src/backend/proto
git commit -m "feat: define layered terrain protocol"
```

### Task 4: Rust Layered Tile Model and Validation

**Files:**
- Modify: `backend/crates/sim-core/src/tile.rs`

- [ ] **Step 1: Write failing Rust tile tests**

Replace/add tests in `backend/crates/sim-core/src/tile.rs`:

```rust
#[test]
fn layered_tile_defaults_to_grass_with_empty_layers() {
    let tile = LayeredTileRecord::default();
    assert_eq!(tile.base, TileBase::Grass);
    assert_eq!(tile.surface, TileSurface::None);
    assert_eq!(tile.cover, TileCover::None);
    assert_eq!(tile.version, 0);
}

#[test]
fn layered_tile_validation_rejects_invalid_physical_combinations() {
    let invalid = LayeredTileRecord {
        base: TileBase::Water,
        surface: TileSurface::Street,
        cover: TileCover::Building,
        display: Some("houses".to_string()),
        zone_id: Some("zone:test".to_string()),
        road_mask: Some(1),
        rail_mask: None,
        version: 0,
    };

    let errors = invalid.validate();
    assert!(errors.contains(&TileValidationError::BuildingOnWater));
    assert!(errors.contains(&TileValidationError::CoverOnTransportSurface));
}
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core layered_tile_defaults_to_grass_with_empty_layers layered_tile_validation_rejects_invalid_physical_combinations
```

Expected: FAIL because `LayeredTileRecord` does not exist.

- [ ] **Step 3: Implement clean tile model**

Modify `backend/crates/sim-core/src/tile.rs`:

```rust
use abutown_protocol::{LayeredTileDto, TileBaseDto, TileCoverDto, TileSurfaceDto};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileBase {
    #[default]
    Grass,
    Water,
    Riverbank,
    Forest,
    Park,
    Reserve,
    Plaza,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileSurface {
    #[default]
    None,
    Street,
    Bridge,
    Rail,
    RailCrossing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TileCover {
    #[default]
    None,
    Building,
    Tree,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayeredTileRecord {
    pub base: TileBase,
    pub surface: TileSurface,
    pub cover: TileCover,
    pub display: Option<String>,
    pub zone_id: Option<String>,
    pub road_mask: Option<u8>,
    pub rail_mask: Option<u8>,
    pub version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileValidationError {
    BridgeWithoutWater,
    BuildingOnWater,
    CoverOnTransportSurface,
    RoadMaskWithoutRoadSurface,
    RailMaskWithoutRailSurface,
}

impl LayeredTileRecord {
    pub fn validate(&self) -> Vec<TileValidationError> {
        let mut errors = Vec::new();
        if self.surface == TileSurface::Bridge && self.base != TileBase::Water && self.base != TileBase::Riverbank {
            errors.push(TileValidationError::BridgeWithoutWater);
        }
        if self.cover == TileCover::Building && self.base == TileBase::Water {
            errors.push(TileValidationError::BuildingOnWater);
        }
        if matches!(self.cover, TileCover::Building | TileCover::Tree) && self.surface != TileSurface::None {
            errors.push(TileValidationError::CoverOnTransportSurface);
        }
        if self.road_mask.is_some() && !matches!(self.surface, TileSurface::Street | TileSurface::Bridge | TileSurface::RailCrossing) {
            errors.push(TileValidationError::RoadMaskWithoutRoadSurface);
        }
        if self.rail_mask.is_some() && !matches!(self.surface, TileSurface::Rail | TileSurface::RailCrossing) {
            errors.push(TileValidationError::RailMaskWithoutRailSurface);
        }
        errors
    }
}
```

Add conversions between `LayeredTileRecord` and `LayeredTileDto`. Delete `TileKind` as a target type. If callers still need temporary conversion, create `pub(crate)` helpers in the caller module, not in `tile.rs`.

- [ ] **Step 4: Update dense chunk tile storage type**

Update imports/usages that currently expect `TileRecord` fields `kind`/`flags` so chunk storage uses:

```rust
pub type TileRecord = LayeredTileRecord;
```

Only keep this type alias if it reduces churn. Do not reintroduce the old `TileKind` fields through the alias.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core layered_tile_defaults_to_grass_with_empty_layers layered_tile_validation_rejects_invalid_physical_combinations
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/tile.rs
git commit -m "feat: add layered tile record model"
```

### Task 5: Rust Terrain Seed Loader

**Files:**
- Create: `backend/crates/sim-core/src/terrain_seed.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`
- Test: `backend/crates/sim-core/src/terrain_seed.rs`

- [ ] **Step 1: Write failing seed loader tests**

Create tests in `backend/crates/sim-core/src/terrain_seed.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_zurich_layered_seed_from_repo_data() {
        let seed = load_zurich_layered_terrain_seed().expect("seed loads");
        assert_eq!(seed.version, 1);
        assert_eq!(seed.world_id, "zurich-river-city-v1");
        assert_eq!(seed.width, 256);
        assert_eq!(seed.height, 256);
        assert_eq!(seed.chunk_size, 32);
        assert_eq!(seed.tiles.len(), 256 * 256);
        assert!(validate_seed(&seed).is_empty());
    }

    #[test]
    fn converts_dense_seed_to_chunk_tiles() {
        let seed = load_zurich_layered_terrain_seed().expect("seed loads");
        let tiles = chunk_tiles_from_seed(&seed, crate::ids::ChunkCoord { x: 0, y: 0 }).expect("chunk exists");
        assert_eq!(tiles.len(), 32 * 32);
    }
}
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core terrain_seed
```

Expected: FAIL because `terrain_seed` module does not exist.

- [ ] **Step 3: Implement seed loader**

Create `backend/crates/sim-core/src/terrain_seed.rs`:

```rust
use crate::ids::ChunkCoord;
use crate::tile::{LayeredTileRecord, TileBase, TileCover, TileSurface};
use serde::Deserialize;

const ZURICH_LAYERED_TERRAIN_SEED: &str =
    include_str!("../../../../data/city/zurich-layered-terrain-seed.json");

#[derive(Debug, Clone, Deserialize)]
pub struct LayeredTerrainSeed {
    pub version: u32,
    pub world_id: String,
    pub width: u32,
    pub height: u32,
    pub chunk_size: u16,
    pub tiles: Vec<SeedTile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedTile {
    pub x: u32,
    pub y: u32,
    pub base: TileBase,
    pub surface: TileSurface,
    pub cover: TileCover,
    pub display: Option<String>,
    pub zone_id: Option<String>,
    pub road_mask: Option<u8>,
    pub rail_mask: Option<u8>,
    pub version: u64,
}

pub fn load_zurich_layered_terrain_seed() -> Result<LayeredTerrainSeed, serde_json::Error> {
    serde_json::from_str(ZURICH_LAYERED_TERRAIN_SEED)
}

pub fn validate_seed(seed: &LayeredTerrainSeed) -> Vec<String> {
    let mut errors = Vec::new();
    if seed.tiles.len() != (seed.width * seed.height) as usize {
        errors.push(format!("tile_count:{}", seed.tiles.len()));
    }
    if seed.width % u32::from(seed.chunk_size) != 0 || seed.height % u32::from(seed.chunk_size) != 0 {
        errors.push("chunk_size:does_not_partition_world".to_string());
    }
    for tile in &seed.tiles {
        let record = tile.to_record();
        for error in record.validate() {
            errors.push(format!("tile:{}:{}:{error:?}", tile.x, tile.y));
        }
    }
    errors
}

pub fn chunk_tiles_from_seed(seed: &LayeredTerrainSeed, coord: ChunkCoord) -> Option<Vec<LayeredTileRecord>> {
    let cs = u32::from(seed.chunk_size);
    let start_x = u32::try_from(coord.x).ok()?.checked_mul(cs)?;
    let start_y = u32::try_from(coord.y).ok()?.checked_mul(cs)?;
    if start_x >= seed.width || start_y >= seed.height {
        return None;
    }
    let mut result = Vec::with_capacity((cs * cs) as usize);
    for y in start_y..start_y + cs {
        for x in start_x..start_x + cs {
            let index = (y * seed.width + x) as usize;
            result.push(seed.tiles.get(index)?.to_record());
        }
    }
    Some(result)
}

impl SeedTile {
    fn to_record(&self) -> LayeredTileRecord {
        LayeredTileRecord {
            base: self.base,
            surface: self.surface,
            cover: self.cover,
            display: self.display.clone(),
            zone_id: self.zone_id.clone(),
            road_mask: self.road_mask,
            rail_mask: self.rail_mask,
            version: self.version,
        }
    }
}
```

If `serde` cannot deserialize enum names because TS emits PascalCase, add `#[serde(rename_all = "PascalCase")]` to the Rust enums in `tile.rs`.

- [ ] **Step 4: Export module**

Modify `backend/crates/sim-core/src/lib.rs`:

```rust
pub mod terrain_seed;
```

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core terrain_seed
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/terrain_seed.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: load layered terrain seed in sim core"
```

### Task 6: Backend Runtime Hydration and Persistence

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-core/src/persistence.rs`
- Modify: `backend/crates/sim-core/src/world/snapshot_provider.rs`
- Modify: `backend/crates/sim-server/src/postgres_snapshots.rs`

- [ ] **Step 1: Write failing runtime hydration test**

Add to `backend/crates/sim-server/src/runtime.rs` tests:

```rust
#[tokio::test]
async fn fresh_runtime_hydrates_chunks_from_layered_terrain_seed() {
    let runtime = SimulationRuntime::new(
        WorldId("abutown-main".to_string()),
        Arc::new(InMemoryWorldEventStore::default()),
    )
    .await
    .expect("runtime builds");

    let snapshot = runtime
        .chunk_snapshot(ChunkCoordDto { x: 4, y: 4 })
        .expect("seeded chunk snapshot");

    assert_eq!(snapshot.tile_count, 1024);
    assert!(snapshot.tiles.iter().any(|tile| tile.base == abutown_protocol::TileBaseDto::Water));
    assert!(snapshot.tiles.iter().any(|tile| tile.surface == abutown_protocol::TileSurfaceDto::Street));
}
```

- [ ] **Step 2: Run test to verify RED**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server fresh_runtime_hydrates_chunks_from_layered_terrain_seed
```

Expected: FAIL because snapshots still emit old tile kind data.

- [ ] **Step 3: Hydrate chunk tiles from seed**

In `SimulationRuntime::new` or the current fresh-world branch, replace synthetic tile initialization:

```rust
let seed = sim_core::terrain_seed::load_zurich_layered_terrain_seed()
    .expect("bundled Zurich layered terrain seed must parse");
let seed_errors = sim_core::terrain_seed::validate_seed(&seed);
assert!(seed_errors.is_empty(), "bundled terrain seed invalid: {seed_errors:?}");

for chunk_y in 0..(seed.height / u32::from(seed.chunk_size)) {
    for chunk_x in 0..(seed.width / u32::from(seed.chunk_size)) {
        let coord = ChunkCoord { x: chunk_x as i32, y: chunk_y as i32 };
        let tiles = sim_core::terrain_seed::chunk_tiles_from_seed(&seed, coord)
            .expect("chunk tiles exist in seed");
        spawn_chunk_entity(&mut world, coord, seed.chunk_size, tiles, 0, ChunkActivity::Active);
    }
}
```

Keep chunk LOD behavior as it is today unless tests require a specific initial marker.

- [ ] **Step 4: Emit layered chunk snapshots**

Modify `backend/crates/sim-core/src/persistence.rs` so `build_chunk_snapshot_from_parts` pushes `LayeredTileDto` records for any non-default layered record, and for the first layered snapshot include all non-default physical tiles. Use:

```rust
if tile != &LayeredTileRecord::default() {
    emitted.push(tile.to_dto(index as u16));
}
```

If the renderer needs complete visible chunks on initial load, expose full chunk snapshots from runtime read view even when the persistence writer emits sparse dirty snapshots. Do not make persistence sparsity dictate wire completeness.

- [ ] **Step 5: Update Postgres snapshot roundtrip**

Modify `backend/crates/sim-server/src/postgres_snapshots.rs` tests to insert/read `LayeredTileDto`:

```rust
tiles: vec![LayeredTileDto {
    local_index: 3,
    base: TileBaseDto::Grass,
    surface: TileSurfaceDto::Street,
    cover: TileCoverDto::None,
    display: None,
    zone_id: Some("zone:test".to_string()),
    road_mask: Some(5),
    rail_mask: None,
    version: 1,
}],
```

- [ ] **Step 6: Verify backend target**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server fresh_runtime_hydrates_chunks_from_layered_terrain_seed
cargo test --manifest-path backend/Cargo.toml -p sim-core persistence
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs backend/crates/sim-core/src/persistence.rs backend/crates/sim-core/src/world/snapshot_provider.rs backend/crates/sim-server/src/postgres_snapshots.rs
git commit -m "feat: hydrate runtime from layered terrain seed"
```

### Task 7: Server Protobuf Mapping and API Tests

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/http.rs`
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [ ] **Step 1: Write failing HTTP chunk snapshot assertions**

Update `backend/crates/sim-server/tests/http.rs` chunk snapshot test:

```rust
let body = get_proto::<w::ChunkSnapshot>(&client, "/chunks/4/4").await;
assert_eq!(body.tile_count, 1024);
assert!(body.tiles.iter().any(|tile| tile.base == w::TileBase::Water as i32));
assert!(body.tiles.iter().any(|tile| tile.surface == w::TileSurface::Street as i32));
```

- [ ] **Step 2: Run test to verify RED**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http chunk_snapshot_is_available
```

Expected: FAIL until `app.rs` maps layered tiles.

- [ ] **Step 3: Map layered DTOs to protobuf**

Replace the old `tile_kind_to_proto` mapping in `backend/crates/sim-server/src/app.rs` with:

```rust
fn tile_base_to_proto(value: abutown_protocol::TileBaseDto) -> w::TileBase {
    match value {
        abutown_protocol::TileBaseDto::Grass => w::TileBase::Grass,
        abutown_protocol::TileBaseDto::Water => w::TileBase::Water,
        abutown_protocol::TileBaseDto::Riverbank => w::TileBase::Riverbank,
        abutown_protocol::TileBaseDto::Forest => w::TileBase::Forest,
        abutown_protocol::TileBaseDto::Park => w::TileBase::Park,
        abutown_protocol::TileBaseDto::Reserve => w::TileBase::Reserve,
        abutown_protocol::TileBaseDto::Plaza => w::TileBase::Plaza,
    }
}

fn layered_tile_to_proto(tile: &abutown_protocol::LayeredTileDto) -> w::LayeredTile {
    w::LayeredTile {
        local_index: u32::from(tile.local_index),
        base: tile_base_to_proto(tile.base) as i32,
        surface: tile_surface_to_proto(tile.surface) as i32,
        cover: tile_cover_to_proto(tile.cover) as i32,
        display: tile.display.clone(),
        zone_id: tile.zone_id.clone(),
        road_mask: tile.road_mask.map(u32::from),
        rail_mask: tile.rail_mask.map(u32::from),
        version: tile.version,
    }
}
```

Add equivalent `tile_surface_to_proto` and `tile_cover_to_proto` functions.

- [ ] **Step 4: Remove old tile command wire path**

Remove or replace tests that send `SetTileKindCommand`. Since this seed phase has no terrain edit UI, command mutation can be cut from the public API for now. If the codebase still needs a command for existing event-store tests, introduce a narrow `SetLayeredTileCommand` with all layers explicit; do not keep `SetTileKindCommand` as the target model.

- [ ] **Step 5: Verify server API tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat: expose layered terrain chunks"
```

### Task 8: Frontend Backend Terrain State

**Files:**
- Create: `src/backend/terrainState.ts`
- Create: `tests/backend/terrainState.test.ts`
- Modify: `src/backend/proto/abutown_pb.ts` via `npm run generate:proto`

- [ ] **Step 1: Write failing terrain state tests**

Create `tests/backend/terrainState.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { applyLayeredChunkSnapshot, createTerrainState, terrainTileAt } from '../../src/backend/terrainState';

describe('terrain state', () => {
  it('stores layered chunk snapshots by world coordinate', () => {
    const state = createTerrainState({ width: 256, height: 256, chunkSize: 32 });

    applyLayeredChunkSnapshot(state, {
      coord: { x: 1, y: 2 },
      tileCount: 1024,
      tiles: [
        {
          localIndex: 5,
          base: 'Grass',
          surface: 'Street',
          cover: 'None',
          display: null,
          zoneId: 'zone:test',
          roadMask: 5,
          railMask: null,
          version: 1,
        },
      ],
    });

    expect(terrainTileAt(state, { x: 37, y: 64 })).toEqual(expect.objectContaining({
      base: 'Grass',
      surface: 'Street',
      roadMask: 5,
    }));
  });
});
```

- [ ] **Step 2: Run test to verify RED**

Run:

```bash
npm test -- tests/backend/terrainState.test.ts
```

Expected: FAIL because `terrainState` does not exist.

- [ ] **Step 3: Implement terrain state**

Create `src/backend/terrainState.ts`:

```ts
export type TerrainCoord = { x: number; y: number };
export type TerrainTile = {
  base: 'Grass' | 'Water' | 'Riverbank' | 'Forest' | 'Park' | 'Reserve' | 'Plaza';
  surface: 'None' | 'Street' | 'Bridge' | 'Rail' | 'RailCrossing';
  cover: 'None' | 'Building' | 'Tree' | 'Detail';
  display: string | null;
  zoneId: string | null;
  roadMask: number | null;
  railMask: number | null;
  version: number;
};

export type TerrainState = {
  width: number;
  height: number;
  chunkSize: number;
  tiles: Map<string, TerrainTile>;
  loadedChunks: Set<string>;
};

export type LayeredChunkSnapshotLike = {
  coord: TerrainCoord;
  tileCount: number;
  tiles: Array<TerrainTile & { localIndex: number }>;
};

export function createTerrainState(input: { width: number; height: number; chunkSize: number }): TerrainState {
  return { ...input, tiles: new Map(), loadedChunks: new Set() };
}

export function applyLayeredChunkSnapshot(state: TerrainState, snapshot: LayeredChunkSnapshotLike): void {
  state.loadedChunks.add(`${snapshot.coord.x}:${snapshot.coord.y}`);
  for (const tile of snapshot.tiles) {
    const localX = tile.localIndex % state.chunkSize;
    const localY = Math.floor(tile.localIndex / state.chunkSize);
    const x = snapshot.coord.x * state.chunkSize + localX;
    const y = snapshot.coord.y * state.chunkSize + localY;
    state.tiles.set(`${x}:${y}`, {
      base: tile.base,
      surface: tile.surface,
      cover: tile.cover,
      display: tile.display,
      zoneId: tile.zoneId,
      roadMask: tile.roadMask,
      railMask: tile.railMask,
      version: tile.version,
    });
  }
}

export function terrainTileAt(state: TerrainState, coord: TerrainCoord): TerrainTile | undefined {
  return state.tiles.get(`${Math.round(coord.x)}:${Math.round(coord.y)}`);
}
```

- [ ] **Step 4: Verify GREEN**

Run:

```bash
npm test -- tests/backend/terrainState.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/backend/terrainState.ts tests/backend/terrainState.test.ts src/backend/proto
git commit -m "feat: add frontend layered terrain state"
```

### Task 9: Renderer Uses Backend Terrain Truth

**Files:**
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Add failing smoke assertion**

In `tests/e2e/render-smoke.spec.ts`, assert the renderer reports backend terrain source:

```ts
expect(state.city.terrainSource).toBe('backend-layered');
expect(state.city.layeredTerrain.loadedTiles).toBeGreaterThan(0);
expect(state.city.layeredTerrain.loadedTiles).toBeLessThanOrEqual(256 * 256);
```

- [ ] **Step 2: Run smoke to verify RED**

Run with backend/preview running:

```bash
npx playwright test tests/e2e/render-smoke.spec.ts --project=chromium
```

Expected: FAIL because diagnostics do not expose `terrainSource`.

- [ ] **Step 3: Switch render source**

Modify `src/main.ts`:

- initialize a `TerrainState`
- fetch/subscribe visible chunk snapshots
- build roads/rails/buildings/trees/details draw loops from `TerrainState`
- remove runtime rendering dependence on `zurichWorld`, `zurichTransport`, and `zurichPlacement`

The renderer-side mapping should use this shape:

```ts
function drawLayeredTile(coord: Coord, tile: TerrainTile): void {
  if (tile.base === 'Park' || tile.base === 'Forest' || tile.base === 'Reserve') drawTileFill(coord, MAP_PARK, 0.82);
  if (tile.base === 'Plaza') drawTileFill(coord, MAP_PLAZA, 0.72);
  if (tile.base === 'Water' || tile.base === 'Riverbank') drawTileFill(coord, tile.base === 'Riverbank' ? MAP_RIVERBANK : MAP_WATER, 0.96);
  if (tile.surface === 'Street' || tile.surface === 'Bridge' || tile.surface === 'RailCrossing') drawRoadFromMask(coord, tile);
  if (tile.surface === 'Rail' || tile.surface === 'RailCrossing') drawRailFromMask(coord, tile);
  if (tile.cover === 'Building') drawBackendBuilding(coord, tile.display);
  if (tile.cover === 'Tree') drawTree(coord);
  if (tile.cover === 'Detail') drawBackendDetail(coord, tile.display);
}
```

Keep visual style functions (`drawTileFill`, `drawCar`, `drawPedestrian`) but feed them backend tiles.

- [ ] **Step 4: Add diagnostics**

In `render_game_to_text`, add:

```ts
terrainSource: 'backend-layered',
layeredTerrain: {
  loadedTiles: terrainState.tiles.size,
  loadedChunks: terrainState.loadedChunks.size,
},
```

- [ ] **Step 5: Verify smoke GREEN**

Run:

```bash
npm run build
npx playwright test tests/e2e/render-smoke.spec.ts --project=chromium
```

Expected: build passes and smoke passes.

- [ ] **Step 6: Commit**

```bash
git add src/main.ts tests/e2e/render-smoke.spec.ts
git commit -m "feat: render map from backend layered terrain"
```

### Task 10: Legacy Exit Checks and Full Verification

**Files:**
- Create: `tests/render/noLegacyTerrainRuntime.test.ts`
- Modify: `progress.md`

- [ ] **Step 1: Add legacy runtime guard test**

Create `tests/render/noLegacyTerrainRuntime.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';

describe('no legacy terrain runtime truth', () => {
  it('does not use Zurich frontend builders as runtime render authority', () => {
    const main = readFileSync('src/main.ts', 'utf8');

    expect(main).not.toContain('buildZurichWorld({');
    expect(main).not.toContain('buildZurichTransport(');
    expect(main).not.toContain('buildZurichPlacement(');
  });
});
```

- [ ] **Step 2: Run test to verify GREEN after renderer migration**

Run:

```bash
npm test -- tests/render/noLegacyTerrainRuntime.test.ts
```

Expected: PASS.

- [ ] **Step 3: Run full frontend verification**

Run:

```bash
npm test
npm run build
npx playwright test tests/e2e/render-smoke.spec.ts --project=chromium
```

Expected:

- Vitest all pass
- build exits 0
- render smoke passes

- [ ] **Step 4: Run full backend verification**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: all backend workspace tests pass.

- [ ] **Step 5: Update progress**

Add a new top entry to `progress.md`:

```text
2026-05-27T00:00:00.000Z - Layered terrain seed: migrated the physical Zurich map to backend-authoritative layered tile records (`base + surface + cover + display/zone/masks`) generated from the existing Zurich builders as a deterministic seed artifact. Fresh backend worlds hydrate chunks from the seed, chunk snapshots expose layered tile data, and the Mini Motorways renderer draws visible terrain/transport/cover from backend chunks instead of frontend-local placement. Scope stayed terrain-only: no companies, homes, workplaces, ownership, jobs, production, money, or ledger fields. Verification: cargo workspace tests passed, Vitest passed, build passed, and Playwright render smoke passed.
```

Use the actual current UTC timestamp.

- [ ] **Step 6: Commit**

```bash
git add tests/render/noLegacyTerrainRuntime.test.ts progress.md
git commit -m "test: guard backend terrain authority"
```

### Task 11: Cleanup Review

**Files:**
- Inspect all files touched by Tasks 1-10.

- [ ] **Step 1: Run targeted greps**

Run:

```bash
rg -n "TileKind|TileMutation|SetTileKind|TileKindSet|buildZurichWorld\\(|buildZurichTransport\\(|buildZurichPlacement\\(" backend src tests
```

Expected:

- `TileKind`, `TileMutation`, `SetTileKind`, `TileKindSet` do not appear in production backend/protocol code except migration comments or deleted references in old docs.
- `buildZurichWorld`, `buildZurichTransport`, and `buildZurichPlacement` may appear in seed generation code and seed tests, but not in `src/main.ts`.

- [ ] **Step 2: Inspect git diff**

Run:

```bash
git diff --stat HEAD~10..HEAD
git diff HEAD~10..HEAD -- backend/crates/protocol/proto/abutown.proto backend/crates/sim-core/src/tile.rs src/main.ts
```

Expected: diff shows layered terrain migration only, no economy/domain fields.

- [ ] **Step 3: Final commit if cleanup edits were needed**

If Step 1 or Step 2 required edits, commit them:

```bash
git add backend src tests docs progress.md package.json data/city scripts
git commit -m "chore: clean layered terrain migration"
```

If no edits were needed, do not create an empty commit.
