# Zurich River City World Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first implementation slice of the approved Zurich-inspired flat OpenGFX river city: broad OpenGFX asset import, a deterministic 256 by 256 world model, validated city placement, and integration into the current Canvas demo without changing existing camera or vehicle mechanics.

**Architecture:** Add pure city-generation modules under `src/city/world*` and keep `src/main.ts` as the Canvas runtime. Generate a 256 by 256 world from semantic zones, then adapt that world into the existing road, rail, building, tree, and diagnostic structures. Add an OpenGFX import script and generated catalog so the renderer can use much broader asset coverage without hard-coding every sheet manually.

**Tech Stack:** Vite, TypeScript, Canvas 2D, Vitest, Playwright, Node.js asset import script, OpenGFX/OpenGFX2 PNG assets.

---

## File Structure

- Create `scripts/import-opengfx-assets.mjs`: imports broad OpenGFX2 PNG coverage into `public/opengfx2/all`, copies license files, and writes a generated TypeScript catalog.
- Create `src/assets/opengfxCatalog.generated.ts`: generated catalog of imported OpenGFX image assets.
- Create `src/assets/opengfxCatalog.ts`: small hand-written helpers for semantic filtering and fallback categories.
- Create `tests/render/opengfxCatalog.test.ts`: validates broad catalog coverage and semantic category lookup.
- Create `src/city/worldTypes.ts`: shared world, zone, terrain, transport, placement, and diagnostic types.
- Create `src/city/zurichWorld.ts`: deterministic 256 by 256 flat river-city layout and zone generation.
- Create `src/city/zurichTransport.ts`: roads, rail, bridge crossings, rail crossings, and masks.
- Create `src/city/zurichPlacement.ts`: buildings, trees, forests, parks, industry, detail placement, and reserved areas.
- Create `src/city/zurichValidation.ts`: invariant checks and category-coverage diagnostics.
- Create `tests/city/zurichWorld.test.ts`: deterministic world-size, zone, terrain, river, forest, and reserve tests.
- Create `tests/city/zurichTransport.test.ts`: road, rail, bridge, crossing, and overlap tests.
- Create `tests/city/zurichPlacement.test.ts`: building, tree, detail, frontage, density, and asset diversity tests.
- Modify `src/main.ts`: use generated Zurich world data while preserving the existing Canvas runtime, camera behavior, vehicle animation, and render order.
- Modify `tests/e2e/render-smoke.spec.ts`: assert Zurich-world diagnostics and visual readiness.
- Modify `package.json`: add `assets:opengfx` script.

## Scope Boundary

This plan implements the first playable visual city-world slice. It does not implement multiplayer persistence, 2000-player simulation, full chunk streaming, or terrain height. It prepares the world for those later systems by using 256 by 256 dimensions and 32 by 32 chunk-compatible data.

### Task 1: Broad OpenGFX Asset Import And Catalog

**Files:**
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-opengfx-assets.mjs`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.generated.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/opengfxCatalog.test.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/package.json`

- [ ] **Step 1: Write the failing catalog tests**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/render/opengfxCatalog.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { opengfxAssets } from '../../src/assets/opengfxCatalog.generated';
import { assetsByCategory, getAssetsForCategory } from '../../src/assets/opengfxCatalog';

describe('OpenGFX catalog', () => {
  it('contains broad generated OpenGFX coverage', () => {
    expect(opengfxAssets.length).toBeGreaterThan(40);
    expect(new Set(opengfxAssets.map((asset) => asset.category)).size).toBeGreaterThanOrEqual(8);
  });

  it('exposes semantic categories for city composition', () => {
    expect(getAssetsForCategory('terrain').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('water').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('road').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('rail').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('building').length).toBeGreaterThan(0);
    expect(getAssetsForCategory('tree').length).toBeGreaterThan(0);
  });

  it('returns an empty list for unknown categories instead of throwing', () => {
    expect(assetsByCategory().get('missing-category')).toBeUndefined();
    expect(getAssetsForCategory('missing-category')).toEqual([]);
  });
});
```

- [ ] **Step 2: Run the failing catalog test**

Run:

```bash
npm test -- tests/render/opengfxCatalog.test.ts
```

Expected: fail because `src/assets/opengfxCatalog.generated.ts` and `src/assets/opengfxCatalog.ts` do not exist.

- [ ] **Step 3: Add the import script and npm command**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/scripts/import-opengfx-assets.mjs`:

```js
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, rmSync, copyFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, extname, join, relative } from 'node:path';
import { tmpdir } from 'node:os';

const repoUrl = 'https://github.com/OpenTTD/OpenGFX2.git';
const root = process.cwd();
const temp = join(tmpdir(), 'abutown-opengfx2-import');
const publicRoot = join(root, 'public', 'opengfx2', 'all');
const licenseRoot = join(root, 'public', 'opengfx2', 'licenses');
const generatedPath = join(root, 'src', 'assets', 'opengfxCatalog.generated.ts');

rmSync(temp, { recursive: true, force: true });
mkdirSync(dirname(generatedPath), { recursive: true });
mkdirSync(publicRoot, { recursive: true });
mkdirSync(licenseRoot, { recursive: true });

execFileSync('git', ['clone', '--depth', '1', '--filter=blob:none', '--sparse', repoUrl, temp], { stdio: 'inherit' });
execFileSync('git', ['-C', temp, 'sparse-checkout', 'set', 'LICENSE', 'credits.md', 'graphics'], { stdio: 'inherit' });

for (const file of ['LICENSE', 'credits.md']) {
  const source = join(temp, file);
  if (existsSync(source)) copyFileSync(source, join(licenseRoot, file));
}

const pngs = [];
function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    if (entry.isFile() && extname(entry.name).toLowerCase() === '.png') pngs.push(full);
  }
}
walk(join(temp, 'graphics'));

const usefulPngs = pngs.filter((file) => {
  const normalized = file.replaceAll('\\', '/');
  return normalized.includes('/64/') || normalized.includes('/32bpp') || normalized.includes('_shape.png') || normalized.includes('overlayalpha');
});

const assets = [];
for (const source of usefulPngs) {
  const rel = relative(join(temp, 'graphics'), source).replaceAll('\\', '/');
  const outputName = rel.replaceAll('/', '__');
  const destination = join(publicRoot, outputName);
  copyFileSync(source, destination);
  assets.push({
    key: outputName.replace(/\.png$/u, ''),
    path: `/opengfx2/all/${outputName}`,
    sourcePath: `graphics/${rel}`,
    fileName: basename(source),
    category: categorize(rel),
  });
}

assets.sort((a, b) => a.key.localeCompare(b.key));

writeFileSync(generatedPath, `export type OpenGfxAssetCategory =
  | 'terrain'
  | 'water'
  | 'road'
  | 'rail'
  | 'bridge'
  | 'building'
  | 'tree'
  | 'vehicle'
  | 'industry'
  | 'station'
  | 'decor'
  | 'unknown';

