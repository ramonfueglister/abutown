# Isometric City Graphics Demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a client-only Vite/PixiJS isometric graphics demo showing a beautiful river city generated from a simulation-ready city model and rendered with selected real OpenGFX2 Classic 64px source sheets.

**Architecture:** The city model is the source of truth. A deterministic generator produces terrain, river, road graph, districts, parcels, buildings, and landmarks; the PixiJS renderer projects that model into an isometric scene. Rendering, camera state, debug overlays, and future simulation state stay separated.

**Tech Stack:** Vite, TypeScript, PixiJS, Vitest, Playwright, ESLint, selected OpenGFX2 Classic 64px PNG sheets, and a semantic atlas manifest.

---

## File Structure

Create these files:

- `package.json`: project scripts and dependencies.
- `index.html`: Vite HTML entrypoint with a bare fullscreen canvas mount.
- `tsconfig.json`: strict TypeScript settings.
- `vite.config.ts`: Vite config with Vitest environment.
- `vitest.config.ts`: Vitest config.
- `playwright.config.ts`: browser smoke-test config.
- `.eslintrc.cjs`: ESLint config for TypeScript.
- `.gitignore`: ignore build output, dependencies, and `.superpowers/`.
- `src/main.ts`: app bootstrap.
- `src/app/styles.css`: fullscreen, no visible UI chrome.
- `src/city/types.ts`: simulation-ready city entity types.
- `src/city/ids.ts`: deterministic ID helpers.
- `src/city/defaultSeed.ts`: curated demo seed parameters.
- `src/city/generateCity.ts`: deterministic river city generator.
- `src/city/validateCity.ts`: graph, parcel, and sprite consistency checks.
- `src/geo/isometric.ts`: world/grid to isometric projection helpers.
- `src/geo/math.ts`: deterministic random, distance, interpolation helpers.
- `src/render/assets.ts`: 64px asset manifest and texture-loading abstraction.
- `src/render/roadSprites.ts`: graph-derived road sprite category resolver.
- `src/render/CityRenderer.ts`: PixiJS scene construction and layer rendering.
- `src/render/CameraController.ts`: smooth pan and continuous zoom.
- `src/render/debugOverlay.ts`: keyboard-toggle debug overlays, hidden by default.
- `public/opengfx2-classic/source/*.png`: selected real OpenGFX2 Classic 64px sprite sheets.
- `public/opengfx2-classic/licenses/*`: copied OpenGFX2 license and credits files.
- `src/assets/opengfx2-classic/README.md`: asset provenance and import notes.
- `src/assets/opengfx2-classic/atlas.json`: atlas manifest mapping semantic asset keys to source sprite-sheet frames.
- `tests/geo/isometric.test.ts`: coordinate transform tests.
- `tests/city/generateCity.test.ts`: determinism and model structure tests.
- `tests/city/validateCity.test.ts`: graph and parcel validation tests.
- `tests/render/roadSprites.test.ts`: road orientation to sprite category tests.
- `tests/e2e/render-smoke.spec.ts`: browser smoke test for non-empty scene and camera interaction.

Modify these files:

- `docs/superpowers/specs/2026-05-11-isometric-city-graphics-demo-design.md`: only if implementation reveals a spec correction that must be documented.

---

## Task 1: Project Scaffold

**Files:**
- Create: `package.json`
- Create: `index.html`
- Create: `tsconfig.json`
- Create: `vite.config.ts`
- Create: `vitest.config.ts`
- Create: `playwright.config.ts`
- Create: `.eslintrc.cjs`
- Create: `.gitignore`
- Create: `src/app/styles.css`
- Create: `src/main.ts`

- [ ] **Step 1: Create package and config files**

Create `package.json`:

```json
{
  "name": "abutown",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite --host 127.0.0.1",
    "build": "tsc --noEmit && vite build",
    "test": "vitest run --passWithNoTests",
    "test:watch": "vitest",
    "test:e2e": "playwright test",
    "lint": "eslint . --ext .ts"
  },
  "dependencies": {
    "pixi.js": "^8.0.0"
  },
  "devDependencies": {
    "@playwright/test": "^1.44.0",
    "@typescript-eslint/eslint-plugin": "^7.0.0",
    "@typescript-eslint/parser": "^7.0.0",
    "eslint": "^8.57.0",
    "typescript": "^5.4.0",
    "vite": "^5.2.0",
    "vitest": "^1.5.0"
  }
}
```

Create `.gitignore`:

```gitignore
node_modules/
dist/
coverage/
.vite/
.superpowers/
playwright-report/
test-results/
/tmp-opengfx2/
```

Create `.eslintrc.cjs`:

```js
module.exports = {
  root: true,
  parser: '@typescript-eslint/parser',
  plugins: ['@typescript-eslint'],
  extends: ['eslint:recommended', 'plugin:@typescript-eslint/recommended'],
  env: {
    browser: true,
    es2022: true,
    node: true,
  },
  parserOptions: {
    sourceType: 'module',
  },
  ignorePatterns: ['dist/', 'node_modules/', 'playwright-report/', 'test-results/'],
};
```

Create `index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Abutown</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

Create `tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "module": "ESNext",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "allowJs": false,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "forceConsistentCasingInFileNames": true,
    "moduleResolution": "Bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true
  },
  "include": ["src", "tests", "vite.config.ts", "vitest.config.ts", "playwright.config.ts"]
}
```

Create `vite.config.ts`:

```ts
import { defineConfig } from 'vite';

export default defineConfig({
  server: {
    host: '127.0.0.1',
    port: 5173,
  },
});
```

Create `vitest.config.ts`:

```ts
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/**/*.test.ts'],
  },
});
```

Create `playwright.config.ts`:

```ts
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:5173',
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'npm run dev',
    url: 'http://127.0.0.1:5173',
    reuseExistingServer: true,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
```

Create `src/app/styles.css`:

```css
html,
body,
#app {
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: #1b2a24;
}

canvas {
  display: block;
  width: 100vw;
  height: 100vh;
}
```

Create `src/main.ts`:

```ts
import './app/styles.css';

