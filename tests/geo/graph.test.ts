import { describe, expect, it } from 'vitest';
import { buildRoadGraph } from '../../scripts/geo/lib/graph.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

const dLon = 0.000013; // ≈ 1 m Ost am Anker
const pt = (i: number) => ({ lon: ANCHOR.lon + i * dLon * 10, lat: ANCHOR.lat });
const osmRoads = { elements: [
  { type: 'way', id: 100, tags: { highway: 'residential', maxspeed: '30', lanes: '2' },
    nodes: [1, 2, 3], geometry: [pt(0), pt(1), pt(2)] },
  { type: 'way', id: 101, tags: { highway: 'service', oneway: 'yes' },
    nodes: [2, 4], geometry: [pt(1), { lon: ANCHOR.lon + dLon * 10, lat: ANCHOR.lat + 0.0001 }] },
  { type: 'node', id: 2, tags: { highway: 'traffic_signals' },
    lat: pt(1).lat, lon: pt(1).lon },
  { type: 'relation', id: 900, tags: { type: 'restriction', restriction: 'no_left_turn' },
    members: [
      { type: 'way', ref: 100, role: 'from' },
      { type: 'node', ref: 2, role: 'via' },
      { type: 'way', ref: 101, role: 'to' },
    ] },
] };

const dem = { heightAt: () => 450 };

describe('buildRoadGraph', () => {
  const g = buildRoadGraph({ osmRoads, projector: makeProjector(ANCHOR), dem });
  it('splits way 100 at shared node 2 → 3 edges total', () => {
    expect(g.edgeA.length).toBe(3);
  });
  it('marks node 2 as signal and node ids survive', () => {
    const i2 = g.nodeOsmId.findIndex((id) => id === 2n);
    expect(g.nodeSignal[i2]).toBe(true);
  });
  it('carries attributes: maxspeed 30, oneway on the service edge', () => {
    expect(g.edgeMaxspeed).toContain(30);
    expect(g.edgeOneway).toContain(1);
  });
  it('resolves the turn restriction to edge indices via node 2', () => {
    expect(g.restrictionFromEdge.length).toBe(1);
    const via = g.restrictionViaNode[0];
    expect(g.nodeOsmId[via]).toBe(2n);
  });
  it('drapes elevation onto every polyline point', () => {
    expect(g.edgePtY.every((y) => y === 450)).toBe(true);
  });
});