export type OpenGfxAsset = {
  key: string;
  path: string;
  sourcePath: string;
  fileName: string;
  category: OpenGfxAssetCategory;
};

export const opengfxAssets: OpenGfxAsset[] = ${JSON.stringify(assets, null, 2)} as const satisfies OpenGfxAsset[];
`);

console.log(`Imported ${assets.length} OpenGFX assets into ${publicRoot}`);

function categorize(rel) {
  const value = rel.toLowerCase();
  if (value.includes('terrain') || value.includes('ground') || value.includes('landscape')) return 'terrain';
  if (value.includes('water') || value.includes('river') || value.includes('canal')) return 'water';
  if (value.includes('bridge')) return 'bridge';
  if (value.includes('road') || value.includes('street')) return 'road';
  if (value.includes('rail') || value.includes('train') || value.includes('track')) return 'rail';
  if (value.includes('station')) return 'station';
  if (value.includes('town') || value.includes('house') || value.includes('office') || value.includes('church')) return 'building';
  if (value.includes('tree') || value.includes('forest')) return 'tree';
  if (value.includes('vehicle') || value.includes('bus') || value.includes('lorry') || value.includes('truck')) return 'vehicle';
  if (value.includes('industry') || value.includes('industrial')) return 'industry';
  if (value.includes('object') || value.includes('furniture') || value.includes('fence')) return 'decor';
  return 'unknown';
}
```

Modify `/Users/ramonfuglister/Desktop/Coding/abutown/package.json` scripts:

```json
"assets:opengfx": "node scripts/import-opengfx-assets.mjs"
```

- [ ] **Step 4: Add catalog helper implementation**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/assets/opengfxCatalog.ts`:

```ts
import { opengfxAssets, type OpenGfxAsset, type OpenGfxAssetCategory } from './opengfxCatalog.generated';

export function assetsByCategory(): Map<OpenGfxAssetCategory | string, OpenGfxAsset[]> {
  const result = new Map<OpenGfxAssetCategory | string, OpenGfxAsset[]>();
  for (const asset of opengfxAssets) {
    const assets = result.get(asset.category) ?? [];
    assets.push(asset);
    result.set(asset.category, assets);
  }
  return result;
}

export function getAssetsForCategory(category: OpenGfxAssetCategory | string): OpenGfxAsset[] {
  return assetsByCategory().get(category) ?? [];
}

export function firstAssetPath(category: OpenGfxAssetCategory | string, fallback: string): string {
  return getAssetsForCategory(category)[0]?.path ?? fallback;
}
```

- [ ] **Step 5: Run the importer**

Run:

```bash
npm run assets:opengfx
```

Expected: command exits `0`, `public/opengfx2/all` contains many PNG files, and `src/assets/opengfxCatalog.generated.ts` exists.

- [ ] **Step 6: Verify catalog tests pass**

Run:

```bash
npm test -- tests/render/opengfxCatalog.test.ts
```

Expected: pass.

- [ ] **Step 7: Commit**

Run:

```bash
git add package.json package-lock.json scripts/import-opengfx-assets.mjs src/assets/opengfxCatalog.generated.ts src/assets/opengfxCatalog.ts tests/render/opengfxCatalog.test.ts public/opengfx2/all public/opengfx2/licenses
git commit -m "feat: import broad OpenGFX asset catalog"
```

Expected: commit succeeds.

### Task 2: 256 By 256 Zurich World Types And Layout

**Files:**
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/worldTypes.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichWorld.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/city/zurichWorld.test.ts`

- [ ] **Step 1: Write failing world layout tests**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/city/zurichWorld.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';

describe('buildZurichWorld', () => {
  it('builds a deterministic flat 256 by 256 city region', () => {
    const first = buildZurichWorld({ seed: 1848 });
    const second = buildZurichWorld({ seed: 1848 });

    expect(first).toEqual(second);
    expect(first.width).toBe(256);
    expect(first.height).toBe(256);
    expect(first.chunkSize).toBe(32);
    expect(first.terrain.size).toBe(256 * 256);
    expect(first.zones.length).toBeGreaterThanOrEqual(10);
  });

  it('contains river, old town, rail center, forest, industry, residential, and reserve zones', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const zoneKinds = new Set(world.zones.map((zone) => zone.kind));

    expect(zoneKinds.has('river')).toBe(true);
    expect(zoneKinds.has('old-town')).toBe(true);
    expect(zoneKinds.has('rail-center')).toBe(true);
    expect(zoneKinds.has('forest')).toBe(true);
    expect(zoneKinds.has('industry')).toBe(true);
    expect(zoneKinds.has('residential')).toBe(true);
    expect(zoneKinds.has('reserve')).toBe(true);
  });

  it('keeps the terrain flat while reserving meaningful water and forest coverage', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const terrainValues = [...world.terrain.values()];

    expect(new Set(terrainValues.map((tile) => tile.elevation))).toEqual(new Set([0]));
    expect(terrainValues.filter((tile) => tile.kind === 'water').length).toBeGreaterThan(1800);
    expect(terrainValues.filter((tile) => tile.kind === 'forest').length).toBeGreaterThan(4500);
    expect(terrainValues.filter((tile) => tile.kind === 'reserve').length).toBeGreaterThan(2500);
  });
});
```

- [ ] **Step 2: Run the failing world tests**

Run:

```bash
npm test -- tests/city/zurichWorld.test.ts
```

Expected: fail because `src/city/zurichWorld.ts` does not exist.

- [ ] **Step 3: Add shared world types**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/worldTypes.ts`:

