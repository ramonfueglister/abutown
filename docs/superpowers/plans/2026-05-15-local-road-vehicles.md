# Local Road Vehicles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Make rendered road vehicles first-class local vehicle entities, clickable and inspectable, without treating cars as agents.

**Architecture:** Keep people and vehicles separate. `src/render/localRoadVehicles.ts` projects the existing runtime `Car` records into stable `localVehicles`; `src/render/roadVehicleInspector.ts` formats selected vehicle details; `src/main.ts` owns hit testing, selection, canvas feedback, and diagnostics. The old generic population vocabulary is cleaned so `vehicle` no longer appears as an agent kind.

**Tech Stack:** TypeScript, Vite canvas app, Vitest, Playwright smoke test, pak128 vehicle sprites.

---

## File Structure

- Create `src/render/localRoadVehicles.ts`: pure projection and hit testing for rendered road vehicles.
- Create `tests/render/localRoadVehicles.test.ts`: unit coverage for stable IDs, position projection, and hit radius behavior.
- Create `src/render/roadVehicleInspector.ts`: pure formatter for selected vehicle inspector rows.
- Create `tests/render/roadVehicleInspector.test.ts`: unit coverage for null and selected vehicle formatting.
- Modify `src/main.ts`: import vehicle projection/inspector, track selected vehicle state, draw selection feedback, update diagnostics.
- Modify `tests/e2e/render-smoke.spec.ts`: assert `localVehicles`, `mobility.vehicles`, and click selection.
- Modify `src/types.ts`, `src/agents/generateAgents.ts`, `tests/agents/generateAgents.test.ts`, and `tests/render/agentLod.test.ts`: remove the misleading "vehicle agent" vocabulary.

## Task 1: Local Road Vehicle Projection

**Files:**
- Create: `src/render/localRoadVehicles.ts`
- Test: `tests/render/localRoadVehicles.test.ts`

- [x] **Step 1: Write the failing projection tests**

Create `tests/render/localRoadVehicles.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import {
  buildLocalRoadVehicles,
  findNearestLocalRoadVehicle,
  type LocalRoadVehicleSource,
} from '../../src/render/localRoadVehicles';

const vehicles: LocalRoadVehicleSource[] = [
  {
    path: [{ x: 10, y: 20 }, { x: 14, y: 20 }, { x: 14, y: 24 }],
    offset: 0.5,
    speed: 1.4,
    sprite: { sheet: 'bus', role: 'vehicle.bus' },
  },
  {
    path: [{ x: 40, y: 8 }, { x: 40, y: 12 }],
    offset: 1.25,
    speed: 1.9,
    sprite: { sheet: 'truck', role: 'vehicle.truck' },
  },
];

describe('local road vehicles', () => {
  it('projects rendered cars into stable local vehicles', () => {
    expect(buildLocalRoadVehicles(vehicles)).toEqual([
      {
        id: 'vehicle:road:0',
        kind: 'road-vehicle',
        state: 'driving',
        coord: { x: 12, y: 20 },
        pathIndex: 0,
        nextCoord: { x: 14, y: 20 },
        speed: 1.4,
        spriteSheet: 'bus',
        role: 'vehicle.bus',
      },
      {
        id: 'vehicle:road:1',
        kind: 'road-vehicle',
        state: 'driving',
        coord: { x: 40, y: 11 },
        pathIndex: 1,
        nextCoord: { x: 40, y: 8 },
        speed: 1.9,
        spriteSheet: 'truck',
        role: 'vehicle.truck',
      },
    ]);
  });

  it('finds the nearest local road vehicle inside the click radius', () => {
    const localVehicles = buildLocalRoadVehicles(vehicles);
    const hit = findNearestLocalRoadVehicle(localVehicles, { x: 24, y: 40 }, (coord) => ({ x: coord.x * 2, y: coord.y * 2 }), 5);

    expect(hit?.id).toBe('vehicle:road:0');
  });

  it('returns null when no local road vehicle is close enough', () => {
    const localVehicles = buildLocalRoadVehicles(vehicles);
    const hit = findNearestLocalRoadVehicle(localVehicles, { x: 24, y: 40 }, (coord) => ({ x: coord.x * 2, y: coord.y * 2 }), 1);

    expect(hit).toBeNull();
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/localRoadVehicles.test.ts
```

