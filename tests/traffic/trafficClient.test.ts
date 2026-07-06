// tests/traffic/trafficClient.test.ts
//
// Task 9 review carry-forward tests (WS/DOM-free — exercise TrafficClientCore
// and CellGrid directly, per the module's testable-core split):
//
//   (a) CellGrid.cellOfLaneS pins the vertex-keyed per-lane breakpoint
//       classification against hand-computed values on a lane crossing a
//       128 m cell boundary, mirroring backend/crates/winterthur-traffic/
//       src/cells.rs `CellGrid::build` + `cell_of_lane_s` EXACTLY: a
//       breakpoint closes the previous cell's run at the arc position of the
//       vertex where the cell changes; the final run extends to +Infinity.
//
//   (b) stale-vehicle eviction: TrafficClientCore.updateCamera evicts vehicles
//       whose canonical cell (via the SAME CellGrid.cellOfLaneS path the
//       keyframe ghost-heal uses) falls outside the 5×5 hysteresis band,
//       while vehicles still in-band survive.

import { describe, expect, it } from 'vitest';
import { CellGrid, TrafficClientCore, CELL_SIZE_M } from '../../src/diorama/traffic/trafficClient';
import { buildLaneNet, type RawLane } from '../../src/diorama/traffic/deadReckon';

// A straight lane along +x crossing two 128 m cell boundaries:
// v0=(0,0) cell col0 -> v1=(100,0) cell col0 (no change, acc=100) ->
// v2=(150,0) cell col1 (change detected AT this vertex: closes col0's run at
// the CURRENT accumulated arc length, acc=150, not at v1's acc) -> v3=(300,0)
// cell col2 (change detected at this vertex: closes col1's run at acc=300;
// final run col2 extends to +Infinity).
// This is cells.rs's exact rule: `acc` is advanced BEFORE the cell-change
// check on each vertex, so the breakpoint's s_end is the arc length AT the
// vertex that entered the new cell, not the arc length at the last vertex of
// the old cell. Hand-computed by walking cells.rs's loop by hand (see module
// banner above).
const CROSSING_LANE: RawLane = {
  id: 42,
  edge: 42,
  index: 0,
  lengthM: 300,
  pts: [[0, 0], [100, 0], [150, 0], [300, 0]],
};

describe('CellGrid.cellOfLaneS — vertex-keyed breakpoints (cells.rs port)', () => {
  const grid = CellGrid.build([CROSSING_LANE]);

  it('builds a 3-col x 1-row grid over the lane bbox (0..300 x 128 m cells)', () => {
    expect(grid.cols).toBe(3);
    expect(grid.rows).toBe(1);
  });

  it('resolves s inside the first run (before the s=150 breakpoint) to cell 0', () => {
    expect(grid.cellOfLaneS(42, 0)).toBe(0);
    expect(grid.cellOfLaneS(42, 50)).toBe(0);
    expect(grid.cellOfLaneS(42, 100)).toBe(0);
    expect(grid.cellOfLaneS(42, 150)).toBe(0); // exact hit on sEnd closes THIS run
  });

  it('resolves s in the second run (150, 300] to cell 1', () => {
    expect(grid.cellOfLaneS(42, 150.001)).toBe(1);
    expect(grid.cellOfLaneS(42, 200)).toBe(1);
    expect(grid.cellOfLaneS(42, 300)).toBe(1); // exact hit on sEnd closes THIS run
  });

  it('resolves s past the second breakpoint (300, +inf) to the terminal cell 2', () => {
    expect(grid.cellOfLaneS(42, 300.001)).toBe(2);
  });

  it('resolves s past the declared lane length to the terminal cell (final run is +Infinity)', () => {
    expect(grid.cellOfLaneS(42, 10_000)).toBe(2);
  });

  it('returns -1 for an unknown lane id', () => {
    expect(grid.cellOfLaneS(999, 0)).toBe(-1);
  });

  it('agrees with cellOf(x, z) on the interpolated position at each hand-computed sample', () => {
    // Cross-check: classifying by (x, z) directly (cellOf) must match
    // classifying by (lane, s) via the breakpoints (cellOfLaneS) at the
    // sampled vertices themselves, where both approaches are exact.
    expect(grid.cellOfLaneS(42, 0)).toBe(grid.cellOf(0, 0));
    expect(grid.cellOfLaneS(42, 100)).toBe(grid.cellOf(100, 0));
    expect(grid.cellOfLaneS(42, 10_000)).toBe(grid.cellOf(300, 0));
  });
});