```ts
export type Coord = { x: number; y: number };

export type ZurichTerrainKind = 'grass' | 'water' | 'riverbank' | 'park' | 'forest' | 'reserve' | 'plaza';

export type ZurichZoneKind =
  | 'river'
  | 'old-town'
  | 'rail-center'
  | 'residential'
  | 'forest'
  | 'park'
  | 'industry'
  | 'reserve'
  | 'civic'
  | 'waterfront';

export type ZurichTerrainTile = {
  coord: Coord;
  kind: ZurichTerrainKind;
  elevation: 0;
  zoneId?: string;
};

export type ZurichZone = {
  id: string;
  kind: ZurichZoneKind;
  name: string;
  center: Coord;
  radius: number;
  density: number;
};

export type ZurichWorld = {
  id: string;
  seed: number;
  width: number;
  height: number;
  chunkSize: number;
  zones: ZurichZone[];
  terrain: Map<string, ZurichTerrainTile>;
  river: Coord[];
};

export type ZurichRoadKind = 'street' | 'bridge';

export type ZurichRoadTile = {
  coord: Coord;
  kind: ZurichRoadKind;
  mask: number;
};

export type ZurichRailTile = {
  coord: Coord;
  mask: number;
};

export type ZurichBuildingSheet =
  | 'houses'
  | 'oldhouses'
  | 'cottages'
  | 'townhouses'
  | 'shops'
  | 'flats'
  | 'office'
  | 'modern'
  | 'tower'
  | 'church';

export type ZurichBuilding = {
  coord: Coord;
  sheet: ZurichBuildingSheet;
  frame: number;
  zoneId: string;
};

export type ZurichDetail = {
  coord: Coord;
  category: 'tree' | 'park' | 'civic' | 'industry' | 'decor';
  assetCategory: string;
};

export type ZurichValidationResult = {
  valid: boolean;
  errors: string[];
  stats: Record<string, number>;
};

export function key(coord: Coord): string {
  return `${coord.x}:${coord.y}`;
}

export function parseKey(value: string): Coord {
  const [x, y] = value.split(':').map(Number);
  return { x, y };
}

export function inside(coord: Coord, width: number, height: number): boolean {
  return coord.x >= 0 && coord.y >= 0 && coord.x < width && coord.y < height;
}

export function distance(a: Coord, b: Coord): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}
```

- [ ] **Step 4: Implement deterministic Zurich world layout**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichWorld.ts`:

```ts
import { distance, key, type Coord, type ZurichTerrainKind, type ZurichTerrainTile, type ZurichWorld, type ZurichZone } from './worldTypes';

export type ZurichWorldOptions = {
  seed: number;
};

const WIDTH = 256;
const HEIGHT = 256;
const CHUNK_SIZE = 32;

const zones: ZurichZone[] = [
  { id: 'zone:old-town-west', kind: 'old-town', name: 'Old Town West', center: { x: 112, y: 112 }, radius: 22, density: 0.95 },
  { id: 'zone:old-town-east', kind: 'old-town', name: 'Old Town East', center: { x: 139, y: 112 }, radius: 20, density: 0.92 },
  { id: 'zone:main-station', kind: 'rail-center', name: 'Main Station Quarter', center: { x: 118, y: 145 }, radius: 26, density: 0.9 },
  { id: 'zone:civic', kind: 'civic', name: 'Civic Garden', center: { x: 151, y: 143 }, radius: 18, density: 0.66 },
  { id: 'zone:west-residential', kind: 'residential', name: 'West Residential', center: { x: 74, y: 125 }, radius: 31, density: 0.62 },
  { id: 'zone:north-residential', kind: 'residential', name: 'North Residential', center: { x: 129, y: 72 }, radius: 30, density: 0.58 },
  { id: 'zone:east-residential', kind: 'residential', name: 'East Residential', center: { x: 182, y: 119 }, radius: 34, density: 0.6 },
  { id: 'zone:south-village', kind: 'residential', name: 'South Village', center: { x: 100, y: 196 }, radius: 30, density: 0.48 },
  { id: 'zone:industry', kind: 'industry', name: 'Rail Industry Edge', center: { x: 175, y: 184 }, radius: 28, density: 0.54 },
  { id: 'zone:north-forest', kind: 'forest', name: 'North Forest', center: { x: 58, y: 48 }, radius: 45, density: 0.18 },
  { id: 'zone:east-forest', kind: 'forest', name: 'East Forest', center: { x: 220, y: 72 }, radius: 38, density: 0.18 },
  { id: 'zone:south-forest', kind: 'forest', name: 'South Forest', center: { x: 205, y: 222 }, radius: 42, density: 0.18 },
  { id: 'zone:river-park', kind: 'park', name: 'River Park', center: { x: 144, y: 160 }, radius: 22, density: 0.24 },
  { id: 'zone:west-reserve', kind: 'reserve', name: 'West Expansion Reserve', center: { x: 45, y: 184 }, radius: 28, density: 0.12 },
  { id: 'zone:south-reserve', kind: 'reserve', name: 'South Expansion Reserve', center: { x: 142, y: 226 }, radius: 24, density: 0.12 },
];

