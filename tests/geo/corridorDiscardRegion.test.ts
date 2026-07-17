import { describe, expect, it } from 'vitest';
import {
  isInsideDiscardRegion,
  updateCorridorDiscardAnchor,
} from '../../src/diorama/ksw/geo/corridorDiscardRegion';

// The corridor discard (#144) punches the terrain out inside road/rail
// corridors, and the road skirts are the walls that close that hole. Both
// shaders read the region below; if they ever disagree, one of two artefacts
// appears: a discarded hole with no wall to close it (see-through corridor),
// or a wall with no hole to close (the black rectangle/line chains along the
// far ring — skirts standing exposed on the coarse L0 backdrop, which deviates
// up to ~20 m from the fine heights the platform was draped onto).
//
// isInsideDiscardRegion is the pure JS twin of the TSL predicate. MIRROR
// (load-bearing): keep it in lockstep with insideDiscardRegion/
// outsideDiscardRegion in the same module.
describe('corridorDiscardRegion', () => {
  it('is inside within the radius of the anchor and outside beyond it', () => {
    updateCorridorDiscardAnchor(100, -200, 720);

    expect(isInsideDiscardRegion(100, -200)).toBe(true); // at the anchor
    expect(isInsideDiscardRegion(100 + 719, -200)).toBe(true); // just inside
    expect(isInsideDiscardRegion(100 + 721, -200)).toBe(false); // just outside
    expect(isInsideDiscardRegion(100, -200 + 5000)).toBe(false); // far field
  });

  it('measures radius in the xz plane only, ignoring height', () => {
    updateCorridorDiscardAnchor(0, 0, 100);

    // The anchor is a 2-D (x, z) point: a corridor 90 m away is inside no
    // matter how far the terrain rises or falls there.
    expect(isInsideDiscardRegion(90, 0)).toBe(true);
    expect(isInsideDiscardRegion(0, 90)).toBe(true);
    expect(isInsideDiscardRegion(64, 64)).toBe(true); // hypot ≈ 90.5 < 100
    expect(isInsideDiscardRegion(71, 71)).toBe(false); // hypot ≈ 100.4 > 100
  });

  it('follows the anchor as the camera moves', () => {
    updateCorridorDiscardAnchor(0, 0, 500);
    expect(isInsideDiscardRegion(2000, 0)).toBe(false);

    updateCorridorDiscardAnchor(2000, 0, 500);
    expect(isInsideDiscardRegion(2000, 0)).toBe(true);
    expect(isInsideDiscardRegion(0, 0)).toBe(false);
  });

  it('excludes the boundary exactly, so skirt and terrain never both claim it', () => {
    updateCorridorDiscardAnchor(0, 0, 720);

    // The terrain discards where inside is true; the skirt discards where it is
    // false. Exactly one of them owns every point — including the boundary,
    // which must resolve to "outside" in BOTH shaders (`< radius` / `>= radius`).
    expect(isInsideDiscardRegion(720, 0)).toBe(false);
  });
});