Expected: FAIL because `src/render/localRoadVehicles.ts` does not exist.

- [x] **Step 3: Implement the projection module**

Create `src/render/localRoadVehicles.ts`:

```ts
import type { AssetRole } from '../assets/assetPack';
import type { VehicleSheetName } from './vehicleSprites';

export type LocalRoadVehicleCoord = {
  x: number;
  y: number;
};

export type LocalRoadVehicleSource = {
  path: LocalRoadVehicleCoord[];
  offset: number;
  speed: number;
  sprite: {
    sheet: VehicleSheetName;
    role: Extract<AssetRole, 'vehicle.bus' | 'vehicle.truck'>;
  };
};

export type LocalRoadVehicle = {
  id: string;
  kind: 'road-vehicle';
  state: 'driving';
  coord: LocalRoadVehicleCoord;
  pathIndex: number;
  nextCoord: LocalRoadVehicleCoord;
  speed: number;
  spriteSheet: VehicleSheetName;
  role: Extract<AssetRole, 'vehicle.bus' | 'vehicle.truck'>;
};

export function localRoadVehicleId(index: number): string {
  return `vehicle:road:${index}`;
}

export function buildLocalRoadVehicles(vehicles: readonly LocalRoadVehicleSource[]): LocalRoadVehicle[] {
  return vehicles
    .filter((vehicle) => vehicle.path.length > 0)
    .map((vehicle, index) => {
      const pathIndex = normalizedPathIndex(vehicle);
      const nextCoord = vehicle.path[(pathIndex + 1) % vehicle.path.length];
      return {
        id: localRoadVehicleId(index),
        kind: 'road-vehicle',
        state: 'driving',
        coord: vehiclePosition(vehicle, pathIndex),
        pathIndex,
        nextCoord,
        speed: vehicle.speed,
        spriteSheet: vehicle.sprite.sheet,
        role: vehicle.sprite.role,
      };
    });
}

export function findNearestLocalRoadVehicle(
  vehicles: readonly LocalRoadVehicle[],
  point: LocalRoadVehicleCoord,
  project: (coord: LocalRoadVehicleCoord) => LocalRoadVehicleCoord,
  radius: number,
): LocalRoadVehicle | null {
  let nearest: { vehicle: LocalRoadVehicle; distance: number } | null = null;
  for (const vehicle of vehicles) {
    const projected = project(vehicle.coord);
    const distance = Math.hypot(projected.x - point.x, projected.y - point.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { vehicle, distance };
  }
  return nearest?.vehicle ?? null;
}

function normalizedPathIndex(vehicle: LocalRoadVehicleSource): number {
  const base = Math.floor(vehicle.offset);
  return ((base % vehicle.path.length) + vehicle.path.length) % vehicle.path.length;
}

function vehiclePosition(vehicle: LocalRoadVehicleSource, pathIndex: number): LocalRoadVehicleCoord {
  const next = (pathIndex + 1) % vehicle.path.length;
  const t = vehicle.offset - Math.floor(vehicle.offset);
  return {
    x: lerp(vehicle.path[pathIndex].x, vehicle.path[next].x, t),
    y: lerp(vehicle.path[pathIndex].y, vehicle.path[next].y, t),
  };
}

function lerp(start: number, end: number, t: number): number {
  return start + (end - start) * t;
}
```

- [x] **Step 4: Run test to verify it passes**

Run:

```bash
npm test -- tests/render/localRoadVehicles.test.ts
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/render/localRoadVehicles.ts tests/render/localRoadVehicles.test.ts
git commit -m "feat: project cars as local road vehicles"
```

## Task 2: Road Vehicle Inspector

**Files:**
- Create: `src/render/roadVehicleInspector.ts`
- Test: `tests/render/roadVehicleInspector.test.ts`

- [x] **Step 1: Write the failing inspector tests**