export function buildZurichWorld(options: ZurichWorldOptions): ZurichWorld {
  const river = buildRiver();
  const riverKeys = new Set(river.map(key));
  const terrain = new Map<string, ZurichTerrainTile>();

  for (let y = 0; y < HEIGHT; y += 1) {
    const riverX = riverCenterX(y);
    for (let x = 0; x < WIDTH; x += 1) {
      const coord = { x, y };
      const zone = nearestZone(coord);
      const riverDistance = Math.abs(x - riverX);
      const kind = terrainKind(coord, riverDistance, riverKeys, zone);
      terrain.set(key(coord), { coord, kind, elevation: 0, zoneId: zone?.id });
    }
  }

  return {
    id: 'zurich-river-city-v1',
    seed: options.seed,
    width: WIDTH,
    height: HEIGHT,
    chunkSize: CHUNK_SIZE,
    zones,
    terrain,
    river,
  };
}

function buildRiver(): Coord[] {
  const river: Coord[] = [];
  for (let y = 0; y < HEIGHT; y += 1) {
    const center = riverCenterX(y);
    for (let dx = -3; dx <= 3; dx += 1) river.push({ x: center + dx, y });
  }
  return river;
}

function riverCenterX(y: number): number {
  return 128 + Math.round(Math.sin(y / 23) * 12 + Math.sin(y / 61) * 7);
}

function terrainKind(coord: Coord, riverDistance: number, riverKeys: ReadonlySet<string>, zone?: ZurichZone): ZurichTerrainKind {
  if (riverKeys.has(key(coord))) return 'water';
  if (riverDistance <= 5) return 'riverbank';
  if (zone?.kind === 'forest' && distance(coord, zone.center) <= zone.radius) return 'forest';
  if (zone?.kind === 'reserve' && distance(coord, zone.center) <= zone.radius) return 'reserve';
  if (zone?.kind === 'park' && distance(coord, zone.center) <= zone.radius) return 'park';
  if ((zone?.kind === 'old-town' || zone?.kind === 'civic') && distance(coord, zone.center) < 4) return 'plaza';
  return 'grass';
}

function nearestZone(coord: Coord): ZurichZone | undefined {
  return zones.reduce<ZurichZone | undefined>((best, zone) => {
    if (!best) return zone;
    return distance(coord, zone.center) / zone.radius < distance(coord, best.center) / best.radius ? zone : best;
  }, undefined);
}
```

- [ ] **Step 5: Verify world tests pass**

Run:

```bash
npm test -- tests/city/zurichWorld.test.ts
```

Expected: pass.

- [ ] **Step 6: Commit**

Run:

```bash
git add src/city/worldTypes.ts src/city/zurichWorld.ts tests/city/zurichWorld.test.ts
git commit -m "feat: add deterministic Zurich river world layout"
```

Expected: commit succeeds.

### Task 3: Zurich Transport Network

**Files:**
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichTransport.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/city/zurichTransport.test.ts`

- [ ] **Step 1: Write failing transport tests**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/city/zurichTransport.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';

describe('buildZurichTransport', () => {
  it('creates roads, rail, bridges, and intentional crossings without accidental overlap', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

    expect(transport.roads.size).toBeGreaterThan(1200);
    expect(transport.rails.size).toBeGreaterThan(180);
    expect(transport.bridges.size).toBeGreaterThanOrEqual(3);
    expect(transport.bridges.size).toBeLessThanOrEqual(5);
    expect(transport.railCrossings.size).toBeGreaterThanOrEqual(1);

    let accidentalOverlap = 0;
    for (const roadKey of transport.roads.keys()) {
      if (transport.rails.has(roadKey) && !transport.railCrossings.has(roadKey)) accidentalOverlap += 1;
    }
    expect(accidentalOverlap).toBe(0);
  });

  it('places bridge road tiles only on water or riverbank terrain', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);

    for (const bridgeKey of transport.bridges) {
      const terrain = world.terrain.get(bridgeKey)?.kind;
      expect(['water', 'riverbank']).toContain(terrain);
    }
  });
});
```

- [ ] **Step 2: Run the failing transport test**

Run:

```bash
npm test -- tests/city/zurichTransport.test.ts
```

Expected: fail because `src/city/zurichTransport.ts` does not exist.

- [ ] **Step 3: Implement transport generation**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichTransport.ts`:

