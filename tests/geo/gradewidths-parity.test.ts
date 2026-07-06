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
  roadMaskHalfWidth,
  railMaskHalfWidth,
  MASK_CELL_M,
  MASK_RASTER_MARGIN_M,
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

// Platform wave: the runtime platform OUTER half-width (roads.ts's apron + skirt
// edge) must equal the bake mask footprint PLUS the raster-quantization margin,
// or the discarded fringe between the nominal mask edge and its discretised
// extent shows as a see-through void. The bake stamps `renderHalfWidthM` into
// buildCorridorMask, which floors each way's stamping radius at MASK_CELL_M
// (corridormask.mjs `Math.max(halfWidthM, cellSize)`) — nominal mask footprint =
// max(renderHW, MASK_CELL_M); the platform reaches that + MASK_RASTER_MARGIN_M.
describe('platform outer half-width parity (runtime apron/skirt ↔ bake mask + raster margin)', () => {
  const doc = roadsJson as { roads: Way[]; rails?: Way[] };
  const roads = doc.roads;
  const rails = doc.rails ?? [];

  it('MASK_CELL_M mirrors the bake mask cell size (2.5 m)', () => {
    expect(MASK_CELL_M).toBe(2.5); // bake-world.mjs MASK_CELL_M
  });

  it('raster margin = cellSize·√2/2 (the nearest-cell quantization tolerance)', () => {
    expect(MASK_RASTER_MARGIN_M).toBeCloseTo((2.5 * Math.SQRT2) / 2, 9);
  });

  it('road platform half-widths = max(renderHW, MASK_CELL_M) + margin for every road', () => {
    const corrected = corridorWidths(roads, trafficNet as never);
    const floors = laneFloorWidths(roads, trafficNet as never); // bake width source
    let narrow = 0; // ways floored to the cell (renderHW < 2.5 m)
    for (let i = 0; i < roads.length; i++) {
      const runtime = roadMaskHalfWidth(roads[i].width, corrected[i]);
      // bake mask footprint = max(OSM/laneFloor render HW, MASK_CELL_M); platform
      // reaches that + the raster margin.
      const renderHW = Math.max(roads[i].width, floors[i]) / 2;
      const platformHW = Math.max(renderHW, MASK_CELL_M) + MASK_RASTER_MARGIN_M;
      expect(runtime).toBeCloseTo(platformHW, 9);
      if (renderHW < MASK_CELL_M) narrow++;
    }
    // Non-vacuity: the report measured 54.5 % of ways below the cell floor — the
    // apron exists precisely because many ways are narrower than a mask cell.
    expect(narrow).toBeGreaterThan(0);
  });

  it('rail platform half-widths = max(renderHW, MASK_CELL_M) + margin for every rail', () => {
    for (let i = 0; i < rails.length; i++) {
      const runtime = railMaskHalfWidth(rails[i].width);
      const renderHW = (rails[i].width + 2.2) / 2;
      const platformHW = Math.max(renderHW, MASK_CELL_M) + MASK_RASTER_MARGIN_M;
      expect(runtime).toBeCloseTo(platformHW, 9);
    }
  });
});
