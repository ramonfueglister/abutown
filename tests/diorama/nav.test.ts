import { describe, expect, it } from 'vitest';
import { buildNav, routePath, type NavGraph } from '../../src/diorama/ksw/nav';
import { kswPlan } from '../../src/diorama/ksw/floorPlan';

const nav: NavGraph = buildNav(kswPlan);

type Pt = [number, number];

function insideRect(r: { x: number; z: number; w: number; d: number }, x: number, z: number, pad = 0): boolean {
  return Math.abs(x - r.x) <= r.w / 2 + pad && Math.abs(z - r.z) <= r.d / 2 + pad;
}

// legal = inside a corridor, inside a room, or outside the building (plate)
function isLegal(x: number, z: number): boolean {
  if (kswPlan.corridors.some((c) => insideRect(c, x, z, 0.1))) return true;
  if (kswPlan.rooms.some((r) => insideRect(r.rect, x, z, 0.1))) return true;
  const b = kswPlan.building;
  const outside = !insideRect(b, x, z, -0.05);
  const onPlate = Math.abs(x) <= kswPlan.plate.w / 2 && Math.abs(z) <= kswPlan.plate.d / 2;
  return outside && onPlate;
}

function samplePath(path: Pt[], step = 0.25): Pt[] {
  const out: Pt[] = [];
  for (let i = 0; i < path.length - 1; i++) {
    const [x0, z0] = path[i];
    const [x1, z1] = path[i + 1];
    const len = Math.hypot(x1 - x0, z1 - z0);
    const n = Math.max(1, Math.ceil(len / step));
    for (let k = 0; k <= n; k++) out.push([x0 + ((x1 - x0) * k) / n, z0 + ((z1 - z0) * k) / n]);
  }
  return out;
}

describe('buildNav', () => {
  it('registers a corridor door anchor for every room', () => {
    for (const room of kswPlan.rooms) {
      expect(nav.roomDoors[room.id], room.id).toBeDefined();
    }
  });
});

describe('routePath', () => {
  it('routes between every pair of rooms without leaving legal ground', () => {
    const rooms = kswPlan.rooms;
    for (let i = 0; i < rooms.length; i++) {
      const a = rooms[i];
      const b = rooms[(i + 7) % rooms.length];
      if (a.id === b.id) continue;
      const from: Pt = [a.rect.x, a.rect.z];
      const to: Pt = [b.rect.x, b.rect.z];
      const path = routePath(nav, { point: from, roomId: a.id }, { point: to, roomId: b.id });
      expect(path.length, `${a.id} -> ${b.id}`).toBeGreaterThanOrEqual(2);
      expect(path[0]).toEqual(from);
      expect(path[path.length - 1]).toEqual(to);
      for (const [x, z] of samplePath(path)) {
        expect(isLegal(x, z), `${a.id} -> ${b.id} illegal at ${x.toFixed(1)},${z.toFixed(1)}`).toBe(true);
      }
    }
  });

  it('routes along corridors between corridor points', () => {
    const path = routePath(nav, { point: [-20, -9], roomId: null }, { point: [15, 5], roomId: null });
    for (const [x, z] of samplePath(path)) {
      expect(isLegal(x, z), `corridor route illegal at ${x},${z}`).toBe(true);
    }
  });

  it('connects the plaza to indoor targets through the reception portal', () => {
    const path = routePath(nav, { point: [2, 21], roomId: null }, { point: [4, -2], roomId: 'wardMedizin' });
    expect(path.length).toBeGreaterThan(3);
    // passes through the entrance hall (empfang rect)
    const empfang = kswPlan.rooms.find((r) => r.id === 'empfang')!;
    const throughEntrance = samplePath(path).some(([x, z]) => insideRect(empfang.rect, x, z));
    expect(throughEntrance).toBe(true);
    for (const [x, z] of samplePath(path)) {
      expect(isLegal(x, z), `portal route illegal at ${x.toFixed(1)},${z.toFixed(1)}`).toBe(true);
    }
  });

  it('keeps outdoor-to-outdoor strolls on the plate', () => {
    const path = routePath(nav, { point: [-2, 21], roomId: null }, { point: [20, 22], roomId: null });
    for (const [x, z] of samplePath(path)) {
      expect(isLegal(x, z)).toBe(true);
    }
  });
});