```ts
import { inside, key, parseKey, type Coord, type ZurichRailTile, type ZurichRoadKind, type ZurichRoadTile, type ZurichWorld } from './worldTypes';

const NORTH = 1;
const EAST = 2;
const SOUTH = 4;
const WEST = 8;

export type ZurichTransport = {
  roads: Map<string, ZurichRoadTile>;
  rails: Map<string, ZurichRailTile>;
  bridges: Set<string>;
  railCrossings: Set<string>;
  arterialPaths: Coord[][];
  railPaths: Coord[][];
};

export function buildZurichTransport(world: ZurichWorld): ZurichTransport {
  const railPaths = buildRailPaths(world);
  const railPoints = new Set(railPaths.flatMap((path) => path.map(key)));
  const railCrossings = new Set(['118:154', '151:180']);
  const roadKinds = new Map<string, ZurichRoadKind>();
  const bridgeKeys = new Set<string>();
  const arterialPaths = buildArterialPaths(world);

  const addRoad = (coord: Coord) => {
    if (!inside(coord, world.width, world.height)) return;
    const tileKey = key(coord);
    if (railPoints.has(tileKey) && !railCrossings.has(tileKey)) return;
    const terrain = world.terrain.get(tileKey)?.kind;
    const kind: ZurichRoadKind = terrain === 'water' || terrain === 'riverbank' ? 'bridge' : 'street';
    roadKinds.set(tileKey, kind);
    if (kind === 'bridge') bridgeKeys.add(tileKey);
  };

  for (const path of arterialPaths) for (const coord of path) addRoad(coord);
  for (const zone of world.zones) {
    if (zone.kind === 'forest' || zone.kind === 'river') continue;
    addDistrictStreetPattern(world, addRoad, zone.center, zone.radius, zone.density);
  }

  const roads = new Map<string, ZurichRoadTile>();
  for (const [tileKey, kind] of roadKinds) {
    const coord = parseKey(tileKey);
    roads.set(tileKey, { coord, kind, mask: maskFor(roadKinds, coord) });
  }

  const rails = new Map<string, ZurichRailTile>();
  for (const tileKey of railPoints) {
    const coord = parseKey(tileKey);
    rails.set(tileKey, { coord, mask: maskForRail(railPoints, coord) });
  }

  return { roads, rails, bridges: bridgeKeys, railCrossings, arterialPaths, railPaths };
}

function buildArterialPaths(world: ZurichWorld): Coord[][] {
  return [
    route([{ x: 0, y: 128 }, { x: 73, y: 124 }, { x: 112, y: 112 }, { x: 139, y: 112 }, { x: 206, y: 116 }, { x: world.width - 1, y: 121 }]),
    route([{ x: 104, y: 0 }, { x: 111, y: 78 }, { x: 118, y: 145 }, { x: 101, y: 196 }, { x: 88, y: world.height - 1 }]),
    route([{ x: 43, y: 184 }, { x: 100, y: 196 }, { x: 151, y: 180 }, { x: 175, y: 184 }, { x: world.width - 1, y: 198 }]),
    route([{ x: 20, y: 74 }, { x: 74, y: 125 }, { x: 118, y: 145 }, { x: 151, y: 143 }, { x: 220, y: 160 }]),
  ];
}

function buildRailPaths(world: ZurichWorld): Coord[][] {
  return [
    route([{ x: 0, y: 154 }, { x: 118, y: 154 }, { x: 175, y: 184 }, { x: world.width - 1, y: 191 }]),
    route([{ x: 118, y: 154 }, { x: 126, y: world.height - 1 }]),
  ];
}

function addDistrictStreetPattern(world: ZurichWorld, addRoad: (coord: Coord) => void, center: Coord, radius: number, density: number): void {
  const arm = Math.max(8, Math.round(radius * (density > 0.8 ? 0.95 : 0.65)));
  for (const coord of route([{ x: center.x - arm, y: center.y }, { x: center.x + arm, y: center.y }])) addRoad(coord);
  for (const coord of route([{ x: center.x, y: center.y - Math.round(arm * 0.55) }, { x: center.x, y: center.y + Math.round(arm * 0.55) }])) addRoad(coord);

  if (density > 0.72) {
    for (const offset of [-9, 9]) {
      for (const coord of route([{ x: center.x - arm + 4, y: center.y + offset }, { x: center.x + arm - 4, y: center.y + offset }])) addRoad(coord);
      for (const coord of route([{ x: center.x + offset, y: center.y - arm + 6 }, { x: center.x + offset, y: center.y + arm - 6 }])) addRoad(coord);
    }
  }
}

function route(points: Coord[]): Coord[] {
  const result: Coord[] = [];
  for (let index = 1; index < points.length; index += 1) {
    const segment = cardinalLinePath(points[index - 1], points[index]);
    result.push(...(index === 1 ? segment : segment.slice(1)));
  }
  return result;
}

function cardinalLinePath(from: Coord, to: Coord): Coord[] {
  const result: Coord[] = [];
  let x = from.x;
  let y = from.y;
  result.push({ x, y });
  while (x !== to.x) {
    x += Math.sign(to.x - x);
    result.push({ x, y });
  }
  while (y !== to.y) {
    y += Math.sign(to.y - y);
    result.push({ x, y });
  }
  return result;
}

function maskFor(points: ReadonlyMap<string, unknown>, coord: Coord): number {
  return (
    (points.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (points.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (points.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (points.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}

function maskForRail(points: ReadonlySet<string>, coord: Coord): number {
  return (
    (points.has(key({ x: coord.x, y: coord.y - 1 })) ? NORTH : 0) |
    (points.has(key({ x: coord.x + 1, y: coord.y })) ? EAST : 0) |
    (points.has(key({ x: coord.x, y: coord.y + 1 })) ? SOUTH : 0) |
    (points.has(key({ x: coord.x - 1, y: coord.y })) ? WEST : 0)
  );
}
```

- [ ] **Step 4: Verify transport tests pass**

Run:

```bash
npm test -- tests/city/zurichTransport.test.ts
```

Expected: pass.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/city/zurichTransport.ts tests/city/zurichTransport.test.ts
git commit -m "feat: generate Zurich transport network"
```

Expected: commit succeeds.

### Task 4: Zurich Placement And Validation

**Files:**
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichPlacement.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichValidation.ts`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/city/zurichPlacement.test.ts`

- [ ] **Step 1: Write failing placement and validation tests**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/tests/city/zurichPlacement.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { buildZurichWorld } from '../../src/city/zurichWorld';
import { buildZurichTransport } from '../../src/city/zurichTransport';
import { buildZurichPlacement } from '../../src/city/zurichPlacement';
import { validateZurichCity } from '../../src/city/zurichValidation';
import { key } from '../../src/city/worldTypes';

describe('buildZurichPlacement', () => {
  it('places varied buildings, forests, and reserves without hard-rule conflicts', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);
    const validation = validateZurichCity(world, transport, placement);

    expect(validation.valid).toBe(true);
    expect(validation.errors).toEqual([]);
    expect(placement.buildings.length).toBeGreaterThan(1800);
    expect(placement.trees.length).toBeGreaterThan(3200);
    expect(placement.details.length).toBeGreaterThan(120);
    expect(placement.reserveTiles.size).toBeGreaterThan(2500);
    expect(new Set(placement.buildings.map((building) => building.sheet)).size).toBeGreaterThanOrEqual(8);
  });

  it('keeps buildings off water, roads, and rails', () => {
    const world = buildZurichWorld({ seed: 1848 });
    const transport = buildZurichTransport(world);
    const placement = buildZurichPlacement(world, transport);

    for (const building of placement.buildings) {
      const tileKey = key(building.coord);
      expect(world.terrain.get(tileKey)?.kind).not.toBe('water');
      expect(transport.roads.has(tileKey)).toBe(false);
      expect(transport.rails.has(tileKey)).toBe(false);
    }
  });
});
```