Create `tests/render/roadVehicleInspector.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { buildRoadVehicleInspector } from '../../src/render/roadVehicleInspector';
import type { LocalRoadVehicle } from '../../src/render/localRoadVehicles';

const vehicle: LocalRoadVehicle = {
  id: 'vehicle:road:12',
  kind: 'road-vehicle',
  state: 'driving',
  coord: { x: 42.25, y: 18.75 },
  pathIndex: 7,
  nextCoord: { x: 43, y: 19 },
  speed: 1.734,
  spriteSheet: 'truck',
  role: 'vehicle.truck',
};

describe('road vehicle inspector', () => {
  it('returns null when no road vehicle is selected', () => {
    expect(buildRoadVehicleInspector(null)).toBeNull();
  });

  it('formats compact rows for the selected road vehicle', () => {
    expect(buildRoadVehicleInspector(vehicle)).toEqual({
      title: 'vehicle:road:12',
      rows: [
        { label: 'State', value: 'driving' },
        { label: 'Tile', value: '42.3, 18.8' },
        { label: 'Next', value: '43.0, 19.0' },
        { label: 'Speed', value: '1.73' },
        { label: 'Sprite', value: 'truck' },
      ],
    });
  });
});
```

- [x] **Step 2: Run test to verify it fails**

Run:

```bash
npm test -- tests/render/roadVehicleInspector.test.ts
```

Expected: FAIL because `src/render/roadVehicleInspector.ts` does not exist.

- [x] **Step 3: Implement the inspector module**

Create `src/render/roadVehicleInspector.ts`:

```ts
import type { LocalRoadVehicle } from './localRoadVehicles';

export type RoadVehicleInspectorRow = {
  label: string;
  value: string;
};

export type RoadVehicleInspector = {
  title: string;
  rows: RoadVehicleInspectorRow[];
};

export function buildRoadVehicleInspector(vehicle: LocalRoadVehicle | null): RoadVehicleInspector | null {
  if (!vehicle) return null;
  return {
    title: vehicle.id,
    rows: [
      { label: 'State', value: vehicle.state },
      { label: 'Tile', value: formatCoord(vehicle.coord) },
      { label: 'Next', value: formatCoord(vehicle.nextCoord) },
      { label: 'Speed', value: vehicle.speed.toFixed(2) },
      { label: 'Sprite', value: vehicle.spriteSheet },
    ],
  };
}

function formatCoord(coord: { x: number; y: number }): string {
  return `${coord.x.toFixed(1)}, ${coord.y.toFixed(1)}`;
}
```

- [x] **Step 4: Run test to verify it passes**

Run:

```bash
npm test -- tests/render/roadVehicleInspector.test.ts
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add src/render/roadVehicleInspector.ts tests/render/roadVehicleInspector.test.ts
git commit -m "feat: add road vehicle inspector model"
```

## Task 3: Main Runtime Vehicle Selection

**Files:**
- Modify: `src/main.ts`
- Test: `tests/e2e/render-smoke.spec.ts`

- [x] **Step 1: Write the failing E2E expectations**

In `tests/e2e/render-smoke.spec.ts`, after the existing `localAgents` assertions, add:

```ts
expect(state.city.mobility).toEqual(expect.objectContaining({
  status: 'local-mobility',
  agents: state.city.pedestrians,
  vehicles: state.city.cars,
  stops: 0,
}));
expect(state.city.localVehicles.count).toBe(state.city.cars);
expect(state.city.localVehicles.selectedId).toBeNull();
expect(state.city.localVehicles.vehicles.length).toBe(state.city.cars);
expect(state.city.localVehicles.vehicles[0]).toEqual(expect.objectContaining({
  id: 'vehicle:road:0',
  kind: 'road-vehicle',
  state: 'driving',
  coord: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
  screen: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
}));
```

After the existing pedestrian click assertion, add a second click for a visible vehicle:

```ts
const clickableVehicle = selectedState.city.localVehicles.vehicles.find(
  (vehicle: { screen: { x: number; y: number } }) =>
    vehicle.screen.x > 16 &&
    vehicle.screen.x < 393 &&
    vehicle.screen.y > 16 &&
    vehicle.screen.y < 503,
);
expect(clickableVehicle).toBeTruthy();
await page.mouse.click(clickableVehicle.screen.x, clickableVehicle.screen.y);
const vehicleSelectedState = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
expect(vehicleSelectedState.city.localVehicles.selectedId).toBe(clickableVehicle.id);
expect(vehicleSelectedState.city.localAgents.selectedId).toBeNull();
expect(vehicleSelectedState.city.vehicleInspector).toEqual(expect.objectContaining({
  title: clickableVehicle.id,
  rows: expect.arrayContaining([
    expect.objectContaining({ label: 'State', value: 'driving' }),
    expect.objectContaining({ label: 'Tile', value: expect.any(String) }),
    expect.objectContaining({ label: 'Speed', value: expect.any(String) }),
  ]),
}));
```

- [x] **Step 2: Run E2E to verify it fails**

Run:

```bash
npm run build
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: FAIL because `state.city.localVehicles` and `state.city.vehicleInspector` are undefined.

- [x] **Step 3: Wire local vehicles into `src/main.ts`**

Apply these changes:

```ts
import {
  buildLocalRoadVehicles,
  findNearestLocalRoadVehicle,
  localRoadVehicleId,
  type LocalRoadVehicle,
} from './render/localRoadVehicles';
import { buildRoadVehicleInspector, type RoadVehicleInspector } from './render/roadVehicleInspector';
```

Change the `CarDrawable` type:

```ts
type CarDrawable = { type: 'car'; coord: Coord; car: Car; vehicleId: string };
```

Add state next to `selectedAgentId`:

```ts
let selectedAgentId: string | null = null;
let selectedVehicleId: string | null = null;
```

Update `drawScene()` car drawables:

```ts
const carDrawables = cars
  .map((car, index) => ({ type: 'car' as const, coord: carPosition(car), car, vehicleId: localRoadVehicleId(index) }))
  .filter((item) => isCoordVisible(item.coord, visibleGrid))
  .sort(compareDrawables);
