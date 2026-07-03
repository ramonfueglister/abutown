// tests/geo/trafficnet.test.ts
// Validates the baked lane-level traffic network (data/winterthur/trafficnet.json)
// — the single source of truth for both the Rust server and the browser client.
// The bake is offline (scripts/geo/bake-traffic-net.mjs); these assertions guard
// the invariants everything downstream relies on: id integrity, turn coverage,
// lane geometry, Swiss right-hand offset, signal phase completeness, and
// roundabout yielding.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

type Vec = [number, number];
interface Node {
  id: number;
  x: number;
  z: number;
  kind: 'signal' | 'roundabout' | 'priority' | 'uncontrolled' | 'dead_end';
  signal: { cycleS: number; phases: { greenS: number; turns: number[] }[] } | null;
}
interface Edge {
  id: number;
  from: number;
  to: number;
  speedMs: number;
  laneCount: number;
  priorityRoad: boolean;
  lanes: number[];
}
interface Lane {
  id: number;
  edge: number;
  index: number;
  lengthM: number;
  pts: Vec[];
}
interface Turn {
  id: number;
  fromLane: number;
  toLane: number;
  node: number;
  conflictsWith: number[];
  yieldsTo: number[];
}
interface TrafficNet {
  meta: { anchor: { lon: number; lat: number }; laneWidth: number; cellSize: number };
  nodes: Node[];
  edges: Edge[];
  lanes: Lane[];
  turns: Turn[];
}

const net: TrafficNet = JSON.parse(
  readFileSync(new URL('../../data/winterthur/trafficnet.json', import.meta.url), 'utf8'),
);

const nodeById = new Map(net.nodes.map((n) => [n.id, n]));
const edgeById = new Map(net.edges.map((e) => [e.id, e]));
const laneById = new Map(net.lanes.map((l) => [l.id, l]));

function polylineLength(pts: Vec[]): number {
  let d = 0;
  for (let i = 1; i < pts.length; i++) {
    d += Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
  }
  return d;
}

