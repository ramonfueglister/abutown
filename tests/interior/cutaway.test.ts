// TDD for cutawayState (T18, S3c): the pure zoom→(cutH, upperFade) mapping that
// drives the dollhouse cutaway. Tokens: kswS3.{fadeStartR:90, fadeEndR:55,
// cutHeight:3.2}. Contract (plan §Task 18):
//   • radius ≥ fadeStartR → OFF: cutH = 1e6 (no slice), upperFade = 1 (closed)
//   • radius ≤ fadeEndR   → ON:  cutH = 3.2,  upperFade = 0 (fully open)
//   • between: upperFade interpolates linearly; cutH jumps to 3.2 once
//     upperFade < 0.15, else stays 1e6.
import { describe, expect, it } from 'vitest';
import { cutawayState } from '../../src/diorama/ksw/interior/cutaway';
import { kswS3 } from '../../src/diorama/designTokens';

const { fadeStartR, fadeEndR, cutHeight } = kswS3;

describe('cutawayState', () => {
  it('is fully closed (off) at and above fadeStartR', () => {
    for (const r of [fadeStartR, fadeStartR + 1, 200, 1500]) {
      const s = cutawayState(r);
      expect(s.upperFade).toBe(1);
      expect(s.cutH).toBe(1e6);
    }
  });

  it('is fully open (on) at and below fadeEndR', () => {
    for (const r of [fadeEndR, fadeEndR - 1, 20, 0]) {
      const s = cutawayState(r);
      expect(s.upperFade).toBe(0);
      expect(s.cutH).toBe(cutHeight);
    }
  });

  it('interpolates upperFade linearly across the fade window', () => {
    const mid = (fadeStartR + fadeEndR) / 2;
    const s = cutawayState(mid);
    // linear: at the midpoint fade should be ~0.5
    expect(s.upperFade).toBeGreaterThan(0.45);
    expect(s.upperFade).toBeLessThan(0.55);
  });

  it('keeps cutH off until upperFade drops below 0.15, then slices at cutHeight', () => {
    // just inside the open end where fade is small but > 0: cutH must be sliced
    const nearOpen = fadeEndR + (fadeStartR - fadeEndR) * 0.1; // fade ~0.1
    const sOpen = cutawayState(nearOpen);
    expect(sOpen.upperFade).toBeLessThan(0.15);
    expect(sOpen.cutH).toBe(cutHeight);
    // near the closed end where fade is large: no slice
    const nearClosed = fadeEndR + (fadeStartR - fadeEndR) * 0.9; // fade ~0.9
    const sClosed = cutawayState(nearClosed);
    expect(sClosed.upperFade).toBeGreaterThan(0.15);
    expect(sClosed.cutH).toBe(1e6);
  });

  it('is monotonic: upperFade never decreases as radius grows', () => {
    let prev = -1;
    for (let r = fadeEndR - 5; r <= fadeStartR + 5; r += 1) {
      const f = cutawayState(r).upperFade;
      expect(f).toBeGreaterThanOrEqual(prev - 1e-9);
      prev = f;
    }
  });

  it('is deterministic (same radius → same state)', () => {
    for (const r of [30, 55, 72.5, 90, 400]) {
      expect(cutawayState(r)).toEqual(cutawayState(r));
    }
  });

  it('clamps upperFade to [0,1]', () => {
    for (let r = 0; r <= 300; r += 3) {
      const f = cutawayState(r).upperFade;
      expect(f).toBeGreaterThanOrEqual(0);
      expect(f).toBeLessThanOrEqual(1);
    }
  });
});
