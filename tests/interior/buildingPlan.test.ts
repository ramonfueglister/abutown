// Multi-storey building plan (Phase A): one FloorPlan per storey, vertically
// zoned like a real hospital. All geometry invariants of the single-floor
// generator hold per storey.
import { describe, expect, it } from 'vitest';
import { generateBuildingPlan } from '../../src/diorama/ksw/interior/generatePlan';
import type { Zone } from '../../src/diorama/ksw/interior/zones';

const zones: Zone[] = [
  { id: 'z0', x: 0, z: 0, w: 60, d: 30 },
  { id: 'z1', x: 50, z: 0, w: 30, d: 24 },
];
const door = { x: -10, z: 15, yaw: 0 };

describe('generateBuildingPlan', () => {
  it('derives the storey count from the eave height', () => {
    expect(generateBuildingPlan(zones, door, 14).storeyCount).toBe(4);
    expect(generateBuildingPlan(zones, door, 3).storeyCount).toBe(1);
    expect(generateBuildingPlan(zones, door, 14).storeys).toHaveLength(4);
  });

  it('level 0 leads with Empfang + Notfall; imaging stays on the ground floor', () => {
    const bp = generateBuildingPlan(zones, door, 14);
    const labels0 = bp.storeys[0].rooms.map((r) => r.label).join('|');
    expect(labels0).toContain('Empfang');
    expect(labels0).toContain('Notfall');
    expect(labels0).toContain('Radiologie');
  });

  it('a middle level carries OP/IPS, an upper level carries Bettenstationen', () => {
    const bp = generateBuildingPlan(zones, door, 17); // 5 storeys
    const mid = bp.storeys[1].rooms.map((r) => r.label).join('|');
    expect(mid).toMatch(/OP|Intensiv/);
    const upper = bp.storeys[3].rooms.map((r) => r.label).join('|');
    expect(upper).toContain('Bettenstation');
  });

  it('the top level of a ≥4-storey building is Technik', () => {
    const bp = generateBuildingPlan(zones, door, 17);
    const top = bp.storeys[bp.storeyCount - 1].rooms.map((r) => r.label).join('|');
    expect(top).toContain('Technik');
  });

  it('people exist ONLY on level 0 (nav is 2D in Phase A)', () => {
    const bp = generateBuildingPlan(zones, door, 14);
    expect(bp.storeys[0].rooms.some((r) => r.people.length > 0)).toBe(true);
    for (let k = 1; k < bp.storeyCount; k++) {
      for (const room of bp.storeys[k].rooms) expect(room.people).toHaveLength(0);
    }
  });

  it('every storey keeps the single-floor invariants: rooms inside the zone set, no room overlap', () => {
    const bp = generateBuildingPlan(zones, door, 14);
    const insideSomeZone = (x: number, z: number): boolean =>
      zones.some((zn) => x >= zn.x - zn.w / 2 - 1e-6 && x <= zn.x + zn.w / 2 + 1e-6 && z >= zn.z - zn.d / 2 - 1e-6 && z <= zn.z + zn.d / 2 + 1e-6);
    for (const plan of bp.storeys) {
      for (const room of plan.rooms) {
        expect(insideSomeZone(room.rect.x - room.rect.w / 2, room.rect.z - room.rect.d / 2)).toBe(true);
        expect(insideSomeZone(room.rect.x + room.rect.w / 2, room.rect.z + room.rect.d / 2)).toBe(true);
      }
      for (let i = 0; i < plan.rooms.length; i++) {
        for (let j = i + 1; j < plan.rooms.length; j++) {
          const a = plan.rooms[i].rect;
          const b = plan.rooms[j].rect;
          const e = 1e-6;
          const overlap =
            a.x - a.w / 2 < b.x + b.w / 2 - e && b.x - b.w / 2 < a.x + a.w / 2 - e &&
            a.z - a.d / 2 < b.z + b.d / 2 - e && b.z - b.d / 2 < a.z + a.d / 2 - e;
          expect(overlap).toBe(false);
        }
      }
    }
  });

  it('is deterministic', () => {
    expect(generateBuildingPlan(zones, door, 14)).toEqual(generateBuildingPlan(zones, door, 14));
  });
});
