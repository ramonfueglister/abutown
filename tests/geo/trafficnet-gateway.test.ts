// tests/geo/trafficnet-gateway.test.ts
// Gemeinde-scale bake behavior: buildTrafficNet accepts an optional GeoJSON
// `boundary` (lon/lat Polygon/MultiPolygon). Drivable ways are clipped at the
// boundary BEFORE projection: the inside part is kept and the boundary
// crossing becomes a new terminal vertex whose node is classified
// kind:'gateway'. Fully-outside ways are dropped; in-out-in ways split into
// separate inside segments. After the graph build only the largest connected
// component (by lane length) survives — gateway stubs attached to it stay.
import { describe, expect, it } from 'vitest';
// eslint-disable-next-line import/no-relative-packages
import { buildTrafficNet } from '../../scripts/geo/lib/trafficnet.mjs';
import { makeProjector } from '../../scripts/geo/lib/project.mjs';

const ANCHOR = { lon: 8.72, lat: 47.5 };

// Square boundary around the anchor: lon 8.718..8.722, lat 47.499..47.501
// (~300 m × ~220 m). Closed ring, counter-clockwise; lon/lat pairs.
const SQUARE = {
  type: 'Polygon',
  coordinates: [
    [
      [8.718, 47.499],
      [8.722, 47.499],
      [8.722, 47.501],
      [8.718, 47.501],
      [8.718, 47.499],
    ],
  ],
};

type LonLat = [number, number];
function way(id: number, coords: LonLat[], tags: Record<string, string> = {}) {
  return {
    type: 'way',
    id,
    tags: { highway: 'residential', ...tags },
    geometry: coords.map(([lon, lat]) => ({ lon, lat })),
  };
}

function build(ways: ReturnType<typeof way>[], boundary?: object) {
  return buildTrafficNet({
    osmRoads: { elements: ways },
    osmTrafficNodes: { elements: [] },
    projector: makeProjector(ANCHOR) as unknown as {
      toLocal: (lon: number, lat: number) => [number, number];
    },
    anchor: ANCHOR,
    boundary,
  });
}

describe('trafficnet boundary gateways', () => {
  // 3-way network: A—B, B—C inside; B—D crosses the east boundary edge
  // (lon 8.722) and continues outside to D.
  const A: LonLat = [8.719, 47.5];
  const B: LonLat = [8.72, 47.5];
  const C: LonLat = [8.72, 47.5005];
  const D: LonLat = [8.723, 47.5]; // outside (east of 8.722)
  const threeWays = [way(1, [A, B]), way(2, [B, C]), way(3, [B, D])];

  it('without a boundary behaves as before (no gateways, D kept)', () => {
    const net = build(threeWays);
    expect(net.nodes.some((n) => n.kind === 'gateway')).toBe(false);
    expect(net.meta.gatewayCount).toBe(0);
    // D's node exists: x ≈ projection of lon 8.723 (~226 m east)
    const maxX = Math.max(...net.nodes.map((n) => n.x));
    expect(maxX).toBeGreaterThan(200);
  });

  it('clips the crossing way, creates one gateway node, counts it in meta', () => {
    const net = build(threeWays, SQUARE);
    const gateways = net.nodes.filter((n) => n.kind === 'gateway');
    expect(gateways.length).toBe(1);
    expect(net.meta.gatewayCount).toBe(1);
    // the outside part is gone: no node east of the boundary (lon 8.722
    // projects to ~150.7 m east of the 8.72 anchor)
    const proj = makeProjector(ANCHOR);
    const [boundaryX] = proj.toLocal(8.722, 47.5);
    for (const n of net.nodes) expect(n.x).toBeLessThanOrEqual(boundaryX + 0.5);
    // the gateway sits ON the boundary (within quantization)
    expect(Math.abs(gateways[0].x - boundaryX)).toBeLessThanOrEqual(0.5);
    // gateway is degree-1 (in+out edge of the two-way stub) and has no turns
    const gid = gateways[0].id;
    const touching = net.edges.filter((e) => e.from === gid || e.to === gid);
    const neighbours = new Set(touching.flatMap((e) => [e.from, e.to]));
    neighbours.delete(gid);
    expect(neighbours.size).toBe(1);
    expect(net.turns.some((t) => t.node === gid)).toBe(false);
  });

  it('quantizes all coordinates to 2 decimals (incl. the cut point)', () => {
    const net = build(threeWays, SQUARE);
    const q2 = (v: number) => Math.abs(v * 100 - Math.round(v * 100)) < 1e-6;
    for (const n of net.nodes) {
      expect(q2(n.x)).toBe(true);
      expect(q2(n.z)).toBe(true);
    }
    for (const l of net.lanes) {
      expect(q2(l.lengthM)).toBe(true);
      for (const [x, z] of l.pts) {
        expect(q2(x)).toBe(true);
        expect(q2(z)).toBe(true);
      }
    }
  });

  it('drops fully-outside ways', () => {
    const outside = way(9, [
      [8.724, 47.5],
      [8.725, 47.5],
    ]);
    const net = build([...threeWays, outside], SQUARE);
    expect(net.meta.gatewayCount).toBe(1);
    const proj = makeProjector(ANCHOR);
    const [boundaryX] = proj.toLocal(8.722, 47.5);
    for (const n of net.nodes) expect(n.x).toBeLessThanOrEqual(boundaryX + 0.5);
  });

  it('splits an in-out-in way into two inside segments with two gateways', () => {
    // way leaves through the east edge and comes back further north; both
    // inside parts attach to the inside network so they survive pruning.
    const inOutIn = way(4, [
      [8.72, 47.5], // B, inside (shared with the network)
      [8.723, 47.5], // outside
      [8.723, 47.5005], // outside
      [8.72, 47.5005], // C, inside (shared with the network)
    ]);
    const net = build([way(1, [A, B]), way(2, [B, C]), inOutIn], SQUARE);
    expect(net.meta.gatewayCount).toBe(2);
    expect(net.nodes.filter((n) => n.kind === 'gateway').length).toBe(2);
  });

  it('prunes disconnected fragments, keeping the largest component by lane length', () => {
    // an isolated 2-node fragment inside the boundary, far from the network
    const fragment = way(8, [
      [8.7185, 47.4995],
      [8.7187, 47.4995],
    ]);
    const net = build([...threeWays, fragment], SQUARE);
    // the fragment's nodes (west, ~-113 m) must be gone
    for (const n of net.nodes) expect(n.x).toBeGreaterThan(-100);
    // main network intact: A, B, C, gateway
    expect(net.nodes.length).toBe(4);
    expect(net.meta.gatewayCount).toBe(1);
  });

  it('accepts Feature and MultiPolygon boundaries (with altitude coords)', () => {
    const multi = {
      type: 'Feature',
      geometry: {
        type: 'MultiPolygon',
        coordinates: [[SQUARE.coordinates[0].map(([lon, lat]: number[]) => [lon, lat, 450])]],
      },
    };
    const net = build(threeWays, multi);
    expect(net.meta.gatewayCount).toBe(1);
  });
});
