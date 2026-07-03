import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { generateInteriorPlan, type MainDoor } from '../../src/diorama/ksw/interior/generatePlan';
import { decomposeToZones, type Zone } from '../../src/diorama/ksw/interior/zones';
import type { Room, WallSide } from '../../src/diorama/ksw/floorPlan';

type Rect = { x: number; z: number; w: number; d: number };
const EPS = 1e-6;

function inside(rect: Rect, x: number, z: number, margin: number): boolean {
  return (
    x >= rect.x - rect.w / 2 + margin - EPS &&
    x <= rect.x + rect.w / 2 - margin + EPS &&
    z >= rect.z - rect.d / 2 + margin - EPS &&
    z <= rect.z + rect.d / 2 - margin + EPS
  );
}

function wallLine(room: Room, side: WallSide): { fixed: number; along: 'x' | 'z'; center: number; len: number } {
  const r = room.rect;
  switch (side) {
    case 'n':
      return { fixed: r.z - r.d / 2, along: 'x', center: r.x, len: r.w };
    case 's':
      return { fixed: r.z + r.d / 2, along: 'x', center: r.x, len: r.w };
    case 'w':
      return { fixed: r.x - r.w / 2, along: 'z', center: r.z, len: r.d };
    case 'e':
      return { fixed: r.x + r.w / 2, along: 'z', center: r.z, len: r.d };
  }
}

// Two 20x20 zones side by side (a 6 m gap between them along x): each big
// enough for a full three-row ladder, close enough for a cross-corridor.
const FIXTURE_ZONES: Zone[] = [
  { id: 'z0', x: -13, z: 0, w: 20, d: 20 },
  { id: 'z1', x: 13, z: 0, w: 20, d: 20 },
];
const FIXTURE_DOOR: MainDoor = { x: -13, z: 10, yaw: 0 };

describe('generateInteriorPlan — 2-zone fixture', () => {
  it('emits at least two corridors per zone (the ladder spines)', () => {
    const plan = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    for (const z of FIXTURE_ZONES) {
      const inZone = plan.corridors.filter(
        (c) => c.x >= z.x - z.w / 2 - EPS && c.x <= z.x + z.w / 2 + EPS && c.z >= z.z - z.d / 2 - EPS && c.z <= z.z + z.d / 2 + EPS,
      );
      expect(inZone.length, `zone ${z.id} corridors`).toBeGreaterThanOrEqual(2);
    }
  });

  it('has at least one cross-corridor connecting the two adjacent zones', () => {
    const plan = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    // a connector sits in the gap between the two zones (x roughly 0)
    const connector = plan.corridors.find((c) => Math.abs(c.x) < 6 && c.w < c.d + 20 && c.z > -EPS && c.z < EPS + 1);
    expect(connector, 'no cross-corridor between the two zones').toBeTruthy();
  });

  it('every room sits fully inside its zone rect', () => {
    const plan = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    for (const room of plan.rooms) {
      const z = FIXTURE_ZONES.find(
        (zn) => room.rect.x >= zn.x - zn.w / 2 - EPS && room.rect.x <= zn.x + zn.w / 2 + EPS,
      )!;
      expect(z, `room ${room.id} in no zone`).toBeTruthy();
      expect(room.rect.x - room.rect.w / 2).toBeGreaterThanOrEqual(z.x - z.w / 2 - EPS);
      expect(room.rect.x + room.rect.w / 2).toBeLessThanOrEqual(z.x + z.w / 2 + EPS);
      expect(room.rect.z - room.rect.d / 2).toBeGreaterThanOrEqual(z.z - z.d / 2 - EPS);
      expect(room.rect.z + room.rect.d / 2).toBeLessThanOrEqual(z.z + z.d / 2 + EPS);
    }
  });

  it('the door zone contains reception + emergency room labels', () => {
    const plan = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    const doorZoneRooms = plan.rooms.filter((r) => r.id.startsWith('z0-'));
    const labels = doorZoneRooms.map((r) => r.label).join(' | ');
    expect(labels).toContain('Empfang');
    expect(labels).toContain('Notfall');
  });

  it('is deterministic', () => {
    const a = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    const b = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    expect(a).toEqual(b);
  });
});

