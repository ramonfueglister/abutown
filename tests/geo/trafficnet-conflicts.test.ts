// tests/geo/trafficnet-conflicts.test.ts
//
// FIX-C1: distance-based turn-conflict detection in the bake transform
// (scripts/geo/lib/trafficnet.mjs). The pure-intersection test missed
// cross-stream near-collisions — two turn chords that pass within a couple of
// metres of the shared node without a proper crossing, so the kernel let both
// streams through at once (33 near-collision pairs at ~90° yaw in the
// investigation). These tests pin the new rule:
//   * two chords that pass within CONFLICT_CLEARANCE_M without crossing DO
//     conflict (the root-cause case);
//   * near-parallel chords that only graze (opposing/aligned throughs) do NOT
//     conflict (no false serialization of a two-way through movement);
//   * chords that properly cross still conflict (regression);
//   * at a real 4-way node, parallel multilane same-movement turns do NOT
//     conflict, while turns merging onto the same toLane DO.
import { describe, expect, it } from 'vitest';
import { chordsConflict, CONFLICT_CLEARANCE_M, buildTrafficNet } from '../../scripts/geo/lib/trafficnet.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

type Chord = [[number, number], [number, number]];

describe('chordsConflict — distance-based near-miss detection', () => {
  it('flags two chords that pass within CONFLICT_CLEARANCE_M without crossing', () => {
    // Two near-perpendicular chords that miss each other by ~1.5 m: chord A runs
    // along +x at z=0; chord B runs along +z at x=+2 (offset so the segments do
    // NOT proper-cross — B starts past A's right end region — yet their closest
    // approach is ~1.5 m, inside the 2.5 m clearance).
    const A: Chord = [
      [-12, 0],
      [12, 0],
    ];
    // Vertical chord at x = 13.5, from z=-12..12 — its nearest point to A is
    // (13.5, 0) vs A's end (12, 0): distance 1.5 m; the segments never cross.
    const B: Chord = [
      [13.5, -12],
      [13.5, 12],
    ];
    // sanity: they must NOT properly cross (A ends at x=12, B is at x=13.5)
    expect(Math.abs(13.5 - 12)).toBeLessThan(CONFLICT_CLEARANCE_M);
    expect(chordsConflict(A, B)).toBe(true);
  });

  it('does NOT flag near-parallel chords that only graze (opposing throughs)', () => {
    // Two anti-parallel collinear chords (opposing straight-through movements on
    // the same road): closest distance ≈ 0 but they are parallel → NOT a
    // conflict (they run on their own offset lanes in reality).
    const A: Chord = [
      [-12, 0],
      [12, 0],
    ];
    const B: Chord = [
      [12, 0],
      [-12, 0],
    ];
    expect(chordsConflict(A, B)).toBe(false);
  });

  it('does NOT flag two far-apart perpendicular chords (clear of the clearance)', () => {
    const A: Chord = [
      [-12, 0],
      [12, 0],
    ];
    const B: Chord = [
      [20, -12],
      [20, 12],
    ]; // 8 m clear of A's end
    expect(chordsConflict(A, B)).toBe(false);
  });

  it('flags properly-crossing chords (regression on the original intersection rule)', () => {
    const A: Chord = [
      [-12, 0],
      [12, 0],
    ];
    const B: Chord = [
      [0, -12],
      [0, 12],
    ];
    expect(chordsConflict(A, B)).toBe(true);
  });
});

describe('buildTrafficNet — conflict emission at a 4-way cross', () => {
  // A synthetic +-junction: an E-W two-way residential road and a N-S two-way
  // residential road meeting at a shared vertex, placed by inverting the local
  // projector so we control the exact geometry.
  const rad = Math.PI / 180;
  const R = 6371008.8;
  const cos0 = Math.cos(ANCHOR.lat * rad);
  const at = (x: number, z: number) => ({
    lon: x / (rad * R * cos0) + ANCHOR.lon,
    lat: -z / (rad * R) + ANCHOR.lat,
  });
  const way = (id: number, pts: [number, number][], tags: Record<string, string>) => ({
    type: 'way',
    id,
    geometry: pts.map(([x, z]) => at(x, z)),
    tags,
  });

  const osmRoads = {
    elements: [
      way(1, [
        [-100, 0],
        [0, 0],
        [100, 0],
      ], { highway: 'residential' }),
      way(2, [
        [0, -100],
        [0, 0],
        [0, 100],
      ], { highway: 'residential' }),
    ],
  };
  const net = buildTrafficNet({
    osmRoads,
    osmTrafficNodes: { elements: [] },
    projector: makeProjector(ANCHOR) as unknown as { toLocal: (lon: number, lat: number) => [number, number] },
    anchor: ANCHOR,
  });
  const laneEdge = new Map<number, number>(net.lanes.map((l: { id: number; edge: number }) => [l.id, l.edge]));
  const center = net.nodes.find((n: { x: number; z: number }) => Math.hypot(n.x, n.z) < 1)!;
  const group = net.turns.filter((t: { node: number }) => t.node === center.id);

  it('emits at least one crossing conflict at the 4-way node', () => {
    const anyConflict = group.some((t: { conflictsWith: number[] }) => t.conflictsWith.length > 0);
    expect(anyConflict).toBe(true);
  });

  it('turns sharing the same fromLane edge (divergent) never conflict with each other', () => {
    for (const a of group) {
      for (const b of group) {
        if (a.id === b.id) continue;
        if (laneEdge.get(a.fromLane) === laneEdge.get(b.fromLane)) {
          expect(a.conflictsWith).not.toContain(b.id);
        }
      }
    }
  });

  it('turns that merge onto the same toLane ARE mutual conflicts', () => {
    let checked = 0;
    for (const a of group) {
      for (const b of group) {
        if (a.id === b.id) continue;
        if (a.toLane === b.toLane && laneEdge.get(a.fromLane) !== laneEdge.get(b.fromLane)) {
          expect(a.conflictsWith).toContain(b.id);
          checked++;
        }
      }
    }
    // The 4-way cross has merge pairs (two approaches turning onto the same
    // outbound lane); assert we actually exercised the branch.
    expect(checked).toBeGreaterThan(0);
  });
});
