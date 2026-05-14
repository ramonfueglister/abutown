# Traffic Rules Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the first invisible reservation-based traffic-rule slice so cars yield at intersections without visible UI, while keeping the model ready for Rust ECS authority and Traffic LOD.

**Architecture:** Add pure `src/traffic/*` modules for serializable traffic state, intersection extraction, vehicle requests, and a local deterministic reservation authority. `src/main.ts` integrates the local authority only as a temporary reference model; rendering remains pose/sprite-only and never owns traffic priority. The data model uses stable IDs, ticks, versions, compact reservation tables, and diagnostics so it can be replaced by Rust snapshots/deltas later.

**Tech Stack:** TypeScript, Vite, Vitest, Playwright/browser smoke tests, existing Canvas/OpenGFX vehicle renderer.

---

## File Structure

- Create `src/traffic/trafficTypes.ts`
  Owns serializable traffic IDs, reservations, decisions, snapshots, requests, diagnostics, and direction constants. No browser or canvas dependency.
- Create `src/traffic/intersections.ts`
  Converts road graph tiles into deterministic `TrafficIntersection` metadata. No vehicle or renderer dependency.
- Create `src/traffic/vehicleTrafficRequests.ts`
  Converts current vehicle route offsets into intersection reservation requests. Keeps route math out of `main.ts`.
- Create `src/traffic/localTrafficAuthority.ts`
  Pure reservation engine. Takes previous snapshot + current requests + tick, returns next snapshot, per-vehicle decisions, diagnostics.
- Create tests:
  - `tests/traffic/intersections.test.ts`
  - `tests/traffic/vehicleTrafficRequests.test.ts`
  - `tests/traffic/localTrafficAuthority.test.ts`
- Modify `src/main.ts`
  Adds vehicle IDs, builds traffic intersections once, advances a deterministic traffic tick, applies traffic decisions before car offset advances, and exposes diagnostics in `render_game_to_text`.
- Modify `tests/e2e/render-smoke.spec.ts`
  Asserts traffic diagnostics exist and no visible UI is required.

## Task 1: Traffic Types

**Files:**
- Create: `src/traffic/trafficTypes.ts`
- Test indirectly in later tasks.

- [ ] **Step 1: Create the shared traffic types**

Create `src/traffic/trafficTypes.ts`:

```ts
export type TrafficCoord = { x: number; y: number };

export type VehicleId = `vehicle:${number}`;
export type IntersectionId = `intersection:${number}:${number}`;
export type ReservationId = `${IntersectionId}:${VehicleId}:${number}`;

export type TrafficDirection = 'north' | 'east' | 'south' | 'west';
export type TrafficDecisionKind = 'go' | 'yield' | 'stop' | 'blocked';

export type TrafficIntersection = {
  intersectionId: IntersectionId;
  coord: TrafficCoord;
  connectedDirections: readonly TrafficDirection[];
};

export type TrafficReservation = {
  reservationId: ReservationId;
  intersectionId: IntersectionId;
  vehicleId: VehicleId;
  enterTick: number;
  exitTick: number;
  approachEdge: TrafficDirection;
  exitEdge: TrafficDirection;
  conflictMask: number;
  priority: number;
};

export type TrafficVehicleRequest = {
  vehicleId: VehicleId;
  intersectionId: IntersectionId;
  distanceToIntersection: number;
  stopOffset: number;
  currentOffset: number;
  enterTick: number;
  exitTick: number;
  approachEdge: TrafficDirection;
  exitEdge: TrafficDirection;
  conflictMask: number;
  priority: number;
};

export type TrafficDecision = {
  vehicleId: VehicleId;
  kind: TrafficDecisionKind;
  speedFactor: number;
  maxAdvance?: number;
  intersectionId?: IntersectionId;
  reservationId?: ReservationId;
};

export type TrafficRuleDiagnostics = {
  reservedIntersections: number;
  yieldingVehicles: number;
  stoppedForTrafficRules: number;
  blockedVehicles: number;
  intersectionConflictsPrevented: number;
  expiredReservations: number;
  trafficRuleDecisionCount: number;
  unclassifiedTrafficRequests: number;
};

export type TrafficRuleSnapshot = {
  tick: number;
  version: number;
  reservations: readonly TrafficReservation[];
  diagnostics: TrafficRuleDiagnostics;
};

export const EMPTY_TRAFFIC_DIAGNOSTICS: TrafficRuleDiagnostics = {
  reservedIntersections: 0,
  yieldingVehicles: 0,
  stoppedForTrafficRules: 0,
  blockedVehicles: 0,
  intersectionConflictsPrevented: 0,
  expiredReservations: 0,
  trafficRuleDecisionCount: 0,
  unclassifiedTrafficRequests: 0,
};

export function createInitialTrafficRuleSnapshot(): TrafficRuleSnapshot {
  return {
    tick: 0,
    version: 0,
    reservations: [],
    diagnostics: { ...EMPTY_TRAFFIC_DIAGNOSTICS },
  };
}

export function trafficIntersectionId(coord: TrafficCoord): IntersectionId {
  return `intersection:${Math.round(coord.x)}:${Math.round(coord.y)}`;
}

export function trafficReservationId(
  intersectionId: IntersectionId,
  vehicleId: VehicleId,
  enterTick: number,
): ReservationId {
  return `${intersectionId}:${vehicleId}:${enterTick}`;
}

export function trafficKey(coord: TrafficCoord): string {
  return `${Math.round(coord.x)}:${Math.round(coord.y)}`;
}
```

