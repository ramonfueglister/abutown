// tests/geo/gradewidths-parity.test.ts
//
// Parity test: scripts/geo/lib/gradewidths.mjs (the plain-Node bake-side
// twin) must produce EXACTLY the same output as the TS runtime module
// src/diorama/traffic/corridorWidths.ts on the real Winterthur data. Any
// semantic drift between the two ports must fail here.
import { describe, expect, it } from 'vitest';
import { laneFloorWidths } from '../../scripts/geo/lib/gradewidths.mjs';
import { corridorWidths } from '../../src/diorama/traffic/corridorWidths';
import trafficNet from '../../data/winterthur/trafficnet.json';
import roadsJson from '../../data/winterthur/roads.json';

it('mjs twin matches the TS implementation on the real net (sampled)', () => {
  const roads = (roadsJson as { roads: { class: string; width: number; pts: number[][] }[] }).roads
    .filter((_, i) => i % 7 === 0); // ~300 roads, keeps the test fast
  const ts = corridorWidths(roads, trafficNet as never);
  const mjs = laneFloorWidths(roads, trafficNet as never);
  expect(mjs).toEqual(ts);
});
