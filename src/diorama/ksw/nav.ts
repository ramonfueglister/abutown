// Navigation over the KSW floor plan: rooms connect through their corridor
// doors, corridors form a ladder graph (two east-west spines + two end
// connectors), and the plaza reaches the corridors through the reception
// portal. Pure data + functions — fully unit-testable, no three.js.

import type { FloorPlan, Room, WallSide } from './floorPlan';

export type Pt = [number, number];
export type NavPoint = { point: Pt; roomId: string | null };

type DoorAnchor = { inside: Pt; door: Pt; anchor: Pt };

export type NavGraph = {
  plan: FloorPlan;
  roomDoors: Record<string, DoorAnchor[]>; // corridor-connected doors per room
  junctions: Pt[]; // 4 ladder corners
  lanes: { a: number; b: number; xMin: number; xMax: number; zMin: number; zMax: number };
  portal: { outdoor: Pt; chain: Pt[] }; // plaza -> reception -> corridor B anchor
};

const EPS = 1e-6;

function doorWorld(room: Room, wall: WallSide, center: number): { pos: Pt; out: Pt } {
  const r = room.rect;
  switch (wall) {
    case 'n':
      return { pos: [r.x + center, r.z - r.d / 2], out: [0, -1] };
    case 's':
      return { pos: [r.x + center, r.z + r.d / 2], out: [0, 1] };
    case 'w':
      return { pos: [r.x - r.w / 2, r.z + center], out: [-1, 0] };
    case 'e':
      return { pos: [r.x + r.w / 2, r.z + center], out: [1, 0] };
  }
}

export function buildNav(plan: FloorPlan): NavGraph {
  const horizontals = plan.corridors.filter((c) => c.w >= c.d).sort((c1, c2) => c1.z - c2.z);
  const verticals = plan.corridors.filter((c) => c.w < c.d).sort((c1, c2) => c1.x - c2.x);
  const laneA = horizontals[0].z; // north spine centerline z
  const laneB = horizontals[horizontals.length - 1].z; // south spine centerline z
  const laneW = verticals[0].x;
  const laneE = verticals[verticals.length - 1].x;
  const junctions: Pt[] = [
    [laneW, laneA],
    [laneE, laneA],
    [laneW, laneB],
    [laneE, laneB],
  ];

  const roomDoors: Record<string, DoorAnchor[]> = {};
  for (const room of plan.rooms) {
    const anchors: DoorAnchor[] = [];
    for (const d of room.doors) {
      const { pos, out } = doorWorld(room, d.wall, d.center);
      // corridor adjacency: the door line must touch a corridor rect
      const corridor = plan.corridors.find((c) => {
        const cx = pos[0] + out[0] * 0.2;
        const cz = pos[1] + out[1] * 0.2;
        return Math.abs(cx - c.x) <= c.w / 2 + EPS && Math.abs(cz - c.z) <= c.d / 2 + EPS;
      });
      if (!corridor) continue;
      const inside: Pt = [pos[0] - out[0] * 1.4, pos[1] - out[1] * 1.4];
      // anchor: project the door onto the corridor centerline
      const anchor: Pt =
        corridor.w >= corridor.d ? [pos[0], corridor.z] : [corridor.x, pos[1]];
      anchors.push({ inside, door: pos, anchor });
    }
    roomDoors[room.id] = anchors;
  }

  // Reception portal: plaza-side door of the entrance hall down to corridor B.
  // Waypoints skirt the reception desk (lane x = empfang.x + 2.2).
  const empfang = plan.rooms.find((r) => r.id === 'empfang');
  const chain: Pt[] = [];
  let outdoor: Pt = [0, plan.building.z + plan.building.d / 2 + 1.6];
  if (empfang) {
    const south = empfang.doors.find((d) => d.wall === 's');
    const north = empfang.doors.find((d) => d.wall === 'n');
    if (south && north) {
      const sPos = doorWorld(empfang, 's', south.center).pos;
      const nPos = doorWorld(empfang, 'n', north.center).pos;
      const lane = empfang.rect.x + 2.2;
      outdoor = [sPos[0], sPos[1] + 1.6];
      chain.push(
        [sPos[0], sPos[1]],
        [sPos[0], sPos[1] - 1.2],
        [lane, empfang.rect.z + 1.2],
        [lane, empfang.rect.z - 3.4],
        [nPos[0], nPos[1]],
        [nPos[0], laneB],
      );
    }
  }

  return {
    plan,
    roomDoors,
    junctions,
    lanes: { a: laneA, b: laneB, xMin: laneW, xMax: laneE, zMin: laneA, zMax: laneB },
    portal: { outdoor, chain },
  };
}

type Line = 'A' | 'B' | 'W' | 'E';

function laneOf(nav: NavGraph, p: Pt): Line | null {
  const { a, b, xMin, xMax } = nav.lanes;
  if (Math.abs(p[1] - a) < 0.01 && p[0] >= xMin - 0.01 && p[0] <= xMax + 0.01) return 'A';
  if (Math.abs(p[1] - b) < 0.01 && p[0] >= xMin - 0.01 && p[0] <= xMax + 0.01) return 'B';
  if (Math.abs(p[0] - xMin) < 0.01 && p[1] >= a - 0.01 && p[1] <= b + 0.01) return 'W';
  if (Math.abs(p[0] - xMax) < 0.01 && p[1] >= a - 0.01 && p[1] <= b + 0.01) return 'E';
  return null;
}