- [ ] **Step 2: Run the TypeScript build check**

Run:

```bash
npm run build
```

Expected: PASS. The new type-only module compiles without changing runtime behavior.

- [ ] **Step 3: Commit**

```bash
git add src/traffic/trafficTypes.ts
git commit -m "feat: define traffic rule state types"
```

## Task 2: Intersection Extraction

**Files:**
- Create: `tests/traffic/intersections.test.ts`
- Create: `src/traffic/intersections.ts`

- [ ] **Step 1: Write the failing intersection tests**

Create `tests/traffic/intersections.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import {
  ROAD_EAST,
  ROAD_NORTH,
  ROAD_SOUTH,
  ROAD_WEST,
  buildTrafficIntersections,
  directionForRoadStep,
} from '../../src/traffic/intersections';

describe('traffic intersections', () => {
  it('creates deterministic intersection ids for road nodes with degree three or higher', () => {
    const intersections = buildTrafficIntersections([
      { coord: { x: 4, y: 5 }, mask: ROAD_NORTH | ROAD_EAST | ROAD_SOUTH },
      { coord: { x: 1, y: 2 }, mask: ROAD_EAST | ROAD_WEST },
      { coord: { x: 8, y: 9 }, mask: ROAD_NORTH | ROAD_EAST | ROAD_SOUTH | ROAD_WEST },
    ]);

    expect(intersections).toEqual([
      {
        intersectionId: 'intersection:4:5',
        coord: { x: 4, y: 5 },
        connectedDirections: ['north', 'east', 'south'],
      },
      {
        intersectionId: 'intersection:8:9',
        coord: { x: 8, y: 9 },
        connectedDirections: ['north', 'east', 'south', 'west'],
      },
    ]);
  });

  it('classifies route steps into approach directions', () => {
    expect(directionForRoadStep({ x: 2, y: 1 }, { x: 2, y: 2 })).toBe('north');
    expect(directionForRoadStep({ x: 3, y: 2 }, { x: 2, y: 2 })).toBe('east');
    expect(directionForRoadStep({ x: 2, y: 3 }, { x: 2, y: 2 })).toBe('south');
    expect(directionForRoadStep({ x: 1, y: 2 }, { x: 2, y: 2 })).toBe('west');
  });
});
```

- [ ] **Step 2: Run the failing tests**

Run:

```bash
npm test -- tests/traffic/intersections.test.ts
```

Expected: FAIL because `src/traffic/intersections.ts` does not exist.

- [ ] **Step 3: Implement intersection extraction**

Create `src/traffic/intersections.ts`:

```ts
import {
  type TrafficCoord,
  type TrafficDirection,
  type TrafficIntersection,
  trafficIntersectionId,
} from './trafficTypes';

export const ROAD_NORTH = 1;
export const ROAD_EAST = 2;
export const ROAD_SOUTH = 4;
export const ROAD_WEST = 8;

export type TrafficRoadTile = {
  coord: TrafficCoord;
  mask: number;
};

export function buildTrafficIntersections(roads: Iterable<TrafficRoadTile>): TrafficIntersection[] {
  return [...roads]
    .filter((road) => roadDegree(road.mask) >= 3)
    .map((road) => ({
      intersectionId: trafficIntersectionId(road.coord),
      coord: { x: road.coord.x, y: road.coord.y },
      connectedDirections: connectedDirections(road.mask),
    }))
    .sort((a, b) => a.coord.y - b.coord.y || a.coord.x - b.coord.x);
}

export function directionForRoadStep(from: TrafficCoord, to: TrafficCoord): TrafficDirection | undefined {
  const dx = Math.sign(to.x - from.x);
  const dy = Math.sign(to.y - from.y);
  if (dx === 0 && dy === 1) return 'north';
  if (dx === -1 && dy === 0) return 'east';
  if (dx === 0 && dy === -1) return 'south';
  if (dx === 1 && dy === 0) return 'west';
  return undefined;
}

function connectedDirections(mask: number): TrafficDirection[] {
  const result: TrafficDirection[] = [];
  if ((mask & ROAD_NORTH) !== 0) result.push('north');
  if ((mask & ROAD_EAST) !== 0) result.push('east');
  if ((mask & ROAD_SOUTH) !== 0) result.push('south');
  if ((mask & ROAD_WEST) !== 0) result.push('west');
  return result;
}

function roadDegree(mask: number): number {
  return [ROAD_NORTH, ROAD_EAST, ROAD_SOUTH, ROAD_WEST]
    .filter((direction) => (mask & direction) !== 0)
    .length;
}
```

- [ ] **Step 4: Run the intersection tests**

Run:

```bash
npm test -- tests/traffic/intersections.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/traffic/intersections.ts tests/traffic/intersections.test.ts
git commit -m "feat: derive traffic intersections from roads"
```

## Task 3: Vehicle Reservation Requests

**Files:**
- Create: `tests/traffic/vehicleTrafficRequests.test.ts`
- Create: `src/traffic/vehicleTrafficRequests.ts`

- [ ] **Step 1: Write the failing route request tests**

Create `tests/traffic/vehicleTrafficRequests.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { buildTrafficRequestsForVehicles } from '../../src/traffic/vehicleTrafficRequests';

describe('vehicle traffic requests', () => {
  it('creates a reservation request for the next visible intersection ahead of a vehicle', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 120,
      intersections: new Map([
        ['1:0', { intersectionId: 'intersection:1:0', coord: { x: 1, y: 0 }, connectedDirections: ['west', 'south', 'east'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:7',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 1, y: 1 }],
          offset: 0.25,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([
      expect.objectContaining({
        vehicleId: 'vehicle:7',
        intersectionId: 'intersection:1:0',
        currentOffset: 0.25,
        distanceToIntersection: 0.75,
        stopOffset: expect.any(Number),
        enterTick: expect.any(Number),
        exitTick: expect.any(Number),
        approachEdge: 'west',
        exitEdge: 'south',
        conflictMask: 1,
      }),
    ]);
    expect(requests[0].stopOffset).toBeLessThan(1);
    expect(requests[0].enterTick).toBeGreaterThanOrEqual(120);
    expect(requests[0].exitTick).toBeGreaterThan(requests[0].enterTick);
  });

  it('does not request intersections behind the vehicle or beyond the lookahead', () => {
    const requests = buildTrafficRequestsForVehicles({
      tick: 9,
      lookaheadTiles: 1.25,
      intersections: new Map([
        ['3:0', { intersectionId: 'intersection:3:0', coord: { x: 3, y: 0 }, connectedDirections: ['west', 'east', 'south'] }],
      ]),
      vehicles: [
        {
          vehicleId: 'vehicle:1',
          path: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 2, y: 0 }, { x: 3, y: 0 }],
          offset: 0.5,
          speed: 1,
        },
      ],
    });

    expect(requests).toEqual([]);
  });
});
```

- [ ] **Step 2: Run the failing tests**

Run:

```bash
npm test -- tests/traffic/vehicleTrafficRequests.test.ts
```

