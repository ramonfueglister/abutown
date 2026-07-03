import { describe, expect, it } from 'vitest';
import { accessPoints } from '../../scripts/geo/lib/access.mjs';

// Eine Kante von (0,0) nach (100,0), Klasse residential(4)
const graph = {
  edgeA: [0], edgeB: [1], edgeClass: [4],
  edgePtOffset: [0], edgePtX: [0, 100], edgePtZ: [0, 0], edgePtY: [450, 450],
};

describe('accessPoints', () => {
  it('binds a building at (40,10) to offset ~40 on the edge', () => {
    const [ap] = accessPoints({ graph, footprints: [[[38, 8], [42, 8], [42, 12], [38, 12]]] });
    expect(ap.edge).toBe(0);
    expect(ap.offsetM).toBeCloseTo(40, 0);
  });

  it('returns sentinel when nothing within 80 m', () => {
    const [ap] = accessPoints({ graph, footprints: [[[0, 500], [1, 500], [1, 501]]] });
    expect(ap.edge).toBe(0xffffffff);
  });

  it('prefers a farther drivable edge over a closer footway', () => {
    // Edge 0: footway (class 8) along z=0, x=0..20 — only 5 m from the building centroid.
    // Edge 1: drivable road (class 4) along z=30, x=0..100 — 30 m from the building centroid.
    const mixedGraph = {
      edgeA: [0, 2],
      edgeB: [1, 3],
      edgeClass: [8, 4],
      edgePtOffset: [0, 2],
      edgePtX: [0, 20, 0, 100],
      edgePtZ: [0, 0, 30, 30],
      edgePtY: [450, 450, 450, 450],
    };
    // Building centroid at (10, 5): distance to footway (z=0) = 5 m, distance to road (z=30) = 25 m.
    const building = [[8, 3], [12, 3], [12, 7], [8, 7]];
    const [ap] = accessPoints({ graph: mixedGraph, footprints: [building] });
    expect(ap.edge).toBe(1); // the drivable edge (index 1), not the closer footway (index 0)
    expect(mixedGraph.edgeClass[ap.edge]).toBeLessThanOrEqual(6);
  });
});