describe('trafficnet baked asset', () => {
  it('(a) has more than 100 edges', () => {
    expect(net.edges.length).toBeGreaterThan(100);
  });

  it('(b) every lane/edge/turn id reference resolves', () => {
    for (const l of net.lanes) expect(edgeById.has(l.edge)).toBe(true);
    for (const e of net.edges) {
      expect(nodeById.has(e.from)).toBe(true);
      expect(nodeById.has(e.to)).toBe(true);
      for (const lid of e.lanes) expect(laneById.has(lid)).toBe(true);
    }
    for (const t of net.turns) {
      expect(laneById.has(t.fromLane)).toBe(true);
      expect(laneById.has(t.toLane)).toBe(true);
      expect(nodeById.has(t.node)).toBe(true);
      for (const c of t.conflictsWith) expect(net.turns.some((x) => x.id === c)).toBe(true);
      for (const y of t.yieldsTo) expect(net.turns.some((x) => x.id === y)).toBe(true);
    }
  });

  it('(c) every non-dead_end node with ≥1 in and ≥1 out edge has ≥1 turn', () => {
    const inCount = new Map<number, number>();
    const outCount = new Map<number, number>();
    for (const e of net.edges) {
      outCount.set(e.from, (outCount.get(e.from) ?? 0) + 1);
      inCount.set(e.to, (inCount.get(e.to) ?? 0) + 1);
    }
    const turnNodes = new Set(net.turns.map((t) => t.node));
    for (const n of net.nodes) {
      if (n.kind === 'dead_end') continue;
      if ((inCount.get(n.id) ?? 0) >= 1 && (outCount.get(n.id) ?? 0) >= 1) {
        expect(turnNodes.has(n.id)).toBe(true);
      }
    }
  });

  it('(d) every lane lengthM ≈ its polyline length (±1%)', () => {
    for (const l of net.lanes) {
      const len = polylineLength(l.pts);
      expect(Math.abs(l.lengthM - len)).toBeLessThanOrEqual(Math.max(len * 0.01, 0.05));
    }
  });

  it('(e) right-hand rule: ≥95% of two-way pairs have lane 0 to the right of travel', () => {
    // Two-way edge pairs share the same node endpoints reversed. For each edge,
    // lane 0 must lie to the RIGHT of the edge's travel direction (Switzerland
    // drives on the right). The ground plane is +x=EAST, +z=SOUTH — a
    // left-handed (screen y-down) frame, so "driver's right" is the +90°
    // (visually clockwise) rotation of the heading: rotate dir=(dx,dz) → right
    // normal (-dz, dx). Facing east dir=(1,0) → right=(0,1)=SOUTH ✓. A point on
    // the right therefore has a positive component along that normal, i.e.
    // cross(dir, off) = dir.x·off.z − dir.z·off.x > 0. We test lane 0's midpoint
    // offset from the straight from→to node centreline.
    // A two-way edge and its reverse share the same underlying centreline, so
    // their lane-0 polylines must run antiparallel and be laterally separated,
    // with each one on the RIGHT of its own travel direction. We test that by
    // comparing the two lane-0 polylines directly: at matching arc positions,
    // lane0(A) minus lane0(reverse-B) (walked in reverse) is a vector that must
    // point to A's right (cross(dirA, sep) > 0). This is robust to curvature —
    // it uses the local heading, not the straight node chord.
    const pairKey = (a: number, b: number) => `${a}->${b}`;
    const edgeByEndpoints = new Map(net.edges.map((e) => [pairKey(e.from, e.to), e]));
    let pairs = 0;
    let correct = 0;
    const seen = new Set<number>();
    for (const e of net.edges) {
      const rev = edgeByEndpoints.get(pairKey(e.to, e.from));
      if (!rev) continue; // one-way — skip
      if (seen.has(e.id) || seen.has(rev.id)) continue;
      seen.add(e.id);
      seen.add(rev.id);
      pairs++;
      const la = laneById.get(e.lanes[0])!;
      const lb = laneById.get(rev.lanes[0])!; // runs opposite direction
      // sample lane A at its interior points, find the local heading, and the
      // separation to the geometrically nearest point on lane B.
      const bPts = lb.pts;
      let votes = 0;
      let right = 0;
      for (let i = 1; i < la.pts.length; i++) {
        const a0 = la.pts[i - 1];
        const a1 = la.pts[i];
        const dir: Vec = [a1[0] - a0[0], a1[1] - a0[1]];
        const mid: Vec = [(a0[0] + a1[0]) / 2, (a0[1] + a1[1]) / 2];
        // nearest point on B (B is the reverse, so its points are near A's)
        let best = Infinity;
        let bp: Vec = bPts[0];
        for (const p of bPts) {
          const d = (p[0] - mid[0]) ** 2 + (p[1] - mid[1]) ** 2;
          if (d < best) {
            best = d;
            bp = p;
          }
        }
        // separation vector from B (left lane, oncoming) to A (our lane). Our
        // lane must be to the right of our heading → cross(dir, sep) > 0.
        const sep: Vec = [mid[0] - bp[0], mid[1] - bp[1]];
        if (Math.hypot(sep[0], sep[1]) < 0.1) continue; // coincident — skip
        const cross = dir[0] * sep[1] - dir[1] * sep[0];
        votes++;
        if (cross > 0) right++;
      }
      // the pair counts as correct if the majority of its samples put lane 0 on
      // the right (a few curvature/endpoint samples can flip near tight bends).
      if (votes > 0 && right / votes >= 0.5) correct++;
    }
    expect(pairs).toBeGreaterThan(0);
    expect(correct / pairs).toBeGreaterThanOrEqual(0.95);
  });

  it('(f) signal nodes: phases cover every incoming turn id exactly once per cycle', () => {
    const signalNodes = net.nodes.filter((n) => n.kind === 'signal');
    expect(signalNodes.length).toBeGreaterThan(0);
    for (const n of signalNodes) {
      expect(n.signal).not.toBeNull();
      const incoming = net.turns.filter((t) => t.node === n.id).map((t) => t.id);
      const gated: number[] = [];
      for (const ph of n.signal!.phases) gated.push(...ph.turns);
      // exactly once: same set, no duplicates
      expect(gated.slice().sort((a, b) => a - b)).toEqual(incoming.slice().sort((a, b) => a - b));
      expect(new Set(gated).size).toBe(gated.length);
    }
  });

  it('(g) roundabouts: if any exist, their entry turns list non-empty yieldsTo', () => {
    // Winterthur's bbox may contain no junction=roundabout way; the bake still
    // implements roundabout modelling. This is conditional per the brief: if a
    // roundabout node exists, its entry turns must yield to the circulating ring.
    const roundaboutNodes = net.nodes.filter((n) => n.kind === 'roundabout');
    if (roundaboutNodes.length === 0) return; // none on the plate — see report
    const rbIds = new Set(roundaboutNodes.map((n) => n.id));
    const entryTurns = net.turns.filter((t) => rbIds.has(t.node));
    expect(entryTurns.length).toBeGreaterThan(0);
    // at least one turn at a roundabout node yields (the entries)
    expect(entryTurns.some((t) => t.yieldsTo.length > 0)).toBe(true);
  });
});