Expected: FAIL because `vehicleTrafficRequests.ts` does not exist.

- [ ] **Step 3: Implement route request extraction**

Create `src/traffic/vehicleTrafficRequests.ts`:

```ts
import { directionForRoadStep } from './intersections';
import {
  type TrafficIntersection,
  type TrafficVehicleRequest,
  type VehicleId,
  trafficKey,
} from './trafficTypes';

export type TrafficVehicleRouteState = {
  vehicleId: VehicleId;
  path: readonly { x: number; y: number }[];
  offset: number;
  speed: number;
};

export type BuildTrafficRequestsInput = {
  tick: number;
  vehicles: readonly TrafficVehicleRouteState[];
  intersections: ReadonlyMap<string, TrafficIntersection>;
  lookaheadTiles?: number;
  stopDistanceTiles?: number;
  ticksPerTile?: number;
};

export function buildTrafficRequestsForVehicles(input: BuildTrafficRequestsInput): TrafficVehicleRequest[] {
  const lookaheadTiles = input.lookaheadTiles ?? 2.25;
  const stopDistanceTiles = input.stopDistanceTiles ?? 0.42;
  const ticksPerTile = input.ticksPerTile ?? 8;

  return input.vehicles.flatMap((vehicle) => {
    if (vehicle.path.length < 3) return [];
    const base = positiveModulo(Math.floor(vehicle.offset), vehicle.path.length);
    const fraction = vehicle.offset - Math.floor(vehicle.offset);

    for (let step = 1; step <= Math.ceil(lookaheadTiles) + 1; step += 1) {
      const pathIndex = (base + step) % vehicle.path.length;
      const coord = vehicle.path[pathIndex];
      const intersection = input.intersections.get(trafficKey(coord));
      if (!intersection) continue;

      const distanceToIntersection = step - fraction;
      if (distanceToIntersection < 0 || distanceToIntersection > lookaheadTiles) return [];

      const previous = vehicle.path[positiveModulo(pathIndex - 1, vehicle.path.length)];
      const next = vehicle.path[(pathIndex + 1) % vehicle.path.length];
      const approachEdge = directionForRoadStep(previous, coord);
      const exitEdge = directionForRoadStep(next, coord);
      if (!approachEdge || !exitEdge) return [];

      const enterTick = input.tick + Math.max(1, Math.ceil(distanceToIntersection * ticksPerTile));
      const exitTick = enterTick + Math.max(2, Math.ceil((1 / Math.max(0.1, vehicle.speed)) * ticksPerTile));
      return [{
        vehicleId: vehicle.vehicleId,
        intersectionId: intersection.intersectionId,
        distanceToIntersection: Number(distanceToIntersection.toFixed(3)),
        stopOffset: normalizeOffset(pathIndex - stopDistanceTiles, vehicle.path.length),
        currentOffset: vehicle.offset,
        enterTick,
        exitTick,
        approachEdge,
        exitEdge,
        conflictMask: 1,
        priority: stableVehiclePriority(vehicle.vehicleId),
      }];
    }

    return [];
  });
}

function stableVehiclePriority(vehicleId: VehicleId): number {
  const id = Number(vehicleId.split(':')[1]);
  return Number.isFinite(id) ? id : 0;
}

function normalizeOffset(offset: number, pathLength: number): number {
  return Number(positiveModuloFloat(offset, pathLength).toFixed(3));
}

function positiveModulo(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}

function positiveModuloFloat(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}
```

- [ ] **Step 4: Run request tests**

Run:

```bash
npm test -- tests/traffic/vehicleTrafficRequests.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/traffic/vehicleTrafficRequests.ts tests/traffic/vehicleTrafficRequests.test.ts
git commit -m "feat: build vehicle traffic requests"
```

## Task 4: Local Reservation Authority

**Files:**
- Create: `tests/traffic/localTrafficAuthority.test.ts`
- Create: `src/traffic/localTrafficAuthority.ts`

- [ ] **Step 1: Write failing authority tests**