const appRoot = document.querySelector<HTMLDivElement>('#app');

if (!appRoot) {
  throw new Error('Missing #app root element');
}

appRoot.dataset.ready = 'true';
```

- [ ] **Step 2: Install dependencies**

Run:

```bash
npm install
```

Expected: `package-lock.json` is created and dependencies install without errors.

- [ ] **Step 3: Run the initial checks**

Run:

```bash
npm run build
npm test
```

Expected: build passes; Vitest reports no test files or no failing tests depending on Vitest version.

- [ ] **Step 4: Commit scaffold**

Run:

```bash
git add .gitignore .eslintrc.cjs index.html package.json package-lock.json tsconfig.json vite.config.ts vitest.config.ts playwright.config.ts src/app/styles.css src/main.ts
git commit -m "chore: scaffold isometric city app"
```

---

## Task 2: Geometry And Deterministic Math

**Files:**
- Create: `src/geo/isometric.ts`
- Create: `src/geo/math.ts`
- Create: `tests/geo/isometric.test.ts`

- [ ] **Step 1: Write failing coordinate tests**

Create `tests/geo/isometric.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { gridToIso, isoToGrid, worldToIso } from '../../src/geo/isometric';

describe('isometric projection', () => {
  it('projects grid coordinates to 64px isometric tile centers', () => {
    expect(gridToIso({ x: 0, y: 0 })).toEqual({ x: 0, y: 0 });
    expect(gridToIso({ x: 1, y: 0 })).toEqual({ x: 32, y: 16 });
    expect(gridToIso({ x: 0, y: 1 })).toEqual({ x: -32, y: 16 });
    expect(gridToIso({ x: 2, y: 3 })).toEqual({ x: -32, y: 80 });
  });

  it('round-trips iso coordinates back to grid coordinates', () => {
    const grid = { x: 7, y: 4 };
    expect(isoToGrid(gridToIso(grid))).toEqual(grid);
  });

  it('supports world coordinates with sub-tile precision', () => {
    expect(worldToIso({ x: 1.5, y: 0.5 })).toEqual({ x: 32, y: 32 });
  });
});
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
npm test -- tests/geo/isometric.test.ts
```

Expected: FAIL because `src/geo/isometric.ts` does not exist.

- [ ] **Step 3: Implement geometry and math helpers**

Create `src/geo/isometric.ts`:

```ts
export type GridPoint = {
  readonly x: number;
  readonly y: number;
};

export type IsoPoint = {
  readonly x: number;
  readonly y: number;
};

export const TILE_WIDTH = 64;
export const TILE_HEIGHT = 32;
export const HALF_TILE_WIDTH = TILE_WIDTH / 2;
export const HALF_TILE_HEIGHT = TILE_HEIGHT / 2;

export function gridToIso(point: GridPoint): IsoPoint {
  return {
    x: (point.x - point.y) * HALF_TILE_WIDTH,
    y: (point.x + point.y) * HALF_TILE_HEIGHT,
  };
}

export function worldToIso(point: GridPoint): IsoPoint {
  return gridToIso(point);
}

