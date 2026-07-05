// tests/traffic/flowGeomResolution.test.ts
//
// PLACEMENT-CORRECTNESS regression guard for the flow-LOD id-space bug (final
// whole-branch review, Critical finding): FlowState.edge on the wire is a
// traffic-net EDGE id, while all client geometry (TrafficNetGeom.pts/arcLut)
// is keyed by LANE id — two INDEPENDENT 0-based id spaces (18340 edges vs
// 18957 lanes in data/winterthur/trafficnet.json). The original flowLayer
// looked geometry up by `net.pts.get(edgeId)` (id equality), silently drawing
// impostors on unrelated streets net-wide. These tests pin the fix: an edge id
// must resolve through `TrafficNetGeom.edgeToLane` to a polyline that actually
// BELONGS to that edge (lane.edge === edgeId), never by id equality.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { buildLaneNet, type RawLane } from '../../src/diorama/traffic/deadReckon';

describe('edge -> lane geometry resolution (synthetic net, deliberately skewed ids)', () => {
  // Edge 0 owns lane 7; edge 5 owns lanes 3 (index 1) and 9 (index 0). A lane
  // id NEVER equals its edge id here, so any id-equality lookup fails loudly.
  const LANES: RawLane[] = [
    { id: 7, edge: 0, index: 0, lengthM: 100, pts: [[0, 0], [100, 0]] },
    { id: 3, edge: 5, index: 1, lengthM: 50, pts: [[200, 10], [250, 10]] },
    { id: 9, edge: 5, index: 0, lengthM: 50, pts: [[200, 0], [250, 0]] },
  ];

  it('resolves an edge to a lane OF THAT EDGE via lane.edge, not id equality', () => {
    const net = buildLaneNet(LANES);
    expect(net.edgeToLane.get(0)).toBe(7);
    expect(net.pts.get(net.edgeToLane.get(0)!)).toEqual([[0, 0], [100, 0]]);
  });

  it('picks the representative lane (lowest index) regardless of doc order', () => {
    const net = buildLaneNet(LANES); // lane 3 (index 1) precedes lane 9 (index 0)
    expect(net.edgeToLane.get(5)).toBe(9);
  });

  it('has no mapping for an unknown edge id (flowLayer must skip, not draw garbage)', () => {
    const net = buildLaneNet(LANES);
    expect(net.edgeToLane.get(1)).toBeUndefined();
  });
});

describe('edge -> lane geometry resolution (REAL data/winterthur/trafficnet.json)', () => {
  // The committed production net — the same asset the server bakes FlowFrames
  // from. Loading it (~16 MB) once per suite is cheap enough for vitest.
  const doc = JSON.parse(
    readFileSync(resolve(__dirname, '../../data/winterthur/trafficnet.json'), 'utf8'),
  ) as { lanes: RawLane[] };
  const laneById = new Map(doc.lanes.map((l) => [l.id, l]));
  const net = buildLaneNet(doc.lanes);

  it('edge and lane id spaces really are independent (the bug precondition)', () => {
    // If these were equal-sized aligned spaces the old id-equality lookup
    // could have been coincidentally right; assert they are not.
    const edgeIds = new Set(doc.lanes.map((l) => l.edge));
    expect(doc.lanes.length).not.toBe(edgeIds.size);
    // And at least one edge's representative lane has a DIFFERENT id than the
    // edge — id-equality lookup would land on some other street for it.
    const skewed = [...net.edgeToLane].filter(([edgeId, laneId]) => edgeId !== laneId);
    expect(skewed.length).toBeGreaterThan(0);
  });

  it('every sampled edge resolves to a polyline that IS a lane of that edge', () => {
    const edgeIds = [...net.edgeToLane.keys()];
    expect(edgeIds.length).toBeGreaterThan(18000);
    // Deterministic stride sample across the whole id range (~500 edges).
    const stride = Math.max(1, Math.floor(edgeIds.length / 500));
    for (let i = 0; i < edgeIds.length; i += stride) {
      const edgeId = edgeIds[i];
      const laneId = net.edgeToLane.get(edgeId)!;
      const lane = laneById.get(laneId)!;
      expect(lane).toBeDefined();
      // THE regression assertion: the resolved geometry belongs to this edge.
      expect(lane.edge).toBe(edgeId);
      // Representative lane = index 0 (every edge in the baked net has one).
      expect(lane.index).toBe(0);
      // And the polyline the flow layer will draw is exactly that lane's.
      expect(net.pts.get(laneId)).toBe(lane.pts);
    }
  });
});