Create `tests/traffic/localTrafficAuthority.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { stepLocalTrafficAuthority } from '../../src/traffic/localTrafficAuthority';
import { createInitialTrafficRuleSnapshot, type TrafficVehicleRequest } from '../../src/traffic/trafficTypes';

function request(overrides: Partial<TrafficVehicleRequest>): TrafficVehicleRequest {
  return {
    vehicleId: 'vehicle:1',
    intersectionId: 'intersection:4:4',
    distanceToIntersection: 0.8,
    stopOffset: 2.58,
    currentOffset: 1.9,
    enterTick: 20,
    exitTick: 26,
    approachEdge: 'west',
    exitEdge: 'east',
    conflictMask: 1,
    priority: 1,
    ...overrides,
  };
}

describe('local traffic authority', () => {
  it('grants only one reservation for conflicting intersection requests', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:2', priority: 2 }),
        request({ vehicleId: 'vehicle:1', priority: 1 }),
      ],
    });

    expect(result.snapshot.reservations).toHaveLength(1);
    expect(result.snapshot.reservations[0].vehicleId).toBe('vehicle:1');
    expect(result.decisions.get('vehicle:1')?.kind).toBe('go');
    expect(result.decisions.get('vehicle:2')?.kind).toBe('yield');
    expect(result.diagnostics.intersectionConflictsPrevented).toBe(1);
  });

  it('allows non-conflicting reservations in the same time window', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', conflictMask: 1 }),
        request({ vehicleId: 'vehicle:2', conflictMask: 2 }),
      ],
    });

    expect(result.snapshot.reservations.map((reservation) => reservation.vehicleId).sort()).toEqual([
      'vehicle:1',
      'vehicle:2',
    ]);
    expect(result.decisions.get('vehicle:1')?.kind).toBe('go');
    expect(result.decisions.get('vehicle:2')?.kind).toBe('go');
  });

  it('expires old reservations before evaluating new requests', () => {
    const first = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [request({ vehicleId: 'vehicle:1', enterTick: 12, exitTick: 14 })],
    });
    const second = stepLocalTrafficAuthority({
      snapshot: first.snapshot,
      tick: 15,
      requests: [request({ vehicleId: 'vehicle:2', enterTick: 16, exitTick: 18 })],
    });

    expect(second.snapshot.reservations).toHaveLength(1);
    expect(second.snapshot.reservations[0].vehicleId).toBe('vehicle:2');
    expect(second.diagnostics.expiredReservations).toBe(1);
  });

  it('stops a blocked car before it crosses its stop boundary', () => {
    const result = stepLocalTrafficAuthority({
      snapshot: createInitialTrafficRuleSnapshot(),
      tick: 10,
      requests: [
        request({ vehicleId: 'vehicle:1', priority: 1, currentOffset: 2.1 }),
        request({ vehicleId: 'vehicle:2', priority: 2, currentOffset: 2.5, stopOffset: 2.58 }),
      ],
    });

    const blocked = result.decisions.get('vehicle:2');
    expect(blocked?.kind).toBe('stop');
    expect(blocked?.speedFactor).toBe(0);
    expect(blocked?.maxAdvance).toBeCloseTo(0.08, 3);
  });
});
```

- [ ] **Step 2: Run failing authority tests**

Run:

```bash
npm test -- tests/traffic/localTrafficAuthority.test.ts
```

Expected: FAIL because `localTrafficAuthority.ts` does not exist.

- [ ] **Step 3: Implement local authority**

Create `src/traffic/localTrafficAuthority.ts`:

```ts
import {
  EMPTY_TRAFFIC_DIAGNOSTICS,
  type TrafficDecision,
  type TrafficReservation,
  type TrafficRuleDiagnostics,
  type TrafficRuleSnapshot,
  type TrafficVehicleRequest,
  trafficReservationId,
} from './trafficTypes';

export type StepLocalTrafficAuthorityInput = {
  snapshot: TrafficRuleSnapshot;
  tick: number;
  requests: readonly TrafficVehicleRequest[];
};

export type StepLocalTrafficAuthorityResult = {
  snapshot: TrafficRuleSnapshot;
  decisions: Map<string, TrafficDecision>;
  diagnostics: TrafficRuleDiagnostics;
};

export function stepLocalTrafficAuthority(input: StepLocalTrafficAuthorityInput): StepLocalTrafficAuthorityResult {
  const diagnostics: TrafficRuleDiagnostics = { ...EMPTY_TRAFFIC_DIAGNOSTICS };
  const activeReservations = input.snapshot.reservations.filter((reservation) => {
    const active = reservation.exitTick > input.tick;
    if (!active) diagnostics.expiredReservations += 1;
    return active;
  });
  const nextReservations: TrafficReservation[] = [...activeReservations];
  const decisions = new Map<string, TrafficDecision>();

  const sortedRequests = [...input.requests].sort((a, b) =>
    a.enterTick - b.enterTick ||
    a.priority - b.priority ||
    a.vehicleId.localeCompare(b.vehicleId)
  );

  for (const request of sortedRequests) {
    diagnostics.trafficRuleDecisionCount += 1;
    const existing = nextReservations.find((reservation) => reservation.vehicleId === request.vehicleId);
    if (existing) {
      decisions.set(request.vehicleId, {
        vehicleId: request.vehicleId,
        kind: 'go',
        speedFactor: 1,
        intersectionId: request.intersectionId,
        reservationId: existing.reservationId,
      });
      continue;
    }

    const conflict = nextReservations.find((reservation) => reservationsConflict(reservation, request));
    if (conflict) {
      diagnostics.intersectionConflictsPrevented += 1;
      const stopAdvance = distanceToStopBoundary(request.currentOffset, request.stopOffset);
      const nearStopBoundary = stopAdvance <= 0.16 || request.distanceToIntersection <= 0.48;
      const kind = nearStopBoundary ? 'stop' : 'yield';
      if (kind === 'stop') diagnostics.stoppedForTrafficRules += 1;
      if (kind === 'yield') diagnostics.yieldingVehicles += 1;
      diagnostics.blockedVehicles += 1;
      decisions.set(request.vehicleId, {
        vehicleId: request.vehicleId,
        kind,
        speedFactor: kind === 'stop' ? 0 : 0.28,
        maxAdvance: kind === 'stop' ? Math.max(0, Number(stopAdvance.toFixed(3))) : undefined,
        intersectionId: request.intersectionId,
      });
      continue;
    }

    const reservation: TrafficReservation = {
      reservationId: trafficReservationId(request.intersectionId, request.vehicleId, request.enterTick),
      intersectionId: request.intersectionId,
      vehicleId: request.vehicleId,
      enterTick: request.enterTick,
      exitTick: request.exitTick,
      approachEdge: request.approachEdge,
      exitEdge: request.exitEdge,
      conflictMask: request.conflictMask,
      priority: request.priority,
    };
    nextReservations.push(reservation);
    decisions.set(request.vehicleId, {
      vehicleId: request.vehicleId,
      kind: 'go',
      speedFactor: 1,
      intersectionId: request.intersectionId,
      reservationId: reservation.reservationId,
    });
  }

  diagnostics.reservedIntersections = new Set(nextReservations.map((reservation) => reservation.intersectionId)).size;

  return {
    snapshot: {
      tick: input.tick,
      version: input.snapshot.version + 1,
      reservations: nextReservations,
      diagnostics,
    },
    decisions,
    diagnostics,
  };
}

function reservationsConflict(reservation: TrafficReservation, request: TrafficVehicleRequest): boolean {
  return (
    reservation.intersectionId === request.intersectionId &&
    windowsOverlap(reservation.enterTick, reservation.exitTick, request.enterTick, request.exitTick) &&
    (reservation.conflictMask & request.conflictMask) !== 0
  );
}

function windowsOverlap(aStart: number, aEnd: number, bStart: number, bEnd: number): boolean {
  return aStart < bEnd && bStart < aEnd;
}

function distanceToStopBoundary(currentOffset: number, stopOffset: number): number {
  return stopOffset >= currentOffset ? stopOffset - currentOffset : 0;
}
```

- [ ] **Step 4: Run authority tests**

Run:

```bash
npm test -- tests/traffic/localTrafficAuthority.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/traffic/localTrafficAuthority.ts tests/traffic/localTrafficAuthority.test.ts
git commit -m "feat: reserve intersection traffic slots"
```

## Task 5: Integrate Traffic Rules Into Cars

**Files:**
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Add a failing smoke assertion for traffic diagnostics**

Modify `tests/e2e/render-smoke.spec.ts` after the existing vehicle assertions:

```ts
  expect(state.city.vehicleDiagnostics.trafficRuleDecisionCount).toEqual(expect.any(Number));
  expect(state.city.vehicleDiagnostics.reservedIntersections).toEqual(expect.any(Number));
  expect(state.city.vehicleDiagnostics.stoppedForTrafficRules).toEqual(expect.any(Number));
```

- [ ] **Step 2: Run smoke test to verify failure**

Run:

```bash
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: FAIL because the diagnostics do not exist in `render_game_to_text`.

- [ ] **Step 3: Add imports and Car ID state**

Modify `src/main.ts` imports:

```ts
import { buildTrafficIntersections } from './traffic/intersections';
import { stepLocalTrafficAuthority } from './traffic/localTrafficAuthority';
import {
  createInitialTrafficRuleSnapshot,
  type TrafficIntersection,
  type TrafficRuleSnapshot,
  type VehicleId,
  trafficKey,
} from './traffic/trafficTypes';
import { buildTrafficRequestsForVehicles } from './traffic/vehicleTrafficRequests';
```

Modify the `Car` type:

```ts
type Car = {
  id: VehicleId;
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSprite;
};
```

Add module state near the existing `cars` state:

```ts
let trafficIntersections: TrafficIntersection[] = [];
let trafficIntersectionLookup = new Map<string, TrafficIntersection>();
let trafficRuleSnapshot: TrafficRuleSnapshot = createInitialTrafficRuleSnapshot();
let trafficTick = 0;
```

- [ ] **Step 4: Build intersections after roads exist**

In `init()`, after `vehicleCautionTiles = buildVehicleCautionTiles();` or next to existing road-derived setup, add:

```ts
  trafficIntersections = buildTrafficIntersections([...roads.values()].map((road) => ({
    coord: road.coord,
    mask: road.mask,
  })));
  trafficIntersectionLookup = new Map(trafficIntersections.map((intersection) => [trafficKey(intersection.coord), intersection]));
```

- [ ] **Step 5: Assign stable vehicle IDs**

In `buildCars`, update returned cars:

```ts
    return {
      id: `vehicle:${index}`,
      path,
      offset: (index * 7 + Math.floor(index / corridors.length) * 3) % path.length,
      speed: 1.15 + (index % 9) * 0.13,
      sprite: sprites[index % sprites.length],
    };
```

- [ ] **Step 6: Apply traffic decisions before advancing cars**

Replace `advanceCars(dt)` with this structure:

```ts
function advanceCars(dt: number): void {
  trafficTick += 1;
  const leaderOffsets = carLeaderOffsets(cars);
  const trafficRequests = buildTrafficRequestsForVehicles({
    tick: trafficTick,
    intersections: trafficIntersectionLookup,
    vehicles: cars.map((car) => ({
      vehicleId: car.id,
      path: car.path,
      offset: car.offset,
      speed: car.speed,
    })),
  });
  const trafficResult = stepLocalTrafficAuthority({
    snapshot: trafficRuleSnapshot,
    tick: trafficTick,
    requests: trafficRequests,
  });
  trafficRuleSnapshot = trafficResult.snapshot;

  for (const car of cars) {
    const leaderOffset = leaderOffsets.get(car);
    const trafficDecision = trafficResult.decisions.get(car.id);
    const speedFactor = vehicleSpeedFactor({
      path: car.path,
      offset: car.offset,
      cautionTileKeys: vehicleCautionTiles,
    });
    const followingFactor = vehicleFollowingSpeedFactor({
      offset: car.offset,
      leaderOffset,
      pathLength: car.path.length,
    });
    const trafficFactor = trafficDecision?.speedFactor ?? 1;
    const desiredAdvance = car.speed * speedFactor * followingFactor * trafficFactor * dt;
    const followingAdvanceLimit = vehicleFollowingAdvanceLimit({
      offset: car.offset,
      leaderOffset,
      pathLength: car.path.length,
    });
    const trafficAdvanceLimit = trafficDecision?.maxAdvance ?? Number.POSITIVE_INFINITY;
    const safeAdvance = Math.min(desiredAdvance, followingAdvanceLimit, trafficAdvanceLimit);
    car.offset = (car.offset + safeAdvance) % car.path.length;
  }
}
```

- [ ] **Step 7: Add traffic diagnostics to `vehicleDiagnostics()`**

Extend the object returned by `vehicleDiagnostics()`:

```ts
    reservedIntersections: trafficRuleSnapshot.diagnostics.reservedIntersections,
    yieldingVehicles: trafficRuleSnapshot.diagnostics.yieldingVehicles,
    stoppedForTrafficRules: trafficRuleSnapshot.diagnostics.stoppedForTrafficRules,
    blockedVehicles: trafficRuleSnapshot.diagnostics.blockedVehicles,
    intersectionConflictsPrevented: trafficRuleSnapshot.diagnostics.intersectionConflictsPrevented,
    expiredReservations: trafficRuleSnapshot.diagnostics.expiredReservations,
    trafficRuleDecisionCount: trafficRuleSnapshot.diagnostics.trafficRuleDecisionCount,
    unclassifiedTrafficRequests: trafficRuleSnapshot.diagnostics.unclassifiedTrafficRequests,
