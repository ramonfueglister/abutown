// tests/geo/gradewidths-parity.test.ts
//
// Parity test: scripts/geo/lib/gradewidths.mjs (the plain-Node bake-side
// twin) must produce EXACTLY the same output as the TS runtime module
// src/diorama/traffic/corridorWidths.ts on the real Winterthur data. Any
// semantic drift between the two ports must fail here.
import { describe, expect, it } from 'vitest';
import { laneFloorWidths } from '../../scripts/geo/lib/gradewidths.mjs';
import { corridorWidths } from '../../src/diorama/traffic/corridorWidths';
import {
  roadCorridorHalfWidth,
  railCorridorHalfWidth,
} from '../../src/diorama/ksw/geo/groundSampler';
import trafficNet from '../../data/winterthur/trafficnet.json';
import roadsJson from '../../data/winterthur/roads.json';

type Way = { class: string; width: number; pts: number[][] };

it('mjs twin matches the TS implementation on the real net (sampled)', () => {
  const roads = (roadsJson as { roads: Way[] }).roads
    .filter((_, i) => i % 7 === 0); // ~300 roads, keeps the test fast
  const ts = corridorWidths(roads, trafficNet as never);
  const mjs = laneFloorWidths(roads, trafficNet as never);
  expect(mjs).toEqual(ts);
});

// Finding 4: the RUNTIME corridor half-width (groundSampler) and the BAKE
// corridor half-width (bake-world.mjs's `ways`) must agree on the real data for
// EVERY way, or the discard mask / corridor-snap / draping sampler silently
// disagree at corridor edges. Both are the render ribbon half-width + shoulder
// (roads: max(OSM,lane-floor)/2 + 1.5; rails: (w+2.2)/2 + 2). We recompute the
// bake formula inline (bake-world is not importable — it runs the whole pipeline
// on import) and assert equality against the runtime helpers.
describe('corridor half-width parity (runtime groundSampler ↔ bake ways)', () => {
  const doc = roadsJson as { roads: Way[]; rails?: Way[] };
  const roads = doc.roads;
  const rails = doc.rails ?? [];

  it('road corridor half-widths agree for every road', () => {
    const corrected = corridorWidths(roads, trafficNet as never);
    const floors = laneFloorWidths(roads, trafficNet as never); // bake width source
    let checked = 0;
    for (let i = 0; i < roads.length; i++) {
      // runtime uses corridorWidths; bake uses laneFloorWidths — the first
      // parity block proves those are equal, so the two halfWidths must match.
      const runtime = roadCorridorHalfWidth(roads[i].width, corrected[i]);
      const bake = Math.max(roads[i].width, floors[i]) / 2 + 1.5; // bake-world.mjs ways[roads]
      expect(runtime).toBeCloseTo(bake, 9);
      checked++;
    }
    expect(checked).toBe(roads.length);
  });

  it('rail corridor half-widths agree for every rail', () => {
    let checked = 0;
    for (let i = 0; i < rails.length; i++) {
      const runtime = railCorridorHalfWidth(rails[i].width);
      const bake = (rails[i].width + 2.2) / 2 + 2.0; // bake-world.mjs ways[rails]
      expect(runtime).toBeCloseTo(bake, 9);
      checked++;
    }
    expect(checked).toBe(rails.length);
  });
});
