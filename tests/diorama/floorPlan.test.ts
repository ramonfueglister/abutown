import { describe, expect, it } from 'vitest';
import { kswPlan, type Room, type WallSide } from '../../src/diorama/ksw/floorPlan';

type Rect = { x: number; z: number; w: number; d: number };

const EPS = 1e-6;

function overlapArea(a: Rect, b: Rect): number {
  const ox = Math.min(a.x + a.w / 2, b.x + b.w / 2) - Math.max(a.x - a.w / 2, b.x - b.w / 2);
  const oz = Math.min(a.z + a.d / 2, b.z + b.d / 2) - Math.max(a.z - a.d / 2, b.z - b.d / 2);
  return Math.max(ox, 0) * Math.max(oz, 0);
}

function inside(rect: Rect, x: number, z: number, margin: number): boolean {
  return (
    x >= rect.x - rect.w / 2 + margin - EPS &&
    x <= rect.x + rect.w / 2 - margin + EPS &&
    z >= rect.z - rect.d / 2 + margin - EPS &&
    z <= rect.z + rect.d / 2 - margin + EPS
  );
}

// Wall line of a room side: returns the fixed coordinate and the span axis.
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

const b = kswPlan.building;
const perimeter = {
  xMin: b.x - b.w / 2,
  xMax: b.x + b.w / 2,
  zMin: b.z - b.d / 2,
  zMax: b.z + b.d / 2,
};

function isPerimeterWall(room: Room, side: WallSide): boolean {
  const wl = wallLine(room, side);
  if (wl.along === 'x') return Math.abs(wl.fixed - perimeter.zMin) < EPS || Math.abs(wl.fixed - perimeter.zMax) < EPS;
  return Math.abs(wl.fixed - perimeter.xMin) < EPS || Math.abs(wl.fixed - perimeter.xMax) < EPS;
}

// A door leads to a corridor when its wall lies on a corridor edge and the
// door span sits inside the corridor's extent along the wall.
function doorLeadsToCorridor(room: Room, side: WallSide, center: number, width: number): boolean {
  const wl = wallLine(room, side);
  const lo = wl.center + center - width / 2;
  const hi = wl.center + center + width / 2;
  return kswPlan.corridors.some((c) => {
    if (wl.along === 'x') {
      const touches =
        Math.abs(wl.fixed - (c.z - c.d / 2)) < EPS || Math.abs(wl.fixed - (c.z + c.d / 2)) < EPS;
      return touches && lo >= c.x - c.w / 2 - EPS && hi <= c.x + c.w / 2 + EPS;
    }
    const touches =
      Math.abs(wl.fixed - (c.x - c.w / 2)) < EPS || Math.abs(wl.fixed - (c.x + c.w / 2)) < EPS;
    return touches && lo >= c.z - c.d / 2 - EPS && hi <= c.z + c.d / 2 + EPS;
  });
}

describe('kswPlan geometry invariants', () => {
  it('has the full department roster (23 rooms)', () => {
    expect(kswPlan.rooms.length).toBe(23);
    const ids = new Set(kswPlan.rooms.map((r) => r.id));
    expect(ids.size).toBe(kswPlan.rooms.length);
  });

  it('rooms do not overlap each other', () => {
    for (let i = 0; i < kswPlan.rooms.length; i++) {
      for (let j = i + 1; j < kswPlan.rooms.length; j++) {
        const area = overlapArea(kswPlan.rooms[i].rect, kswPlan.rooms[j].rect);
        expect(area, `${kswPlan.rooms[i].id} vs ${kswPlan.rooms[j].id}`).toBeLessThan(EPS);
      }
    }
  });

  it('rooms do not overlap corridors', () => {
    for (const room of kswPlan.rooms) {
      for (const c of kswPlan.corridors) {
        expect(overlapArea(room.rect, c), `${room.id} vs corridor`).toBeLessThan(EPS);
      }
    }
  });

  it('rooms and corridors stay within the building footprint', () => {
    for (const rect of [...kswPlan.rooms.map((r) => r.rect), ...kswPlan.corridors]) {
      expect(rect.x - rect.w / 2).toBeGreaterThanOrEqual(perimeter.xMin - EPS);
      expect(rect.x + rect.w / 2).toBeLessThanOrEqual(perimeter.xMax + EPS);
      expect(rect.z - rect.d / 2).toBeGreaterThanOrEqual(perimeter.zMin - EPS);
      expect(rect.z + rect.d / 2).toBeLessThanOrEqual(perimeter.zMax + EPS);
    }
  });

  it('building and outdoor items stay on the plate', () => {
    const halfW = kswPlan.plate.w / 2;
    const halfD = kswPlan.plate.d / 2;
    expect(perimeter.xMin).toBeGreaterThanOrEqual(-halfW);
    expect(perimeter.xMax).toBeLessThanOrEqual(halfW);
    expect(perimeter.zMin).toBeGreaterThanOrEqual(-halfD);
    expect(perimeter.zMax).toBeLessThanOrEqual(halfD);
    for (const p of [...kswPlan.outdoorProps, ...kswPlan.outdoorPeople]) {
      expect(Math.abs(p.x)).toBeLessThanOrEqual(halfW - 1);
      expect(Math.abs(p.z)).toBeLessThanOrEqual(halfD - 1);
    }
  });

  it('every door and window fits inside its wall', () => {
    for (const room of kswPlan.rooms) {
      for (const o of [...room.doors, ...room.windows]) {
        const wl = wallLine(room, o.wall);
        expect(
          Math.abs(o.center) + o.width / 2,
          `${room.id} ${o.wall} opening`,
        ).toBeLessThanOrEqual(wl.len / 2 - 0.3);
      }
    }
  });

  it('every room has at least one door, and every door leads to a corridor or outside', () => {
    for (const room of kswPlan.rooms) {
      expect(room.doors.length, room.id).toBeGreaterThanOrEqual(1);
      for (const d of room.doors) {
        const ok = isPerimeterWall(room, d.wall) || doorLeadsToCorridor(room, d.wall, d.center, d.width);
        expect(ok, `${room.id} door on ${d.wall} leads nowhere`).toBe(true);
      }
      const reachable = room.doors.some((d) => doorLeadsToCorridor(room, d.wall, d.center, d.width));
      expect(reachable, `${room.id} is not connected to a corridor`).toBe(true);
    }
  });

  it('windows sit only on perimeter walls (interior walls stay solid)', () => {
    for (const room of kswPlan.rooms) {
      for (const w of room.windows) {
        expect(isPerimeterWall(room, w.wall), `${room.id} window on interior ${w.wall} wall`).toBe(true);
      }
    }
  });

  it('props stay inside their room (clear of walls)', () => {
    for (const room of kswPlan.rooms) {
      for (const p of room.props) {
        expect(inside(room.rect, p.x, p.z, 0.55), `${room.id}: ${p.kind} at ${p.x},${p.z}`).toBe(true);
      }
    }
  });

  it('people stay inside their room', () => {
    for (const room of kswPlan.rooms) {
      for (const p of room.people) {
        expect(inside(room.rect, p.x, p.z, 0.5), `${room.id}: ${p.role} at ${p.x},${p.z}`).toBe(true);
      }
    }
  });

  it('every room is furnished and staffed (a living hospital, not a shell)', () => {
    for (const room of kswPlan.rooms) {
      expect(room.props.length, room.id).toBeGreaterThanOrEqual(3);
      expect(room.people.length, room.id).toBeGreaterThanOrEqual(1);
    }
  });
});