const laneEnds: Record<Line, [number, number]> = { A: [0, 1], B: [2, 3], W: [0, 2], E: [1, 3] };

const dist = (p: Pt, q: Pt): number => Math.hypot(p[0] - q[0], p[1] - q[1]);

// Shortest corridor walk between two on-lane points via the 4 ladder corners.
function corridorRoute(nav: NavGraph, from: Pt, to: Pt): Pt[] {
  const lf = laneOf(nav, from);
  const lt = laneOf(nav, to);
  if (!lf || !lt) return [from, to];
  if (lf === lt) return [from, to];

  // Dijkstra over 6 nodes: 4 junctions + from + to
  const nodes: Pt[] = [...nav.junctions, from, to];
  const edges: Array<[number, number]> = [
    [0, 1], // A
    [2, 3], // B
    [0, 2], // W
    [1, 3], // E
  ];
  for (const [idx, lane] of [
    [4, lf],
    [5, lt],
  ] as Array<[number, Line]>) {
    for (const j of laneEnds[lane]) edges.push([idx, j]);
  }
  const n = nodes.length;
  const distArr = new Array<number>(n).fill(Infinity);
  const prev = new Array<number>(n).fill(-1);
  const visited = new Array<boolean>(n).fill(false);
  distArr[4] = 0;
  for (let iter = 0; iter < n; iter++) {
    let u = -1;
    for (let i = 0; i < n; i++) if (!visited[i] && (u === -1 || distArr[i] < distArr[u])) u = i;
    if (u === -1 || distArr[u] === Infinity) break;
    visited[u] = true;
    for (const [x, y] of edges) {
      const v = x === u ? y : y === u ? x : -1;
      if (v === -1) continue;
      const w = dist(nodes[x], nodes[y]);
      if (distArr[u] + w < distArr[v]) {
        distArr[v] = distArr[u] + w;
        prev[v] = u;
      }
    }
  }
  const chain: Pt[] = [];
  let cur = 5;
  while (cur !== -1) {
    chain.push(nodes[cur]);
    cur = prev[cur];
  }
  chain.reverse();
  return chain;
}

function dedupe(path: Pt[]): Pt[] {
  const out: Pt[] = [];
  for (const p of path) {
    const last = out[out.length - 1];
    if (!last || dist(last, p) > 0.01) out.push(p);
  }
  return out;
}

// Expand an endpoint to (corridor anchor, connecting polyline from the raw
// point to that anchor). For rooms, tries every corridor door.
function anchorsFor(nav: NavGraph, np: NavPoint): Array<{ anchor: Pt; prefix: Pt[] }> {
  if (np.roomId) {
    const doors = nav.roomDoors[np.roomId] ?? [];
    return doors.map((d) => ({ anchor: d.anchor, prefix: [np.point, d.inside, d.door, d.anchor] }));
  }
  const [x, z] = np.point;
  const inBuilding =
    Math.abs(x - nav.plan.building.x) < nav.plan.building.w / 2 &&
    Math.abs(z - nav.plan.building.z) < nav.plan.building.d / 2;
  if (inBuilding) {
    // corridor point: drop onto the nearest lane
    const { a, b, xMin, xMax } = nav.lanes;
    const cands: Pt[] = [
      [Math.min(Math.max(x, xMin), xMax), a],
      [Math.min(Math.max(x, xMin), xMax), b],
      [xMin, Math.min(Math.max(z, a), b)],
      [xMax, Math.min(Math.max(z, a), b)],
    ];
    cands.sort((p, q) => dist(np.point, p) - dist(np.point, q));
    return [{ anchor: cands[0], prefix: [np.point, cands[0]] }];
  }
  // outdoor: through the reception portal
  const chain = nav.portal.chain;
  const anchor = chain[chain.length - 1] ?? np.point;
  return [{ anchor, prefix: [np.point, nav.portal.outdoor, ...chain] }];
}

export function routePath(nav: NavGraph, from: NavPoint, to: NavPoint): Pt[] {
  const outdoor = (np: NavPoint): boolean =>
    !np.roomId &&
    !(
      Math.abs(np.point[0] - nav.plan.building.x) < nav.plan.building.w / 2 &&
      Math.abs(np.point[1] - nav.plan.building.z) < nav.plan.building.d / 2
    );
  // open plaza: straight stroll
  if (outdoor(from) && outdoor(to)) return dedupe([from.point, to.point]);

  const froms = anchorsFor(nav, from);
  const tos = anchorsFor(nav, to);
  let best: Pt[] | null = null;
  let bestLen = Infinity;
  for (const f of froms) {
    for (const t of tos) {
      const mid = corridorRoute(nav, f.anchor, t.anchor);
      const path = dedupe([...f.prefix, ...mid, ...[...t.prefix].reverse()]);
      let len = 0;
      for (let i = 0; i < path.length - 1; i++) len += dist(path[i], path[i + 1]);
      if (len < bestLen) {
        bestLen = len;
        best = path;
      }
    }
  }
  return best ?? [from.point, to.point];
}