```

Change car rendering:

```ts
if (item.type === 'car') drawCar(item.car, item.vehicleId === selectedVehicleId);
```

Change `drawCar` signature and add the selection ring before drawing the sprite:

```ts
function drawCar(car: Car, selected: boolean): void {
  const image = images.get(car.sprite.path);
  if (!image) return;
  const base = Math.floor(car.offset);
  const current = car.path[base];
  const next = car.path[(base + 1) % car.path.length];
  const pos = carPosition(car);
  const point = iso(pos);
  const currentPoint = iso(current);
  const nextPoint = iso(next);
  const lane = screenRightLaneOffset(currentPoint, nextPoint, 5.5);
  const frame = vehicleFrameForGridDelta({ x: next.x - current.x, y: next.y - current.y });
  const rect = vehicleFrameRect(car.sprite, frame);
  if (rect.x >= image.width || rect.y >= image.height) return;
  const sourceWidth = Math.min(rect.width, image.width - rect.x);
  const sourceHeight = Math.min(rect.height, image.height - rect.y);
  const scale = car.sprite.scale;
  const width = sourceWidth * scale;
  const height = sourceHeight * scale;
  ctx.save();
  ctx.translate(point.x + lane.x, point.y + lane.y + 7);
  if (selected) {
    ctx.globalAlpha = 0.94;
    ctx.strokeStyle = '#75d7ff';
    ctx.lineWidth = 2 / Math.max(0.75, camera.scale);
    ctx.beginPath();
    ctx.ellipse(0, -Math.max(4, height * 0.32), Math.max(9, width * 0.52), Math.max(7, height * 0.28), 0, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.drawImage(image, rect.x, rect.y, sourceWidth, sourceHeight, -width / 2, -height, width, height);
  ctx.restore();
}
```

Add helpers next to `localPedestrianAgents()`:

```ts
function localRoadVehicles(): LocalRoadVehicle[] {
  return buildLocalRoadVehicles(cars);
}

function selectedRoadVehicle(): LocalRoadVehicle | null {
  if (!selectedVehicleId) return null;
  return localRoadVehicles().find((vehicle) => vehicle.id === selectedVehicleId) ?? null;
}
```

Update click selection so vehicles and pedestrians are mutually exclusive:

```ts
function selectPedestrianAgentAtScreenPoint(point: Coord): void {
  const worldPoint = screenToWorld(point);
  const vehicleHit = findNearestLocalRoadVehicle(localRoadVehicles(), worldPoint, iso, Math.max(10, 24 / camera.scale));
  if (vehicleHit) {
    selectedVehicleId = vehicleHit.id;
    selectedAgentId = null;
    return;
  }
  const agentHit = findNearestPedestrianAgent(localPedestrianAgents(), worldPoint, iso, Math.max(8, 20 / camera.scale));
  selectedAgentId = agentHit?.id ?? null;
  selectedVehicleId = null;
}
```

Rename the function to `selectMobilityEntityAtScreenPoint` in a follow-up refactor inside the same task:

```ts
function selectMobilityEntityAtScreenPoint(point: Coord): void {
  const worldPoint = screenToWorld(point);
  const vehicleHit = findNearestLocalRoadVehicle(localRoadVehicles(), worldPoint, iso, Math.max(10, 24 / camera.scale));
  if (vehicleHit) {
    selectedVehicleId = vehicleHit.id;
    selectedAgentId = null;
    return;
  }
  const agentHit = findNearestPedestrianAgent(localPedestrianAgents(), worldPoint, iso, Math.max(8, 20 / camera.scale));
  selectedAgentId = agentHit?.id ?? null;
  selectedVehicleId = null;
}
```

Update the pointer handler call:

```ts
if (clickDistance < 4) selectMobilityEntityAtScreenPoint({ x: event.clientX, y: event.clientY });
```

Draw the vehicle inspector below the pedestrian inspector:

```ts
drawAgentInspectorPanel(buildPedestrianAgentInspector(selectedPedestrianAgent()));
drawRoadVehicleInspectorPanel(buildRoadVehicleInspector(selectedRoadVehicle()));
```

Implement `drawRoadVehicleInspectorPanel` by reusing the existing panel geometry with `x = 12`, `y = 128`, `width = 232`, border `#75d7ff`, title color `#75d7ff`, and the same row layout as `drawAgentInspectorPanel`.

Serialize vehicles for diagnostics:

```ts
function serializeLocalVehicle(vehicle: LocalRoadVehicle): LocalRoadVehicle & { screen: Coord } {
  const projected = iso(vehicle.coord);
  return {
    ...vehicle,
    screen: {
      x: camera.x + projected.x * camera.scale,
      y: camera.y + projected.y * camera.scale,
    },
  };
}
```

Update `render_game_to_text()`:

```ts
const localVehicles = localRoadVehicles();
const serializedVehicles = localVehicles.map(serializeLocalVehicle);
const selectedVehicle = localVehicles.find((vehicle) => vehicle.id === selectedVehicleId) ?? null;
const selectedSerializedVehicle = selectedVehicle ? serializeLocalVehicle(selectedVehicle) : null;
```

Update the JSON payload:

```ts
mobility: {
  status: 'local-mobility',
  agents: agents.length,
  vehicles: localVehicles.length,
  stops: 0,
},
localVehicles: {
  count: localVehicles.length,
  selectedId: selectedVehicleId,
  selected: selectedSerializedVehicle,
  vehicles: serializedVehicles,
},
vehicleInspector: buildRoadVehicleInspector(selectedVehicle),
```

- [x] **Step 4: Run verification for main wiring**

Run:

```bash
npm test -- tests/render/localRoadVehicles.test.ts tests/render/roadVehicleInspector.test.ts
npm run build
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: all commands pass.

- [x] **Step 5: Commit**

```bash
git add src/main.ts src/render/localRoadVehicles.ts src/render/roadVehicleInspector.ts tests/render/localRoadVehicles.test.ts tests/render/roadVehicleInspector.test.ts tests/e2e/render-smoke.spec.ts
git commit -m "feat: make road vehicles selectable"
```

## Task 4: Remove Vehicle-As-Agent Vocabulary

**Files:**
- Modify: `src/types.ts`
- Modify: `src/agents/generateAgents.ts`
- Modify: `tests/agents/generateAgents.test.ts`
- Modify: `tests/render/agentLod.test.ts`

- [x] **Step 1: Write failing vocabulary expectations**

In `tests/agents/generateAgents.test.ts`, rename the describe block and references so it describes a generated population, not agents:

```ts
import { describe, expect, it } from 'vitest';
import { generatePopulation } from '../../src/agents/generateAgents';
import { generateCity } from '../../src/city/generateCity';

describe('generatePopulation', () => {
  it('creates a deterministic 10,000 entity population without storing entities on the city', () => {
    const city = generateCity();
    const first = generatePopulation(city, { count: 10_000, seed: 9231 });
    const second = generatePopulation(city, { count: 10_000, seed: 9231 });

    expect(first.stats.totalEntities).toBe(10_000);
    expect(first.stats.people + first.stats.vehicles).toBe(10_000);
    expect(first.entities.slice(0, 12)).toEqual(second.entities.slice(0, 12));
    expect('agents' in city).toBe(false);
  });

  it('buckets population entities by road segment', () => {
    const city = generateCity();
    const population = generatePopulation(city, { count: 800, seed: 7 });

    expect(population.segmentBuckets.size).toBeGreaterThan(1);
    expect([...population.segmentBuckets.values()].flat()).toHaveLength(800);
  });
});
```

Update imports in `tests/render/agentLod.test.ts` from `generateAgents` to `generatePopulation`, and update property names from `agents` to `entities` and `totalAgents` to `totalEntities`.

- [x] **Step 2: Run tests to verify they fail**

Run:

```bash
npm test -- tests/agents/generateAgents.test.ts tests/render/agentLod.test.ts
```

Expected: FAIL because `generatePopulation`, `entities`, and `totalEntities` are not implemented.

- [x] **Step 3: Rename the generated population types**

In `src/types.ts`, replace the old agent population types with:

```ts
export type PopulationEntityKind = 'person' | 'vehicle';
export type PopulationEntityRole = 'resident' | 'worker' | 'visitor' | 'service';

export type PopulationEntity = {
  id: string;
  kind: PopulationEntityKind;
  role: PopulationEntityRole;
  roadEdgeId: string;
  progress: number;
  laneOffset: number;
  speedTilesPerSecond: number;
  colorIndex: number;
};

export type GeneratedPopulation = {
  entities: PopulationEntity[];
  segmentBuckets: Map<string, PopulationEntity[]>;
  stats: {
    totalEntities: number;
    people: number;
    vehicles: number;
  };
};
```

In `src/agents/generateAgents.ts`, keep the file path for now to avoid a broad file move, but rename exports and internals:

```ts
import type { City, GeneratedPopulation, PopulationEntity, PopulationEntityKind, PopulationEntityRole, RoadEdge } from '../types';

type GeneratePopulationOptions = {
  count: number;
  seed: number;
};

export function generatePopulation(city: City, options: GeneratePopulationOptions): GeneratedPopulation {
  if (options.count === 0) return emptyPopulation();

  const random = seededRandom(options.seed);
  const eligibleRoads = city.roadEdges.filter((edge) => edge.points.length > 1 && edge.modes.includes('pedestrian'));
  if (eligibleRoads.length === 0) {
    throw new Error('Cannot generate population without eligible pedestrian road edges');
  }

  const weightedRoads = weightRoads(eligibleRoads);
  const entities: PopulationEntity[] = [];
  for (let index = 0; index < options.count; index += 1) {
    const roadEdge = pickWeightedRoad(weightedRoads, random());
    const kind = chooseKind(roadEdge, random());
    const role = chooseRole(kind, random());
    entities.push({
      id: `population:${options.seed}:${index}`,
      kind,
      role,
      roadEdgeId: roadEdge.id,
      progress: random(),
      laneOffset: Number(((random() - 0.5) * (kind === 'vehicle' ? 0.42 : 0.26)).toFixed(3)),
      speedTilesPerSecond: speedFor(kind, role, random()),
      colorIndex: Math.floor(random() * 8),
    });
  }

  const segmentBuckets = new Map<string, PopulationEntity[]>();
  for (const entity of entities) {
    const bucket = segmentBuckets.get(entity.roadEdgeId) ?? [];
    bucket.push(entity);
    segmentBuckets.set(entity.roadEdgeId, bucket);
  }

  return {
    entities,
    segmentBuckets,
    stats: {
      totalEntities: entities.length,
      people: entities.filter((entity) => entity.kind === 'person').length,
      vehicles: entities.filter((entity) => entity.kind === 'vehicle').length,
    },
  };
}

function emptyPopulation(): GeneratedPopulation {
  return {
    entities: [],
    segmentBuckets: new Map(),
    stats: { totalEntities: 0, people: 0, vehicles: 0 },
  };
}

function chooseKind(roadEdge: RoadEdge, randomValue: number): PopulationEntityKind {
  return roadEdge.modes.includes('car') && randomValue < 0.18 ? 'vehicle' : 'person';
}

function chooseRole(kind: PopulationEntityKind, randomValue: number): PopulationEntityRole {
  if (kind === 'vehicle') return randomValue < 0.72 ? 'worker' : 'service';
  if (randomValue < 0.52) return 'resident';
  return randomValue < 0.82 ? 'worker' : 'visitor';
}

function speedFor(kind: PopulationEntityKind, role: PopulationEntityRole, randomValue: number): number {
  return Number((kind === 'vehicle' ? 2.4 + randomValue * (role === 'service' ? 1.8 : 1.2) : 0.55 + randomValue * 0.65).toFixed(3));
}
```

Keep the existing `seededRandom`, `weightRoads`, and `pickWeightedRoad` helpers unchanged.

- [x] **Step 4: Verify no vehicle-as-agent vocabulary remains in active source**

Run:

```bash
rg -n "AgentKind|AgentPopulation|kind: 'vehicle'|kind === 'vehicle' \\? 'vehicle' : 'pedestrian'|vehicles: agents" src tests
npm test -- tests/agents/generateAgents.test.ts tests/render/agentLod.test.ts
```

Expected: the `rg` command has no hits for old type names or vehicle-as-agent stats; both tests pass.

- [x] **Step 5: Commit**

```bash
git add src/types.ts src/agents/generateAgents.ts tests/agents/generateAgents.test.ts tests/render/agentLod.test.ts
git commit -m "refactor: separate generated vehicles from agents"
```

## Task 5: Final Verification

**Files:**
- No planned source changes unless verification exposes a defect.

- [x] **Step 1: Run full frontend unit suite**

Run:

```bash
npm test
```

Expected: all test files pass.

- [x] **Step 2: Run production build**

Run:

```bash
npm run build
```

Expected: TypeScript and Vite build pass.

- [x] **Step 3: Run browser smoke test**

Run:

```bash
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: Chromium smoke test passes. The city reports `mobility.status === "local-mobility"`, `localAgents.count === pedestrians`, and `localVehicles.count === cars`.

- [x] **Step 4: Verify live local server**

Load `http://127.0.0.1:5175/`. Click a visible road vehicle. Confirm a blue vehicle inspector appears and `window.render_game_to_text()` exposes `city.localVehicles.selectedId`.

- [x] **Step 5: Commit only if verification required fixes**

If Task 5 required source changes, commit them:

```bash
git add <changed-files>
git commit -m "fix: complete local road vehicle verification"
```

If Task 5 required no source changes, do not create an empty commit.

## Self-Review

- Spec coverage: The plan covers local vehicle projection, vehicle selection, inspector display, diagnostics, and removal of vehicle-as-agent vocabulary.
- Placeholder scan: No reserved placeholder markers or open-ended implementation steps remain.
- Type consistency: `LocalRoadVehicle`, `buildLocalRoadVehicles`, `buildRoadVehicleInspector`, `localVehicles`, and `GeneratedPopulation` names are used consistently across tasks.