export function isoToGrid(point: IsoPoint): GridPoint {
  return {
    x: (point.y / HALF_TILE_HEIGHT + point.x / HALF_TILE_WIDTH) / 2,
    y: (point.y / HALF_TILE_HEIGHT - point.x / HALF_TILE_WIDTH) / 2,
  };
}
```

Create `src/geo/math.ts`:

```ts
export function createSeededRandom(seed: number): () => number {
  let state = seed >>> 0;

  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

export function lerp(start: number, end: number, t: number): number {
  return start + (end - start) * t;
}

export function distance(a: { readonly x: number; readonly y: number }, b: { readonly x: number; readonly y: number }): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
```

- [ ] **Step 4: Run tests and commit**

Run:

```bash
npm test -- tests/geo/isometric.test.ts
```

Expected: PASS.

Commit:

```bash
git add src/geo/isometric.ts src/geo/math.ts tests/geo/isometric.test.ts
git commit -m "feat: add isometric geometry helpers"
```

---

## Task 3: Simulation-Ready City Types

**Files:**
- Create: `src/city/types.ts`
- Create: `src/city/ids.ts`

- [ ] **Step 1: Add city model types**

Create `src/city/types.ts`:

```ts
export type EntityId = string;

export type TerrainKind = 'grass' | 'water' | 'riverbank' | 'park' | 'plaza';
export type DistrictKind = 'old-town' | 'market' | 'residential' | 'industrial' | 'civic' | 'parkland';
export type LandUse = 'residential' | 'commercial' | 'civic' | 'industrial' | 'park';
export type RoadMode = 'car' | 'pedestrian' | 'service';
export type RoadOrientation = 'northEast' | 'northWest' | 'eastWest' | 'curve' | 'intersection' | 'bridge' | 'deadEnd';
export type RoadDirection = 'oneWayForward' | 'oneWayBackward' | 'twoWay';

export type GridCoord = {
  readonly x: number;
  readonly y: number;
};

export type Tile = {
  readonly id: EntityId;
  readonly coord: GridCoord;
  readonly terrain: TerrainKind;
  readonly elevation: number;
  readonly districtId?: EntityId;
};

export type RoadNode = {
  readonly id: EntityId;
  readonly coord: GridCoord;
  readonly kind: 'junction' | 'bridge' | 'center' | 'exit';
};

export type RoadEdge = {
  readonly id: EntityId;
  readonly from: EntityId;
  readonly to: EntityId;
  readonly points: readonly GridCoord[];
  readonly direction: RoadDirection;
  readonly modes: readonly RoadMode[];
  readonly orientation: RoadOrientation;
  readonly cost: number;
};

export type Block = {
  readonly id: EntityId;
  readonly boundaryRoadIds: readonly EntityId[];
  readonly tileIds: readonly EntityId[];
  readonly districtId: EntityId;
};

export type Parcel = {
  readonly id: EntityId;
  readonly blockId: EntityId;
  readonly accessRoadId: EntityId;
  readonly landUse: LandUse;
  readonly capacityHint: number;
  readonly footprint: readonly GridCoord[];
};

export type Building = {
  readonly id: EntityId;
  readonly parcelId: EntityId;
  readonly districtId: EntityId;
  readonly assetKey: string;
  readonly footprint: readonly GridCoord[];
  readonly roleHints: readonly LandUse[];
  readonly capacityHint: number;
};

export type District = {
  readonly id: EntityId;
  readonly name: string;
  readonly kind: DistrictKind;
  readonly center: GridCoord;
  readonly density: number;
  readonly centrality: number;
};

export type Landmark = {
  readonly id: EntityId;
  readonly name: string;
  readonly coord: GridCoord;
  readonly districtId: EntityId;
  readonly assetKey: string;
};

export type City = {
  readonly id: EntityId;
  readonly seed: number;
  readonly generatorVersion: string;
  readonly width: number;
  readonly height: number;
  readonly tiles: readonly Tile[];
  readonly roadNodes: readonly RoadNode[];
  readonly roadEdges: readonly RoadEdge[];
  readonly blocks: readonly Block[];
  readonly parcels: readonly Parcel[];
  readonly buildings: readonly Building[];
  readonly districts: readonly District[];
  readonly landmarks: readonly Landmark[];
};
```

Create `src/city/ids.ts`:

```ts
export function makeId(prefix: string, parts: readonly (string | number)[]): string {
  return `${prefix}:${parts.join(':')}`;
}
```

- [ ] **Step 2: Run typecheck and commit**

Run:

```bash
npm run build
```

Expected: PASS.

Commit:

```bash
git add src/city/types.ts src/city/ids.ts
git commit -m "feat: define simulation-ready city model"
```

---

## Task 4: Deterministic River City Generator

**Files:**
- Create: `src/city/defaultSeed.ts`
- Create: `src/city/generateCity.ts`
- Create: `tests/city/generateCity.test.ts`

- [ ] **Step 1: Write generator tests**

Create `tests/city/generateCity.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { defaultCitySeed } from '../../src/city/defaultSeed';
import { generateCity } from '../../src/city/generateCity';

describe('generateCity', () => {
  it('generates the same city for the same seed', () => {
    const first = generateCity(defaultCitySeed);
    const second = generateCity(defaultCitySeed);

    expect(second).toEqual(first);
  });

  it('creates a river polycentric city with bridge-connected districts', () => {
    const city = generateCity(defaultCitySeed);

    expect(city.tiles.some((tile) => tile.terrain === 'water')).toBe(true);
    expect(city.districts.length).toBeGreaterThanOrEqual(4);
    expect(city.roadNodes.some((node) => node.kind === 'bridge')).toBe(true);
    expect(city.roadEdges.some((edge) => edge.orientation === 'bridge')).toBe(true);
    expect(city.parcels.length).toBeGreaterThan(20);
    expect(city.buildings.length).toBeGreaterThan(20);
  });
});
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
npm test -- tests/city/generateCity.test.ts
```

Expected: FAIL because generator files do not exist.

- [ ] **Step 3: Implement default seed**

Create `src/city/defaultSeed.ts`:

```ts
export type CitySeed = {
  readonly id: string;
  readonly seed: number;
  readonly width: number;
  readonly height: number;
  readonly generatorVersion: string;
};

export const defaultCitySeed: CitySeed = {
  id: 'abutown-river-polycentric',
  seed: 20260511,
  width: 72,
  height: 72,
  generatorVersion: 'river-polycentric-v1',
};
```

- [ ] **Step 4: Implement deterministic city generator**

Create `src/city/generateCity.ts`:

```ts
import type { CitySeed } from './defaultSeed';
import { makeId } from './ids';
import type { Building, City, District, Landmark, Parcel, RoadEdge, RoadNode, Tile } from './types';

const centers = [
  { name: 'Old Town', kind: 'old-town' as const, x: 26, y: 24, density: 0.95, centrality: 0.95 },
  { name: 'Market Bend', kind: 'market' as const, x: 38, y: 30, density: 0.85, centrality: 0.9 },
  { name: 'North Bank', kind: 'residential' as const, x: 22, y: 42, density: 0.62, centrality: 0.55 },
  { name: 'Civic Hill', kind: 'civic' as const, x: 48, y: 42, density: 0.7, centrality: 0.72 },
  { name: 'Mill Yard', kind: 'industrial' as const, x: 52, y: 24, density: 0.55, centrality: 0.48 },
];

export function generateCity(seed: CitySeed): City {
  const districts: District[] = centers.map((center, index) => ({
    id: makeId('district', [index]),
    name: center.name,
    kind: center.kind,
    center: { x: center.x, y: center.y },
    density: center.density,
    centrality: center.centrality,
  }));

  const tiles = generateTiles(seed, districts);
  const roadNodes = generateRoadNodes(districts);
  const roadEdges = generateRoadEdges(roadNodes);
  const parcels = generateParcels(districts, roadEdges);
  const buildings = generateBuildings(districts, parcels);
  const landmarks = generateLandmarks(districts);

  return {
    id: seed.id,
    seed: seed.seed,
    generatorVersion: seed.generatorVersion,
    width: seed.width,
    height: seed.height,
    tiles,
    roadNodes,
    roadEdges,
    blocks: districts.map((district, index) => ({
      id: makeId('block', [index]),
      boundaryRoadIds: roadEdges.slice(index, index + 2).map((edge) => edge.id),
      tileIds: tiles.filter((tile) => tile.districtId === district.id).slice(0, 18).map((tile) => tile.id),
      districtId: district.id,
    })),
    parcels,
    buildings,
    districts,
    landmarks,
  };
}

function generateTiles(seed: CitySeed, districts: readonly District[]): Tile[] {
  const tiles: Tile[] = [];

  for (let y = 0; y < seed.height; y += 1) {
    for (let x = 0; x < seed.width; x += 1) {
      const riverCenter = 34 + Math.round(Math.sin(y / 8) * 5);
      const riverDistance = Math.abs(x - riverCenter);
      const terrain = riverDistance <= 2 ? 'water' : riverDistance === 3 ? 'riverbank' : 'grass';
      const district = nearestDistrict({ x, y }, districts);

      tiles.push({
        id: makeId('tile', [x, y]),
        coord: { x, y },
        terrain,
        elevation: Math.max(0, Math.floor((x + y) / 28) - (terrain === 'water' ? 1 : 0)),
        districtId: terrain === 'water' ? undefined : district.id,
      });
    }
  }

  return tiles;
}

function generateRoadNodes(districts: readonly District[]): RoadNode[] {
  const centerNodes = districts.map((district) => ({
    id: makeId('roadNode', ['center', district.id]),
    coord: district.center,
    kind: 'center' as const,
  }));

  const bridgeNodes: RoadNode[] = [
    { id: makeId('roadNode', ['bridge', 0]), coord: { x: 34, y: 27 }, kind: 'bridge' },
    { id: makeId('roadNode', ['bridge', 1]), coord: { x: 33, y: 39 }, kind: 'bridge' },
  ];

  const exitNodes: RoadNode[] = [
    { id: makeId('roadNode', ['exit', 'west']), coord: { x: 6, y: 34 }, kind: 'exit' },
    { id: makeId('roadNode', ['exit', 'east']), coord: { x: 66, y: 34 }, kind: 'exit' },
  ];

  return [...centerNodes, ...bridgeNodes, ...exitNodes];
}

function generateRoadEdges(nodes: readonly RoadNode[]): RoadEdge[] {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const ids = [...byId.keys()];
  const pairs: readonly [string, string, RoadEdge['orientation']][] = [
    [ids[0], makeId('roadNode', ['bridge', 0]), 'northEast'],
    [makeId('roadNode', ['bridge', 0]), ids[1], 'bridge'],
    [ids[1], ids[4], 'northEast'],
    [ids[0], ids[2], 'northWest'],
    [ids[2], makeId('roadNode', ['bridge', 1]), 'northEast'],
    [makeId('roadNode', ['bridge', 1]), ids[3], 'bridge'],
    [ids[3], ids[4], 'eastWest'],
    [makeId('roadNode', ['exit', 'west']), ids[2], 'eastWest'],
    [ids[4], makeId('roadNode', ['exit', 'east']), 'eastWest'],
  ];

  return pairs.map(([from, to, orientation], index) => {
    const fromNode = byId.get(from);
    const toNode = byId.get(to);

    if (!fromNode || !toNode) {
      throw new Error(`Invalid road edge ${from} -> ${to}`);
    }

    return {
      id: makeId('roadEdge', [index]),
      from,
      to,
      points: [fromNode.coord, midpoint(fromNode.coord, toNode.coord), toNode.coord],
      direction: 'twoWay',
      modes: ['car', 'pedestrian', 'service'],
      orientation,
      cost: Math.round(distance(fromNode.coord, toNode.coord)),
    };
  });
}

function generateParcels(districts: readonly District[], roadEdges: readonly RoadEdge[]): Parcel[] {
  return districts.flatMap((district, districtIndex) => {
    return Array.from({ length: 9 }, (_, parcelIndex) => {
      const localX = parcelIndex % 3;
      const localY = Math.floor(parcelIndex / 3);
      const x = district.center.x + localX * 3 - 3;
      const y = district.center.y + localY * 3 - 3;

      return {
        id: makeId('parcel', [districtIndex, parcelIndex]),
        blockId: makeId('block', [districtIndex]),
        accessRoadId: roadEdges[(districtIndex + parcelIndex) % roadEdges.length].id,
        landUse: district.kind === 'parkland' ? 'park' : district.kind === 'industrial' ? 'industrial' : district.kind === 'civic' ? 'civic' : parcelIndex % 4 === 0 ? 'commercial' : 'residential',
        capacityHint: Math.max(1, Math.round(district.density * 12)),
        footprint: [
          { x, y },
          { x: x + 1, y },
          { x, y: y + 1 },
          { x: x + 1, y: y + 1 },
        ],
      };
    });
  });
}

function generateBuildings(districts: readonly District[], parcels: readonly Parcel[]): Building[] {
  return parcels.map((parcel, index) => {
    const district = districts.find((item) => item.id === parcel.blockId.replace('block', 'district')) ?? districts[index % districts.length];
    return {
      id: makeId('building', [index]),
      parcelId: parcel.id,
      districtId: district.id,
      assetKey: `building.${parcel.landUse}`,
      footprint: parcel.footprint,
      roleHints: [parcel.landUse],
      capacityHint: parcel.capacityHint,
    };
  });
}

function generateLandmarks(districts: readonly District[]): Landmark[] {
  return districts.map((district, index) => ({
    id: makeId('landmark', [index]),
    name: `${district.name} Landmark`,
    coord: { x: district.center.x + 1, y: district.center.y - 1 },
    districtId: district.id,
    assetKey: `landmark.${district.kind}`,
  }));
}

function nearestDistrict(coord: { readonly x: number; readonly y: number }, districts: readonly District[]): District {
  return districts.reduce((nearest, district) => {
    return distance(coord, district.center) < distance(coord, nearest.center) ? district : nearest;
  }, districts[0]);
}

function midpoint(a: { readonly x: number; readonly y: number }, b: { readonly x: number; readonly y: number }): { readonly x: number; readonly y: number } {
  return {
    x: Math.round((a.x + b.x) / 2),
    y: Math.round((a.y + b.y) / 2),
  };
}

function distance(a: { readonly x: number; readonly y: number }, b: { readonly x: number; readonly y: number }): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}
```

- [ ] **Step 5: Run tests and commit**

Run:

```bash
npm test -- tests/city/generateCity.test.ts
npm run build
```

Expected: PASS.

Commit:

```bash
git add src/city/defaultSeed.ts src/city/generateCity.ts tests/city/generateCity.test.ts
git commit -m "feat: generate deterministic river city model"
```

---

## Task 5: City Validation And Road Sprite Resolution

**Files:**
- Create: `src/city/validateCity.ts`
- Create: `src/render/roadSprites.ts`
- Create: `tests/city/validateCity.test.ts`
- Create: `tests/render/roadSprites.test.ts`

- [ ] **Step 1: Write validation tests**

Create `tests/city/validateCity.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { defaultCitySeed } from '../../src/city/defaultSeed';
import { generateCity } from '../../src/city/generateCity';
import { validateCity } from '../../src/city/validateCity';

