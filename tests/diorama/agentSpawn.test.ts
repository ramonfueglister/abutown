import { describe, expect, it } from 'vitest';
import { buildNav } from '../../src/diorama/ksw/nav';
import { buildSpawnSpecs, planSpawnSpecs } from '../../src/diorama/ksw/agentSpawn';
import { kswPlan } from '../../src/diorama/ksw/floorPlan';
import type { PersonRole } from '../../src/diorama/ksw/floorPlan';

const nav = buildNav(kswPlan);

const planCount =
  kswPlan.rooms.reduce((n, r) => n + r.people.length, 0) + kswPlan.outdoorPeople.length + kswPlan.walkers.length;

function insideRect(r: { x: number; z: number; w: number; d: number }, x: number, z: number, pad = 0): boolean {
  return Math.abs(x - r.x) <= r.w / 2 + pad && Math.abs(z - r.z) <= r.d / 2 + pad;
}

describe('agentSpawn', () => {
  it('default (no count) is exactly the authored plan crowd', () => {
    const specs = buildSpawnSpecs(kswPlan, nav);
    expect(specs).toEqual(planSpawnSpecs(kswPlan));
    expect(specs.length).toBe(planCount);
  });

  it('is deterministic: same count, same crowd', () => {
    expect(buildSpawnSpecs(kswPlan, nav, 500)).toEqual(buildSpawnSpecs(kswPlan, nav, 500));
  });

  it('scaling keeps the authored people as a stable prefix with sequential seeds', () => {
    const big = buildSpawnSpecs(kswPlan, nav, 5000);
    expect(big.length).toBe(5000);
    expect(big.slice(0, planCount)).toEqual(planSpawnSpecs(kswPlan));
    for (const [i, s] of big.entries()) expect(s.spec.seed).toBe(i + 1);
  });

  it('counts below the plan slice the plan', () => {
    const few = buildSpawnSpecs(kswPlan, nav, 10);
    expect(few).toEqual(planSpawnSpecs(kswPlan).slice(0, 10));
  });

  it('extras extrapolate the plan role mix and hit the 40/30/30 kind mix', () => {
    const specs = buildSpawnSpecs(kswPlan, nav, 10000);
    const extras = specs.slice(planCount);
    const kinds = { rounds: 0, outdoor: 0, resident: 0 };
    for (const s of extras) kinds[s.spec.kind] += 1;
    expect(kinds.rounds / extras.length).toBeCloseTo(0.4, 1);
    expect(kinds.outdoor / extras.length).toBeCloseTo(0.3, 1);
    expect(kinds.resident / extras.length).toBeCloseTo(0.3, 1);

    const planRoles: Partial<Record<PersonRole, number>> = {};
    for (const s of planSpawnSpecs(kswPlan)) planRoles[s.spec.role] = (planRoles[s.spec.role] ?? 0) + 1;
    const extraRoles: Partial<Record<PersonRole, number>> = {};
    for (const s of extras) extraRoles[s.spec.role] = (extraRoles[s.spec.role] ?? 0) + 1;
    for (const [role, n] of Object.entries(planRoles) as Array<[PersonRole, number]>) {
      expect((extraRoles[role] ?? 0) / extras.length).toBeCloseTo(n / planCount, 1);
    }
  });

  it('every extra spawns on legal ground for its kind', () => {
    const specs = buildSpawnSpecs(kswPlan, nav, 2000).slice(planCount);
    for (const s of specs) {
      const [x, z] = s.spec.home;
      if (s.spec.kind === 'resident') {
        const room = kswPlan.rooms.find((r) => r.id === s.spec.homeRoomId);
        expect(room, `room ${s.spec.homeRoomId}`).toBeDefined();
        expect(insideRect(room!.rect, x, z), `resident ${x},${z} in ${room!.id}`).toBe(true);
      } else if (s.spec.kind === 'outdoor') {
        expect(s.spec.homeRoomId).toBeNull();
        expect(
          kswPlan.outdoorSlabs.some((slab) => insideRect(slab, x, z)),
          `outdoor ${x},${z} on a slab`,
        ).toBe(true);
      } else {
        expect(s.spec.homeRoomId).toBeNull();
        expect([nav.lanes.a, nav.lanes.b]).toContain(z);
        expect(x).toBeGreaterThanOrEqual(nav.lanes.xMin);
        expect(x).toBeLessThanOrEqual(nav.lanes.xMax);
      }
    }
  });
});