- [ ] **Step 2: Run the failing placement test**

Run:

```bash
npm test -- tests/city/zurichPlacement.test.ts
```

Expected: fail because placement and validation modules do not exist.

- [ ] **Step 3: Implement placement**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichPlacement.ts`:

```ts
import { distance, inside, key, parseKey, type Coord, type ZurichBuilding, type ZurichBuildingSheet, type ZurichDetail, type ZurichWorld, type ZurichZone } from './worldTypes';
import type { ZurichTransport } from './zurichTransport';

export type ZurichPlacement = {
  buildings: ZurichBuilding[];
  trees: Coord[];
  details: ZurichDetail[];
  reserveTiles: Set<string>;
};

const sheetPools: Record<ZurichZone['kind'], ZurichBuildingSheet[]> = {
  river: ['townhouses'],
  'old-town': ['oldhouses', 'townhouses', 'shops', 'church'],
  'rail-center': ['shops', 'flats', 'office', 'modern', 'tower'],
  residential: ['houses', 'cottages', 'oldhouses', 'townhouses'],
  forest: ['cottages'],
  park: ['church', 'oldhouses'],
  industry: ['shops', 'office', 'flats'],
  reserve: ['cottages', 'houses'],
  civic: ['church', 'office', 'shops'],
  waterfront: ['townhouses', 'shops', 'flats'],
};

export function buildZurichPlacement(world: ZurichWorld, transport: ZurichTransport): ZurichPlacement {
  const blocked = new Set<string>([...transport.roads.keys(), ...transport.rails.keys()]);
  const buildings: ZurichBuilding[] = [];
  const trees: Coord[] = [];
  const details: ZurichDetail[] = [];
  const reserveTiles = new Set<string>();

  for (const tile of world.terrain.values()) {
    if (tile.kind === 'reserve') reserveTiles.add(key(tile.coord));
    if (tile.kind === 'forest' && hash(`forest:${key(tile.coord)}`) % 3 !== 0 && !blocked.has(key(tile.coord))) trees.push(tile.coord);
    if (tile.kind === 'park' && hash(`park-tree:${key(tile.coord)}`) % 4 === 0 && !blocked.has(key(tile.coord))) trees.push(tile.coord);
  }

  for (const zone of world.zones) {
    if (zone.kind === 'forest' || zone.kind === 'river') continue;
    const candidates = frontageCandidates(world, transport, blocked, zone);
    for (const coord of candidates) {
      if (buildings.length >= 4200) break;
      if (zone.kind === 'reserve' && hash(`reserve-building:${key(coord)}`) % 9 !== 0) continue;
      if (hash(`building-density:${zone.id}:${key(coord)}`) % 100 > Math.floor(zone.density * 100)) continue;
      const sheets = sheetPools[zone.kind];
      const sheet = sheets[hash(`sheet:${zone.id}:${key(coord)}`) % sheets.length];
      buildings.push({ coord, sheet, frame: hash(`frame:${sheet}:${key(coord)}`) % frameCount(sheet), zoneId: zone.id });
      blocked.add(key(coord));
    }
  }

  for (const zone of world.zones) {
    if (zone.kind === 'civic' || zone.kind === 'park' || zone.kind === 'industry') {
      for (let index = 0; index < 18; index += 1) {
        const coord = {
          x: zone.center.x + ((hash(`detail-x:${zone.id}:${index}`) % (zone.radius * 2)) - zone.radius),
          y: zone.center.y + ((hash(`detail-y:${zone.id}:${index}`) % (zone.radius * 2)) - zone.radius),
        };
        const tileKey = key(coord);
        if (!inside(coord, world.width, world.height) || blocked.has(tileKey)) continue;
        details.push({
          coord,
          category: zone.kind === 'industry' ? 'industry' : zone.kind === 'civic' ? 'civic' : 'park',
          assetCategory: zone.kind === 'industry' ? 'industry' : 'decor',
        });
      }
    }
  }

  return { buildings, trees, details, reserveTiles };
}

function frontageCandidates(world: ZurichWorld, transport: ZurichTransport, blocked: ReadonlySet<string>, zone: ZurichZone): Coord[] {
  const candidates: Coord[] = [];
  for (const road of transport.roads.values()) {
    if (road.kind !== 'street') continue;
    for (const coord of [{ x: road.coord.x + 1, y: road.coord.y }, { x: road.coord.x, y: road.coord.y + 1 }]) {
      const tileKey = key(coord);
      const terrain = world.terrain.get(tileKey)?.kind;
      if (!inside(coord, world.width, world.height) || blocked.has(tileKey)) continue;
      if (terrain === 'water' || terrain === 'riverbank' || terrain === 'forest') continue;
      if (distance(coord, zone.center) <= zone.radius) candidates.push(coord);
    }
  }
  candidates.sort((a, b) => distance(a, zone.center) - distance(b, zone.center) || a.y - b.y || a.x - b.x);
  return candidates;
}