describe('validateCity', () => {
  it('accepts the default city', () => {
    const result = validateCity(generateCity(defaultCitySeed));
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });
});
```

Create `tests/render/roadSprites.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { resolveRoadSpriteKey } from '../../src/render/roadSprites';
import type { RoadEdge } from '../../src/city/types';

const baseEdge: RoadEdge = {
  id: 'roadEdge:test',
  from: 'a',
  to: 'b',
  points: [{ x: 0, y: 0 }, { x: 1, y: 1 }],
  direction: 'twoWay',
  modes: ['car'],
  orientation: 'northEast',
  cost: 1,
};

describe('resolveRoadSpriteKey', () => {
  it('maps graph orientation to road sprite keys', () => {
    expect(resolveRoadSpriteKey({ ...baseEdge, orientation: 'northEast' })).toBe('road.northEast');
    expect(resolveRoadSpriteKey({ ...baseEdge, orientation: 'northWest' })).toBe('road.northWest');
    expect(resolveRoadSpriteKey({ ...baseEdge, orientation: 'eastWest' })).toBe('road.eastWest');
    expect(resolveRoadSpriteKey({ ...baseEdge, orientation: 'bridge' })).toBe('road.bridge');
  });
});
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
npm test -- tests/city/validateCity.test.ts tests/render/roadSprites.test.ts
```

Expected: FAIL because validation and sprite resolver files do not exist.

- [ ] **Step 3: Implement validation and sprite resolver**

Create `src/render/roadSprites.ts`:

```ts
import type { RoadEdge } from '../city/types';