describe('generateInteriorPlan — real KSW footprint', () => {
  const dataPath = resolve(__dirname, '../../data/winterthur/buildings.json');
  const data = JSON.parse(readFileSync(dataPath, 'utf-8')) as {
    buildings: Array<{ zone: string; footprint: number[][]; door?: MainDoor }>;
  };
  const kswBuildings = data.buildings.filter((b) => b.zone === 'ksw');
  function shoelace(r: number[][]): number {
    let a = 0;
    for (let i = 0, j = r.length - 1; i < r.length; j = i++) a += r[j][0] * r[i][1] - r[i][0] * r[j][1];
    return Math.abs(a) / 2;
  }
  let main = kswBuildings[0];
  let maxA = shoelace(main.footprint);
  for (const b of kswBuildings) {
    const a = shoelace(b.footprint);
    if (a > maxA) {
      maxA = a;
      main = b;
    }
  }
  const zones = decomposeToZones(main.footprint);
  const door: MainDoor = main.door ?? { x: zones[0].x, z: zones[0].z, yaw: 0 };
  const plan = generateInteriorPlan(zones, door);

  it('produces a non-trivial furnished plan', () => {
    expect(plan.rooms.length).toBeGreaterThan(10);
    expect(plan.corridors.length).toBeGreaterThanOrEqual(zones.length * 2);
  });

  it('every room is fully inside one of the zone rects', () => {
    for (const room of plan.rooms) {
      const zoneId = room.id.split('-r')[0];
      const z = zones.find((zn) => zn.id === zoneId)!;
      expect(z, `room ${room.id} maps to no zone`).toBeTruthy();
      expect(room.rect.x - room.rect.w / 2).toBeGreaterThanOrEqual(z.x - z.w / 2 - EPS);
      expect(room.rect.x + room.rect.w / 2).toBeLessThanOrEqual(z.x + z.w / 2 + EPS);
      expect(room.rect.z - room.rect.d / 2).toBeGreaterThanOrEqual(z.z - z.d / 2 - EPS);
      expect(room.rect.z + room.rect.d / 2).toBeLessThanOrEqual(z.z + z.d / 2 + EPS);
    }
  });

  it('every door fits inside its wall with the floorPlan end margin', () => {
    for (const room of plan.rooms) {
      for (const d of room.doors) {
        const wl = wallLine(room, d.wall);
        expect(Math.abs(d.center) + d.width / 2, `${room.id} ${d.wall} door`).toBeLessThanOrEqual(wl.len / 2 - 0.3 + EPS);
      }
    }
  });

  it('every room has at least one door', () => {
    for (const room of plan.rooms) {
      expect(room.doors.length, room.id).toBeGreaterThanOrEqual(1);
    }
  });

  it('all props stay inside their room (clear of walls)', () => {
    for (const room of plan.rooms) {
      for (const p of room.props) {
        expect(inside(room.rect, p.x, p.z, 0.5), `${room.id}: ${p.kind} at ${p.x.toFixed(1)},${p.z.toFixed(1)}`).toBe(true);
      }
    }
  });

  it('all people stay inside their room', () => {
    for (const room of plan.rooms) {
      for (const p of room.people) {
        expect(inside(room.rect, p.x, p.z, 0.4), `${room.id}: ${p.role} at ${p.x.toFixed(1)},${p.z.toFixed(1)}`).toBe(true);
      }
    }
  });

  it('the door zone carries the reception + emergency departments', () => {
    const allLabels = plan.rooms.map((r) => r.label).join(' | ');
    expect(allLabels).toContain('Empfang');
    expect(allLabels).toContain('Notfall');
  });

  it('is deterministic across calls', () => {
    const a = generateInteriorPlan(zones, door);
    const b = generateInteriorPlan(zones, door);
    expect(a).toEqual(b);
  });
});
