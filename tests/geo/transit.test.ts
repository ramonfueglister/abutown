import { describe, expect, it } from 'vitest';
import { transformTransit } from '../../scripts/geo/lib/transit.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

const projector = makeProjector(ANCHOR);

// One edge from (0,0) to (100,0), class 4 (drivable/residential).
const graph = {
  edgeA: [0], edgeB: [1], edgeClass: [4],
  edgePtOffset: [0], edgePtX: [0, 100], edgePtZ: [0, 0], edgePtY: [450, 450],
};

const relation = {
  type: 'relation',
  tags: { route: 'bus', ref: '10', name: 'Bus 10' },
  members: [
    { type: 'node', role: 'platform', lon: ANCHOR.lon, lat: ANCHOR.lat, tags: { name: 'Stop A' } },
    { type: 'node', role: 'platform', lon: ANCHOR.lon + 0.0006, lat: ANCHOR.lat, tags: { name: 'Stop B' } },
  ],
};

describe('transformTransit', () => {
  it('groups a bus route relation into 1 line with 2 stops bound to the edge', () => {
    const out = transformTransit({ osmTransit: { elements: [relation] }, graph, projector });
    expect(out.lineRef).toEqual(['10']);
    expect(out.lineMode).toEqual([0]);
    expect(out.lineStopOffset).toEqual([0]);
    expect(out.stopEdge).toHaveLength(2);
    for (const e of out.stopEdge) expect(e).toBe(0);
    for (const off of out.stopOffsetM) expect(off).toBeGreaterThanOrEqual(0);
    expect(out.stopName).toEqual(['Stop A', 'Stop B']);
  });

  it('skips relations with an unknown route mode', () => {
    const weird = { type: 'relation', tags: { route: 'ferry', ref: 'F1' }, members: [] };
    const out = transformTransit({ osmTransit: { elements: [weird] }, graph, projector });
    expect(out.lineRef).toEqual([]);
  });
});