export function resolveRoadSpriteKey(edge: RoadEdge): string {
  return `road.${edge.orientation}`;
}
```

Create `src/city/validateCity.ts`:

```ts
import type { City } from './types';
import { resolveRoadSpriteKey } from '../render/roadSprites';

export type ValidationResult = {
  readonly valid: boolean;
  readonly errors: readonly string[];
};

export function validateCity(city: City): ValidationResult {
  const errors: string[] = [];
  const nodeIds = new Set(city.roadNodes.map((node) => node.id));
  const roadIds = new Set(city.roadEdges.map((edge) => edge.id));
  const parcelIds = new Set(city.parcels.map((parcel) => parcel.id));
  const districtIds = new Set(city.districts.map((district) => district.id));

  for (const edge of city.roadEdges) {
    if (!nodeIds.has(edge.from)) errors.push(`Road edge ${edge.id} has missing from node ${edge.from}`);
    if (!nodeIds.has(edge.to)) errors.push(`Road edge ${edge.id} has missing to node ${edge.to}`);
    if (edge.points.length < 2) errors.push(`Road edge ${edge.id} has fewer than two points`);
    if (!resolveRoadSpriteKey(edge).startsWith('road.')) errors.push(`Road edge ${edge.id} has invalid sprite key`);
  }

  for (const parcel of city.parcels) {
    if (!roadIds.has(parcel.accessRoadId)) errors.push(`Parcel ${parcel.id} has missing access road ${parcel.accessRoadId}`);
    if (parcel.footprint.length === 0) errors.push(`Parcel ${parcel.id} has no footprint`);
  }

  for (const building of city.buildings) {
    if (!parcelIds.has(building.parcelId)) errors.push(`Building ${building.id} has missing parcel ${building.parcelId}`);
    if (!districtIds.has(building.districtId)) errors.push(`Building ${building.id} has missing district ${building.districtId}`);
  }

  return {
    valid: errors.length === 0,
    errors,
  };
}
```

- [ ] **Step 4: Run tests and commit**

Run:

```bash
npm test -- tests/city/validateCity.test.ts tests/render/roadSprites.test.ts
npm run build
```

Expected: PASS.

Commit:

```bash
git add src/city/validateCity.ts src/render/roadSprites.ts tests/city/validateCity.test.ts tests/render/roadSprites.test.ts
git commit -m "feat: validate city graph and road sprites"
```

---

## Task 6: OpenGFX2 Asset Import And Renderer

**Files:**
- Create: `public/opengfx2-classic/source/temperate_groundtiles_32bpp.png`
- Create: `public/opengfx2-classic/source/universal_watertiles_32bpp.png`
- Create: `public/opengfx2-classic/source/universal_rivertiles_32bpp.png`
- Create: `public/opengfx2-classic/source/temperate_park_32bpp.png`
- Create: `public/opengfx2-classic/source/road_town_overlayalpha.png`
- Create: `public/opengfx2-classic/source/road_overlayalpha.png`
- Create: `public/opengfx2-classic/source/general_bridgetiles_32bpp.png`
- Create: `public/opengfx2-classic/source/houses_shape.png`
- Create: `public/opengfx2-classic/source/shopsandoffices_shape.png`
- Create: `public/opengfx2-classic/source/churches_shape.png`
- Create: `public/opengfx2-classic/source/town_tree_32bpp.png`
- Create: `public/opengfx2-classic/licenses/LICENSE`
- Create: `public/opengfx2-classic/licenses/credits.md`
- Create: `src/assets/opengfx2-classic/README.md`
- Create: `src/assets/opengfx2-classic/atlas.json`
- Create: `src/render/assets.ts`
- Create: `src/render/CityRenderer.ts`
- Modify: `src/main.ts`

- [ ] **Step 1: Import selected OpenGFX2 Classic 64px sheets**

Run:

```bash
rm -rf /tmp/abutown-opengfx2
git clone --depth 1 --filter=blob:none --sparse https://github.com/OpenTTD/OpenGFX2.git /tmp/abutown-opengfx2
cd /tmp/abutown-opengfx2
git sparse-checkout set LICENSE credits.md graphics/terrain/64 graphics/infrastructure/64 graphics/towns/temperate/64 graphics/towns/streetfurniture/64
cd /Users/ramonfuglister/Desktop/Coding/abutown
mkdir -p public/opengfx2-classic/source public/opengfx2-classic/licenses
cp /tmp/abutown-opengfx2/LICENSE public/opengfx2-classic/licenses/LICENSE
cp /tmp/abutown-opengfx2/credits.md public/opengfx2-classic/licenses/credits.md
cp /tmp/abutown-opengfx2/graphics/terrain/64/temperate_groundtiles_32bpp.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/terrain/64/universal_watertiles_32bpp.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/terrain/64/universal_rivertiles_32bpp.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/terrain/64/temperate_park_32bpp.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/infrastructure/64/road_town_overlayalpha.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/infrastructure/64/road_overlayalpha.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/terrain/64/general_bridgetiles_32bpp.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/towns/temperate/64/houses_shape.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/towns/temperate/64/shopsandoffices_shape.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/towns/temperate/64/churches_shape.png public/opengfx2-classic/source/
cp /tmp/abutown-opengfx2/graphics/towns/streetfurniture/64/town_tree_32bpp.png public/opengfx2-classic/source/
```

Expected: selected real OpenGFX2 PNG sheets and license files exist under `public/opengfx2-classic/`.

- [ ] **Step 2: Add atlas manifest**

Create `src/assets/opengfx2-classic/README.md`:

```md
# OpenGFX2 Classic Assets

