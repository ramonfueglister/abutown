import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { buildZonedNav, zonedReachableRooms } from '../../src/diorama/ksw/nav';
import { generateInteriorPlan, type MainDoor } from '../../src/diorama/ksw/interior/generatePlan';
import { decomposeToZones, type Zone } from '../../src/diorama/ksw/interior/zones';

const FIXTURE_ZONES: Zone[] = [
  { id: 'z0', x: -13, z: 0, w: 20, d: 20 },
  { id: 'z1', x: 13, z: 0, w: 20, d: 20 },
];
const FIXTURE_DOOR: MainDoor = { x: -13, z: 10, yaw: 0 };

describe('buildZonedNav — 2-zone fixture', () => {
  it('reaches every room from the main door across both zones', () => {
    const plan = generateInteriorPlan(FIXTURE_ZONES, FIXTURE_DOOR);
    const nav = buildZonedNav(plan, FIXTURE_DOOR);
    const reachable = zonedReachableRooms(nav);
    const allRoomIds = new Set(plan.rooms.map((r) => r.id));
    for (const id of allRoomIds) {
      expect(reachable.has(id), `room ${id} unreachable from the main door`).toBe(true);
    }
    // and it actually visits rooms in BOTH zones
    expect([...reachable].some((id) => id.startsWith('z0-'))).toBe(true);
    expect([...reachable].some((id) => id.startsWith('z1-'))).toBe(true);
  });
});

describe('buildZonedNav — real KSW footprint', () => {
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

  it('reaches every generated room from the main door', () => {
    const nav = buildZonedNav(plan, door);
    const reachable = zonedReachableRooms(nav);
    const missing = plan.rooms.map((r) => r.id).filter((id) => !reachable.has(id));
    expect(missing, `${missing.length} rooms unreachable: ${missing.slice(0, 8).join(', ')}`).toEqual([]);
  });
});