```

- [ ] **Step 8: Run focused integration tests**

Run:

```bash
npm test -- tests/traffic/intersections.test.ts tests/traffic/vehicleTrafficRequests.test.ts tests/traffic/localTrafficAuthority.test.ts
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/main.ts tests/e2e/render-smoke.spec.ts
git commit -m "feat: apply local traffic rule authority"
```

## Task 6: Runtime Verification And Visual QA

**Files:**
- Modify only if verification reveals a bug.

- [ ] **Step 1: Run full test suite**

Run:

```bash
npm test
```

Expected: all tests pass.

- [ ] **Step 2: Run production build**

Run:

```bash
npm run build
```

Expected: build succeeds.

- [ ] **Step 3: Verify the live browser on the existing port**

Do not start a parallel dev server if port `5176` is already active. Use the existing browser/app URL:

```bash
node --input-type=module -e "import { chromium } from '@playwright/test'; const browser = await chromium.launch(); const page = await browser.newPage({ viewport: { width: 900, height: 720 } }); await page.goto('http://127.0.0.1:5176/', { waitUntil: 'load' }); await page.waitForFunction(() => typeof window.render_game_to_text === 'function'); await page.waitForTimeout(500); const state = JSON.parse(await page.evaluate(() => window.render_game_to_text())); console.log(JSON.stringify({ cars: state.city.cars, vehicleDiagnostics: state.city.vehicleDiagnostics }, null, 2)); await browser.close();"
```

Expected:

- `cars` remains greater than `0`.
- `pathTilesOffRoad` remains `0`.
- `pathTilesOnRails` remains `0`.
- `teleportingVehiclePaths` remains `0`.
- `illegalVehicleUTurnPaths` remains `0`.
- `trafficRuleDecisionCount` is a number.
- no visible UI is required.

- [ ] **Step 4: Browser screenshot check**

Use the Browser plugin or existing playtest script against `http://127.0.0.1:5176/`. Confirm visually:

- cars still render on roads,
- no signs, signals, or HUD were added,
- cars do not freeze globally,
- yielding/stop behavior appears near busy intersections.

- [ ] **Step 5: Commit verification-only fixes if any**

If this task required bug fixes:

```bash
git add src tests
git commit -m "fix: stabilize traffic rule integration"
```

If no fixes were needed, do not create an empty commit.

## Task 7: Final Review

**Files:**
- No code files unless review finds issues.

- [ ] **Step 1: Check diff and status**

Run:

```bash
git status --short --branch
git log --oneline -6
```

Expected: only intentional commits are present; no unrelated files are staged or modified.

- [ ] **Step 2: Re-read the design success criteria**

Open:

```bash
sed -n '1,260p' docs/superpowers/specs/2026-05-14-traffic-rules-design.md
```

Confirm:

- no visible UI/signs/lights,
- local TypeScript authority is marked as reference model,
- data is serializable and stable-ID based,
- diagnostics exist,
- no global all-cars/all-intersections scan was introduced.

- [ ] **Step 3: Final verification before reporting**

Run:

```bash
npm test && npm run build
```

Expected: both commands exit `0`.

Report the commit hashes, verification output summary, and any remaining known limitations.