describe('TrafficClientCore.updateCamera — stale-vehicle eviction', () => {
  // Two lanes far apart so their cells never overlap: lane 1 sits at the
  // origin (cells around col 0), lane 2 sits 1000 m east (well outside any
  // 5x5 band centred near the origin).
  const NEAR_LANE: RawLane = { id: 1, edge: 1, index: 0, lengthM: 50, pts: [[0, 0], [50, 0]] };
  const FAR_LANE: RawLane = { id: 2, edge: 2, index: 0, lengthM: 50, pts: [[1000, 1000], [1050, 1000]] };

  function makeCore(): TrafficClientCore {
    const net = buildLaneNet([NEAR_LANE, FAR_LANE]);
    return new TrafficClientCore([NEAR_LANE, FAR_LANE], net);
  }

  it('evicts vehicles whose cell falls outside the 5x5 band, keeps in-band vehicles', () => {
    const core = makeCore();
    // Seed directly (bypassing the wire) — near-lane vehicle should survive,
    // far-lane vehicle should be evicted once the camera centres on the origin.
    core.vehicles.set(101, { lane: NEAR_LANE.id, s: 10, v: 0, tickAt: 0, cls: 0 });
    core.vehicles.set(202, { lane: FAR_LANE.id, s: 10, v: 0, tickAt: 0, cls: 0 });

    const { subscribe } = core.updateCamera(0, 0);

    expect(core.vehicles.has(101)).toBe(true);
    expect(core.vehicles.has(202)).toBe(false);
    // Sanity: the 3x3 subscribe set around the origin includes cell 0.
    expect(subscribe.length).toBeGreaterThan(0);
  });

  it('keeps a vehicle inside the 5x5 hysteresis band even if outside the 3x3 subscribe set', () => {
    const core = makeCore();
    // Place the "camera" so cell (0,0) is within the 5x5 keep-band radius but
    // outside the 3x3 want-band — e.g. camera at (2*CELL_SIZE_M, 0): the
    // vehicle's cell (col 0) is 2 cells away (within radius 2, outside radius 1).
    core.vehicles.set(303, { lane: NEAR_LANE.id, s: 10, v: 0, tickAt: 0, cls: 0 });
    core.updateCamera(2 * CELL_SIZE_M, 0);
    expect(core.vehicles.has(303)).toBe(true);
  });

  it('evicts once the camera moves far enough that the vehicle leaves the 5x5 band entirely', () => {
    const core = makeCore();
    core.vehicles.set(303, { lane: NEAR_LANE.id, s: 10, v: 0, tickAt: 0, cls: 0 });
    // First settle near the vehicle so it isn't evicted immediately.
    core.updateCamera(0, 0);
    expect(core.vehicles.has(303)).toBe(true);
    // Now pan far away — outside any 5x5 band around the near lane.
    core.updateCamera(1000, 1000);
    expect(core.vehicles.has(303)).toBe(false);
  });
});

describe('TrafficClientCore.applyFrame — keyframe ghost-heal uses the canonical cell path', () => {
  const NEAR_LANE: RawLane = { id: 1, edge: 1, index: 0, lengthM: 50, pts: [[0, 0], [50, 0]] };

  it('evicts a stale vehicle resolved (via cellOfLaneS) to the keyframed cell but absent from it', () => {
    const net = buildLaneNet([NEAR_LANE]);
    const core = new TrafficClientCore([NEAR_LANE], net);
    core.vehicles.set(7, { lane: NEAR_LANE.id, s: 5, v: 0, tickAt: 0, cls: 0 });

    const cell = core.grid.cellOfLaneS(NEAR_LANE.id, 5);
    // Keyframe for that same cell lists a different vehicle only.
    core.applyFrame({
      cell,
      tick: 10,
      keyframe: true,
      vehicles: [{ id: 9, lane: NEAR_LANE.id, sQ: 100, vQ: 0, class: 0 }],
      departed: [],
    });

    expect(core.vehicles.has(7)).toBe(false); // ghost healed
    expect(core.vehicles.has(9)).toBe(true);
  });
});