function frameCount(sheet: ZurichBuildingSheet): number {
  if (sheet === 'church') return 3;
  if (sheet === 'cottages') return 3;
  if (sheet === 'townhouses') return 6;
  if (sheet === 'modern') return 8;
  if (sheet === 'tower') return 12;
  return 12;
}

function hash(value: string): number {
  let result = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}
```

- [ ] **Step 4: Implement validation**

Create `/Users/ramonfuglister/Desktop/Coding/abutown/src/city/zurichValidation.ts`:

```ts
import { key, type ZurichValidationResult, type ZurichWorld } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

export function validateZurichCity(world: ZurichWorld, transport: ZurichTransport, placement: ZurichPlacement): ZurichValidationResult {
  const errors: string[] = [];
  let roadRailOverlap = 0;
  let invalidBuildings = 0;
  let bridgeErrors = 0;

  for (const roadKey of transport.roads.keys()) {
    if (transport.rails.has(roadKey) && !transport.railCrossings.has(roadKey)) roadRailOverlap += 1;
  }

  for (const bridgeKey of transport.bridges) {
    const terrain = world.terrain.get(bridgeKey)?.kind;
    if (terrain !== 'water' && terrain !== 'riverbank') bridgeErrors += 1;
  }

  for (const building of placement.buildings) {
    const tileKey = key(building.coord);
    const terrain = world.terrain.get(tileKey)?.kind;
    if (terrain === 'water' || transport.roads.has(tileKey) || transport.rails.has(tileKey)) invalidBuildings += 1;
  }

  if (roadRailOverlap > 0) errors.push(`roadRailOverlap:${roadRailOverlap}`);
  if (bridgeErrors > 0) errors.push(`bridgeErrors:${bridgeErrors}`);
  if (invalidBuildings > 0) errors.push(`invalidBuildings:${invalidBuildings}`);

  return {
    valid: errors.length === 0,
    errors,
    stats: {
      roadTiles: transport.roads.size,
      railTiles: transport.rails.size,
      bridges: transport.bridges.size,
      railCrossings: transport.railCrossings.size,
      buildings: placement.buildings.length,
      trees: placement.trees.length,
      details: placement.details.length,
      reserveTiles: placement.reserveTiles.size,
      roadRailOverlap,
      bridgeErrors,
      invalidBuildings,
    },
  };
}
```

- [ ] **Step 5: Verify placement tests pass**

Run:

```bash
npm test -- tests/city/zurichPlacement.test.ts
```

Expected: pass.

- [ ] **Step 6: Commit**

Run:

```bash
git add src/city/zurichPlacement.ts src/city/zurichValidation.ts tests/city/zurichPlacement.test.ts
git commit -m "feat: place and validate Zurich city objects"
```

Expected: commit succeeds.

### Task 5: Integrate Zurich World Into The Existing Canvas Demo

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Extend the e2e smoke test before changing the runtime**

Modify `/Users/ramonfuglister/Desktop/Coding/abutown/tests/e2e/render-smoke.spec.ts` by adding these assertions after `expect(state.city.cars).toBeGreaterThan(0);`:

```ts
  expect(state.city.worldId).toBe('zurich-river-city-v1');
  expect(state.city.width).toBe(256);
  expect(state.city.height).toBe(256);
  expect(state.city.bridges).toBeGreaterThanOrEqual(3);
  expect(state.city.railCrossings).toBeGreaterThanOrEqual(1);
  expect(state.city.trees).toBeGreaterThan(3000);
  expect(state.city.reserveTiles).toBeGreaterThan(2500);
  expect(state.city.invalidBuildings).toBe(0);
  expect(state.city.roadRailOverlap).toBe(0);
```

- [ ] **Step 2: Run the failing e2e test**

Run:

```bash
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: fail because the current runtime still exposes the old smaller city diagnostics.

- [ ] **Step 3: Import Zurich world modules into `src/main.ts`**

At the top of `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`, add:

```ts
import { buildZurichPlacement } from './city/zurichPlacement';
import { buildZurichTransport } from './city/zurichTransport';
import { validateZurichCity } from './city/zurichValidation';
import { buildZurichWorld } from './city/zurichWorld';
import { key as worldKey, type ZurichBuilding, type ZurichRailTile, type ZurichRoadTile } from './city/worldTypes';
```

- [ ] **Step 4: Replace local world constants and generated collections**

Replace the current `WIDTH`, `HEIGHT`, `terrain`, `railPaths`, `railReserved`, `railCrossings`, `roads`, `rails`, `buildings`, and `trees` initialization block with:

```ts
const zurichWorld = buildZurichWorld({ seed: 1848 });
const zurichTransport = buildZurichTransport(zurichWorld);
const zurichPlacement = buildZurichPlacement(zurichWorld, zurichTransport);
const zurichValidation = validateZurichCity(zurichWorld, zurichTransport, zurichPlacement);

const WIDTH = zurichWorld.width;
const HEIGHT = zurichWorld.height;
const terrain = new Map([...zurichWorld.terrain].map(([tileKey, tile]) => [tileKey, toRuntimeTerrain(tile.kind)]));
const roads = new Map<string, RoadTile>(
  [...zurichTransport.roads].map(([tileKey, road]) => [tileKey, { coord: road.coord, kind: road.kind, mask: road.mask }])
);
const rails = new Map<string, RailTile>(
  [...zurichTransport.rails].map(([tileKey, rail]) => [tileKey, { coord: rail.coord, mask: rail.mask }])
);
const railCrossings = zurichTransport.railCrossings;
const railReserved = new Set(rails.keys());
const railPaths = zurichTransport.railPaths;
const railYardPaths: Coord[][] = [];
const railStations = buildRailStations();
const buildings = zurichPlacement.buildings.map(toRuntimeBuilding);
const trees = zurichPlacement.trees;
const cars = buildCars();
```

Add these helper functions near the existing generation helpers:

```ts
function toRuntimeTerrain(kind: string): Terrain {
  if (kind === 'water') return 'water';
  if (kind === 'riverbank') return 'riverbank';
  if (kind === 'park' || kind === 'forest' || kind === 'reserve' || kind === 'plaza') return 'park';
  return 'grass';
}

function toRuntimeBuilding(building: ZurichBuilding): Building {
  return {
    coord: building.coord,
    sheet: building.sheet,
    frame: building.frame,
    district: building.zoneId,
  };
}
```

- [ ] **Step 5: Remove obsolete local generation calls from runtime path**

Leave old helper functions in the file only if TypeScript does not report unused errors. Remove or stop calling these local generators when they are no longer needed:

```ts
buildTerrain();
buildRoadNetwork();
buildRailPaths();
buildRailReserved();
buildRailCrossings();
buildRailNetwork();
buildBuildings();
buildTrees();
```

Keep rendering functions, camera functions, vehicle functions, sprite cleanup, and asset loading behavior unchanged.

- [ ] **Step 6: Update rail station positions for the 256 map**

Modify `buildRailStations()` in `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts` to return station tiles near the Zurich rail center:

```ts
function buildRailStations(): RailStation[] {
  return [
    { coord: { x: 116, y: 153 }, frame: 0 },
    { coord: { x: 117, y: 153 }, frame: 0 },
    { coord: { x: 118, y: 153 }, frame: 0 },
    { coord: { x: 119, y: 153 }, frame: 0 },
    { coord: { x: 120, y: 153 }, frame: 0 },
  ];
}
```

- [ ] **Step 7: Focus the initial camera on the new city center**

In `resize()`, replace the first-focus coordinate with:

```ts
const focus = iso({ x: 128, y: 132 });
```

Keep all existing camera control, zoom, damping, and bounds logic unchanged.

- [ ] **Step 8: Extend runtime diagnostics**

In the object returned by `window.render_game_to_text`, add:

```ts
worldId: zurichWorld.id,
width: WIDTH,
height: HEIGHT,
reserveTiles: zurichPlacement.reserveTiles.size,
validationErrors: zurichValidation.errors.length,
```

Also map these existing or new stats into the `city` object:

```ts
roadRailOverlap: zurichValidation.stats.roadRailOverlap,
railCrossings: zurichValidation.stats.railCrossings,
invalidBuildings: zurichValidation.stats.invalidBuildings,
```

- [ ] **Step 9: Run unit tests and build**

Run:

```bash
npm test -- tests/city/zurichWorld.test.ts tests/city/zurichTransport.test.ts tests/city/zurichPlacement.test.ts tests/render/opengfxCatalog.test.ts
npm run build
```

Expected: both commands exit `0`.

- [ ] **Step 10: Run the browser smoke test**

Run:

```bash
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: pass with no console errors.

- [ ] **Step 11: Commit**

Run:

```bash
git add src/main.ts tests/e2e/render-smoke.spec.ts
git commit -m "feat: render Zurich river city world"
```

Expected: commit succeeds.

### Task 6: Visual QA Snapshot And Final Verification

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/progress.md`
- Create: `/Users/ramonfuglister/Desktop/Coding/abutown/artifacts/abutown-zurich-river-city-2026-05-14.png`

- [ ] **Step 1: Start the local dev server**

Run:

```bash
npm run dev -- --port 5175
```

Expected: Vite serves `http://127.0.0.1:5175/`. Keep the session running while verifying.

- [ ] **Step 2: Capture a browser screenshot**

Run Playwright against `http://127.0.0.1:5175/` and save a screenshot to:

```text
/Users/ramonfuglister/Desktop/Coding/abutown/artifacts/abutown-zurich-river-city-2026-05-14.png
```

Use this script in a temporary Node or Playwright runner:

```ts
import { chromium } from '@playwright/test';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
await page.goto('http://127.0.0.1:5175/');
await page.locator('#game').waitFor({ state: 'visible' });
await page.waitForFunction(() => window.render_game_to_text?.());
await page.screenshot({ path: 'artifacts/abutown-zurich-river-city-2026-05-14.png', fullPage: true });
await browser.close();
```

- [ ] **Step 3: Inspect diagnostics**

Run in the browser page:

```ts
JSON.parse(window.render_game_to_text())
```

Expected values:

```json
{
  "city": {
    "worldId": "zurich-river-city-v1",
    "width": 256,
    "height": 256,
    "roadRailOverlap": 0,
    "invalidBuildings": 0
  }
}
```

- [ ] **Step 4: Update progress**

Append this line to `/Users/ramonfuglister/Desktop/Coding/abutown/progress.md`:

```md
2026-05-14 - Zurich river city world: imported broad OpenGFX coverage, added deterministic 256x256 flat river-city layout, integrated validated roads/rails/buildings/trees into the existing Canvas demo, and captured visual QA at artifacts/abutown-zurich-river-city-2026-05-14.png.
```

- [ ] **Step 5: Run final verification**

Run:

```bash
npm test
npm run build
npm run test:e2e
```

Expected: all commands exit `0`.

- [ ] **Step 6: Commit**

Run:

```bash
git add progress.md artifacts/abutown-zurich-river-city-2026-05-14.png
git commit -m "test: verify Zurich river city world"
```

Expected: commit succeeds.

## Self-Review Notes

- Spec coverage: this plan covers broad OpenGFX import, 256 by 256 flat world scale, river city zones, forests, parks, old town, rail center, industry, reserves, deterministic placement, validation, and browser QA.
- Deferred scope: multiplayer persistence, 2000-player family simulation, full chunk streaming, and hills remain out of scope as required by the design.
- Type consistency: the plan defines `ZurichWorld`, `ZurichTransport`, `ZurichPlacement`, and validation types before they are used by runtime integration.
- Existing behavior protection: integration explicitly preserves Canvas rendering, camera controls, vehicle movement, and visible OpenGFX style.