export function pathLength(path: Pt[]): number {
  let len = 0;
  for (let i = 0; i < path.length - 1; i++) len += dist(path[i], path[i + 1]);
  return len;
}

// ───────────────────────────────────────────────────────────────────────────
// Multi-zone navigation (T17, S3b-2), additive. buildNav above stays untouched
// (it encodes the single-ladder assumption the pre-S3 tests rely on). The
// zone-ladder interior generated by generatePlan.ts has one ladder PER zone
// plus cross-corridors between adjacent zones, so a room-and-corridor
// reachability graph is the right model here rather than the fixed 4-corner
// ladder.
//
// Graph model:
//   • Every corridor rect is a node. Two corridor nodes are connected when
//     their rects overlap or touch (the zone spines + the connector corridors
//     that bridge adjacent zones).
//   • Every room is a node, connected to each corridor its door opens onto (the
//     door's outward point lands inside the corridor rect).
//   • The main door connects to whatever corridor is nearest its inward point.
// BFS from the main door then reaches every room in a well-formed plan.

type ZoneRect = { x: number; z: number; w: number; d: number };

// how far past a door to probe for its corridor (a touch beyond the wall face)
const CW_PROBE = 0.6;

export type ZonedNavGraph = {
  plan: FloorPlan;
  // adjacency by node key: `c<i>` corridor nodes, `r:<roomId>` room nodes.
  adj: Map<string, Set<string>>;
  entry: string; // corridor node the main door opens onto
};

function rectsTouch(a: ZoneRect, b: ZoneRect, tol = 0.05): boolean {
  return (
    a.x - a.w / 2 <= b.x + b.w / 2 + tol &&
    b.x - b.w / 2 <= a.x + a.w / 2 + tol &&
    a.z - a.d / 2 <= b.z + b.d / 2 + tol &&
    b.z - b.d / 2 <= a.z + a.d / 2 + tol
  );
}

function pointInRect(x: number, z: number, r: ZoneRect, tol = 0.05): boolean {
  return Math.abs(x - r.x) <= r.w / 2 + tol && Math.abs(z - r.z) <= r.d / 2 + tol;
}

export function buildZonedNav(plan: FloorPlan, mainDoor: { x: number; z: number }): ZonedNavGraph {
  const adj = new Map<string, Set<string>>();
  const link = (a: string, b: string): void => {
    if (!adj.has(a)) adj.set(a, new Set());
    if (!adj.has(b)) adj.set(b, new Set());
    adj.get(a)!.add(b);
    adj.get(b)!.add(a);
  };

  const corridors = plan.corridors;
  // corridor↔corridor edges: any two overlapping/touching corridor rects
  for (let i = 0; i < corridors.length; i++) {
    adj.set(`c${i}`, adj.get(`c${i}`) ?? new Set());
    for (let j = i + 1; j < corridors.length; j++) {
      if (rectsTouch(corridors[i], corridors[j])) link(`c${i}`, `c${j}`);
    }
  }

  // room↔corridor edges: the door's outward point must land in a corridor
  for (const room of plan.rooms) {
    const rk = `r:${room.id}`;
    adj.set(rk, adj.get(rk) ?? new Set());
    for (const d of room.doors) {
      const { pos, out } = doorWorld(room, d.wall, d.center);
      const px = pos[0] + out[0] * (CW_PROBE);
      const pz = pos[1] + out[1] * (CW_PROBE);
      for (let i = 0; i < corridors.length; i++) {
        if (pointInRect(px, pz, corridors[i])) link(rk, `c${i}`);
      }
    }
  }

  // entry: the corridor nearest the main door's inward projection (or its
  // position if it already sits in one).
  let entry = 'c0';
  let bestD = Infinity;
  for (let i = 0; i < corridors.length; i++) {
    const c = corridors[i];
    const dx = Math.max(Math.abs(mainDoor.x - c.x) - c.w / 2, 0);
    const dz = Math.max(Math.abs(mainDoor.z - c.z) - c.d / 2, 0);
    const dd = Math.hypot(dx, dz);
    if (dd < bestD) {
      bestD = dd;
      entry = `c${i}`;
    }
  }

  return { plan, adj, entry };
}

// BFS from the main-door entry corridor; returns the set of reachable room ids.
export function zonedReachableRooms(nav: ZonedNavGraph): Set<string> {
  const seen = new Set<string>();
  const rooms = new Set<string>();
  const queue: string[] = [nav.entry];
  seen.add(nav.entry);
  while (queue.length > 0) {
    const cur = queue.shift()!;
    if (cur.startsWith('r:')) rooms.add(cur.slice(2));
    for (const nb of nav.adj.get(cur) ?? []) {
      if (!seen.has(nb)) {
        seen.add(nb);
        queue.push(nb);
      }
    }
  }
  return rooms;
}