The MVP renderer uses selected OpenGFX2 Classic 64px source sheets with a small semantic atlas.

OpenGFX2 source: https://github.com/OpenTTD/OpenGFX2
License: GPL-2.0

The source sheets used by the app live in `public/opengfx2-classic/source/`. The copied license and credits live in `public/opengfx2-classic/licenses/`.

`atlas.json` maps game-facing keys such as `terrain.grass`, `road.northEast`, and `building.residential` to rectangular frames in those source sheets. When expanding asset coverage, add semantic keys here instead of hard-coding sprite-sheet coordinates in renderer code.
```

Create `src/assets/opengfx2-classic/atlas.json`:

```json
{
  "terrain.grass": { "source": "temperate_groundtiles_32bpp.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "terrain.water": { "source": "universal_watertiles_32bpp.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "terrain.riverbank": { "source": "universal_rivertiles_32bpp.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "terrain.park": { "source": "temperate_park_32bpp.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "terrain.plaza": { "source": "temperate_groundtiles_32bpp.png", "frame": { "x": 128, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.northEast": { "source": "road_overlayalpha.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.northWest": { "source": "road_overlayalpha.png", "frame": { "x": 64, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.eastWest": { "source": "road_town_overlayalpha.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.curve": { "source": "road_town_overlayalpha.png", "frame": { "x": 128, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.intersection": { "source": "road_town_overlayalpha.png", "frame": { "x": 256, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.bridge": { "source": "general_bridgetiles_32bpp.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "road.deadEnd": { "source": "road_town_overlayalpha.png", "frame": { "x": 320, "y": 0, "w": 64, "h": 42 }, "anchor": { "x": 0.5, "y": 0.38 } },
  "building.residential": { "source": "houses_shape.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "building.commercial": { "source": "shopsandoffices_shape.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "building.civic": { "source": "churches_shape.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "building.industrial": { "source": "shopsandoffices_shape.png", "frame": { "x": 64, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "building.park": { "source": "town_tree_32bpp.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "landmark.old-town": { "source": "churches_shape.png", "frame": { "x": 0, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "landmark.market": { "source": "shopsandoffices_shape.png", "frame": { "x": 64, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "landmark.residential": { "source": "houses_shape.png", "frame": { "x": 64, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "landmark.industrial": { "source": "shopsandoffices_shape.png", "frame": { "x": 128, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "landmark.civic": { "source": "churches_shape.png", "frame": { "x": 64, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } },
  "landmark.parkland": { "source": "town_tree_32bpp.png", "frame": { "x": 64, "y": 0, "w": 64, "h": 96 }, "anchor": { "x": 0.5, "y": 0.78 } }
}
```

- [ ] **Step 3: Implement asset loader**

Create `src/render/assets.ts`:

```ts
import { Assets, Rectangle, Texture } from 'pixi.js';
import atlas from '../assets/opengfx2-classic/atlas.json';

export type AssetFrame = {
  readonly source: string;
  readonly frame: {
    readonly x: number;
    readonly y: number;
    readonly w: number;
    readonly h: number;
  };
  readonly anchor: {
    readonly x: number;
    readonly y: number;
  };
};

export type AssetKey = keyof typeof atlas;

const frames: Record<string, AssetFrame> = atlas;
const basePath = '/opengfx2-classic/source/';
const sheetCache = new Map<string, Promise<Texture>>();

export function getAssetFrame(key: string): AssetFrame {
  return frames[key] ?? frames['terrain.grass'];
}

export async function loadTexture(key: string): Promise<Texture> {
  const entry = getAssetFrame(key);
  const sheet = await loadSheet(entry.source);

  return new Texture({
    source: sheet.source,
    frame: new Rectangle(entry.frame.x, entry.frame.y, entry.frame.w, entry.frame.h),
  });
}

function loadSheet(source: string): Promise<Texture> {
  const cached = sheetCache.get(source);
  if (cached) return cached;

  const promise = Assets.load<Texture>(`${basePath}${source}`);
  sheetCache.set(source, promise);
  return promise;
}
```

- [ ] **Step 4: Implement renderer**

Create `src/render/CityRenderer.ts`:

```ts
import { Application, Container, Graphics, Sprite } from 'pixi.js';
import type { City } from '../city/types';
import { gridToIso } from '../geo/isometric';
import { getAssetFrame, loadTexture } from './assets';
import { resolveRoadSpriteKey } from './roadSprites';

export class CityRenderer {
  readonly stage = new Container();
  private readonly terrainLayer = new Container();
  private readonly roadLayer = new Container();
  private readonly buildingLayer = new Container();

  constructor(private readonly app: Application) {
    this.stage.addChild(this.terrainLayer, this.roadLayer, this.buildingLayer);
    this.app.stage.addChild(this.stage);
  }

  async render(city: City): Promise<void> {
    this.clear();
    await this.renderTerrain(city);
    await this.renderRoads(city);
    await this.renderBuildings(city);
    this.stage.sortableChildren = true;
  }

  private clear(): void {
    this.terrainLayer.removeChildren();
    this.roadLayer.removeChildren();
    this.buildingLayer.removeChildren();
  }

  private async renderTerrain(city: City): Promise<void> {
    for (const tile of city.tiles) {
      const iso = gridToIso(tile.coord);
      const assetKey = `terrain.${tile.terrain}`;
      const texture = await loadTexture(assetKey);
      const frame = getAssetFrame(assetKey);
      const sprite = new Sprite(texture);
      sprite.anchor.set(frame.anchor.x, frame.anchor.y);
      sprite.x = iso.x;
      sprite.y = iso.y;
      sprite.alpha = tile.terrain === 'water' ? 0.82 : 1;
      this.terrainLayer.addChild(sprite);
    }
  }

  private async renderRoads(city: City): Promise<void> {
    for (const edge of city.roadEdges) {
      for (const point of edge.points) {
        const iso = gridToIso(point);
        const assetKey = resolveRoadSpriteKey(edge);
        const texture = await loadTexture(assetKey);
        const frame = getAssetFrame(assetKey);
        const sprite = new Sprite(texture);
        sprite.anchor.set(frame.anchor.x, frame.anchor.y);
        sprite.x = iso.x;
        sprite.y = iso.y - 2;
        this.roadLayer.addChild(sprite);
      }
    }
  }

  private async renderBuildings(city: City): Promise<void> {
    for (const building of city.buildings) {
      const coord = building.footprint[0];
      const iso = gridToIso(coord);
      const texture = await loadTexture(building.assetKey);
      const frame = getAssetFrame(building.assetKey);
      const sprite = new Sprite(texture);
      sprite.anchor.set(frame.anchor.x, frame.anchor.y);
      sprite.x = iso.x;
      sprite.y = iso.y - 14;
      sprite.zIndex = iso.y;
      this.buildingLayer.addChild(sprite);
    }
  }

  addBackgroundMarker(): void {
    const marker = new Graphics();
    marker.rect(-4, -4, 8, 8).fill(0xffffff);
    this.stage.addChild(marker);
  }
}
```

- [ ] **Step 5: Wire renderer into bootstrap**

Replace `src/main.ts` with:

```ts
import { Application } from 'pixi.js';
import './app/styles.css';
import { defaultCitySeed } from './city/defaultSeed';
import { generateCity } from './city/generateCity';
import { validateCity } from './city/validateCity';
import { CityRenderer } from './render/CityRenderer';

const appRoot = document.querySelector<HTMLDivElement>('#app');

if (!appRoot) {
  throw new Error('Missing #app root element');
}

const app = new Application();
await app.init({
  resizeTo: window,
  background: '#1b2a24',
  antialias: false,
});

appRoot.appendChild(app.canvas);
appRoot.dataset.ready = 'true';

const city = generateCity(defaultCitySeed);
const validation = validateCity(city);

if (!validation.valid) {
  throw new Error(`Invalid city:\n${validation.errors.join('\n')}`);
}

const renderer = new CityRenderer(app);
await renderer.render(city);

renderer.stage.x = window.innerWidth / 2;
renderer.stage.y = 60;
renderer.addBackgroundMarker();
```

- [ ] **Step 6: Run build and commit**

Run:

```bash
npm run build
```

Expected: PASS.

Commit:

```bash
git add public/opengfx2-classic src/assets/opengfx2-classic src/render/assets.ts src/render/CityRenderer.ts src/main.ts
git commit -m "feat: render generated city with asset manifest"
```

---

## Task 7: Camera And Hidden Debug Overlays

**Files:**
- Create: `src/render/CameraController.ts`
- Create: `src/render/debugOverlay.ts`
- Modify: `src/main.ts`

- [ ] **Step 1: Implement continuous pan and zoom controller**

Create `src/render/CameraController.ts`:

```ts
import type { Container } from 'pixi.js';
import { clamp } from '../geo/math';

export class CameraController {
  private dragging = false;
  private lastPointer = { x: 0, y: 0 };

  constructor(
    private readonly target: Container,
    private readonly view: HTMLCanvasElement,
  ) {}

  attach(): void {
    this.view.addEventListener('pointerdown', this.onPointerDown);
    this.view.addEventListener('pointermove', this.onPointerMove);
    window.addEventListener('pointerup', this.onPointerUp);
    this.view.addEventListener('wheel', this.onWheel, { passive: false });
  }

  destroy(): void {
    this.view.removeEventListener('pointerdown', this.onPointerDown);
    this.view.removeEventListener('pointermove', this.onPointerMove);
    window.removeEventListener('pointerup', this.onPointerUp);
    this.view.removeEventListener('wheel', this.onWheel);
  }

  private readonly onPointerDown = (event: PointerEvent): void => {
    this.dragging = true;
    this.lastPointer = { x: event.clientX, y: event.clientY };
    this.view.setPointerCapture(event.pointerId);
  };

  private readonly onPointerMove = (event: PointerEvent): void => {
    if (!this.dragging) return;

    const dx = event.clientX - this.lastPointer.x;
    const dy = event.clientY - this.lastPointer.y;
    this.target.x += dx;
    this.target.y += dy;
    this.lastPointer = { x: event.clientX, y: event.clientY };
  };

  private readonly onPointerUp = (): void => {
    this.dragging = false;
  };

  private readonly onWheel = (event: WheelEvent): void => {
    event.preventDefault();
    const previousScale = this.target.scale.x;
    const nextScale = clamp(previousScale * (1 - event.deltaY * 0.001), 0.45, 2.8);
    const rect = this.view.getBoundingClientRect();
    const pointerX = event.clientX - rect.left;
    const pointerY = event.clientY - rect.top;
    const worldX = (pointerX - this.target.x) / previousScale;
    const worldY = (pointerY - this.target.y) / previousScale;

    this.target.scale.set(nextScale);
    this.target.x = pointerX - worldX * nextScale;
    this.target.y = pointerY - worldY * nextScale;
  };
}
```

- [ ] **Step 2: Implement keyboard-toggle debug overlay**

Create `src/render/debugOverlay.ts`:

```ts
import { Container, Graphics, Text } from 'pixi.js';
import type { City } from '../city/types';
import { gridToIso } from '../geo/isometric';

export function createDebugOverlay(city: City): Container {
  const layer = new Container();
  layer.visible = false;

  for (const node of city.roadNodes) {
    const iso = gridToIso(node.coord);
    const dot = new Graphics();
    dot.circle(iso.x, iso.y, 3).fill(0xff3366);
    layer.addChild(dot);
  }

  const label = new Text({
    text: 'Debug: road nodes',
    style: { fill: 0xffffff, fontSize: 14 },
  });
  label.x = 16;
  label.y = 16;
  layer.addChild(label);

  return layer;
}

export function attachDebugToggle(layer: Container): void {
  window.addEventListener('keydown', (event) => {
    if (event.key.toLowerCase() === 'd') {
      layer.visible = !layer.visible;
    }
  });
}
```

- [ ] **Step 3: Wire camera and debug overlay**

Modify `src/main.ts`:

```ts
import { Application } from 'pixi.js';
import './app/styles.css';
import { defaultCitySeed } from './city/defaultSeed';
import { generateCity } from './city/generateCity';
import { validateCity } from './city/validateCity';
import { CameraController } from './render/CameraController';
import { CityRenderer } from './render/CityRenderer';
import { attachDebugToggle, createDebugOverlay } from './render/debugOverlay';

const appRoot = document.querySelector<HTMLDivElement>('#app');

if (!appRoot) {
  throw new Error('Missing #app root element');
}

const app = new Application();
await app.init({
  resizeTo: window,
  background: '#1b2a24',
  antialias: false,
});

appRoot.appendChild(app.canvas);
appRoot.dataset.ready = 'true';

const city = generateCity(defaultCitySeed);
const validation = validateCity(city);

if (!validation.valid) {
  throw new Error(`Invalid city:\n${validation.errors.join('\n')}`);
}

const renderer = new CityRenderer(app);
await renderer.render(city);

renderer.stage.x = window.innerWidth / 2;
renderer.stage.y = 60;
renderer.addBackgroundMarker();

const debugOverlay = createDebugOverlay(city);
renderer.stage.addChild(debugOverlay);
attachDebugToggle(debugOverlay);

const camera = new CameraController(renderer.stage, app.canvas);
camera.attach();
```

- [ ] **Step 4: Build and commit**

Run:

```bash
npm run build
```

Expected: PASS.

Commit:

```bash
git add src/render/CameraController.ts src/render/debugOverlay.ts src/main.ts
git commit -m "feat: add continuous camera and hidden debug overlay"
```

---

## Task 8: Browser Smoke Test

**Files:**
- Create: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Write smoke test**

Create `tests/e2e/render-smoke.spec.ts`:

```ts
import { expect, test } from '@playwright/test';

test('renders a non-empty city scene and supports camera input', async ({ page }) => {
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });

  await page.goto('/');
  await expect(page.locator('#app')).toHaveAttribute('data-ready', 'true');
  const canvas = page.locator('canvas');
  await expect(canvas).toBeVisible();

  const before = await canvas.evaluate((node) => {
    const rect = node.getBoundingClientRect();
    return { width: rect.width, height: rect.height };
  });

  expect(before.width).toBeGreaterThan(400);
  expect(before.height).toBeGreaterThan(300);

  await page.mouse.move(300, 250);
  await page.mouse.down();
  await page.mouse.move(380, 290);
  await page.mouse.up();
  await page.mouse.wheel(0, -250);

  await page.keyboard.press('d');
  await page.keyboard.press('d');

  expect(errors).toEqual([]);
});
```

- [ ] **Step 2: Run smoke test**

Run:

```bash
npx playwright install chromium
npm run test:e2e
```

Expected: PASS.

- [ ] **Step 3: Commit smoke test**

Run:

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test: add render smoke coverage"
```

---

## Task 9: Final Verification

**Files:**
- Modify: no files unless verification reveals a defect.

- [ ] **Step 1: Run full verification**

Run:

```bash
npm run build
npm test
npm run test:e2e
```

Expected: all commands PASS.

- [ ] **Step 2: Manual browser verification**

Run:

```bash
npm run dev
```

Open `http://127.0.0.1:5173`.

Verify:

- The first viewport shows only the city scene, no visible interface.
- Mouse/trackpad wheel zooms continuously.
- Dragging pans the scene.
- Pressing `d` toggles the debug overlay on and off.
- The scene is non-empty and visually coherent.

- [ ] **Step 3: Commit any verification fixes**

If no fixes are needed, do not create an empty commit.

If fixes are needed, run:

```bash
git add <changed-files>
git commit -m "fix: stabilize city demo verification"
```

---

## Self-Review Notes

Spec coverage:

- Client-only graphics demo: Tasks 1, 6, 7, 8.
- OpenGFX2 Classic 64px asset direction: Task 6.
- River polycentric city: Task 4.
- Simulation-ready model: Task 3.
- Correct road direction and sprites: Task 5.
- Continuous zoom and panning: Task 7.
- No visible UI by default: Tasks 1 and 7.
- Debug overlays hidden by default: Task 7.
- Tests for transforms, determinism, validation, road sprite mapping, and browser smoke: Tasks 2, 4, 5, 8.

Known implementation constraint:

- The first atlas uses selected OpenGFX2 Classic source sheets and semantic frame coordinates. A separate asset-expansion plan should broaden sprite coverage after the base renderer, camera, and city model are verified.
