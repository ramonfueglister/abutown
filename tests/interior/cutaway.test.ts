// TDD for peelState (Phase A): the pure orbit-radius → storey-peel mapping.
// Contract (plan Task 1): L peel units; unit 0 fades roof out + top interior
// in; unit j (j≥1) dissolves the shell band of storey L−j while its interior
// fades out and the storey below fades in. EG never fades out.
import { describe, expect, it } from 'vitest';
import { peelState, closedPeel, storeyLayout, type PeelCfg } from '../../src/diorama/ksw/interior/cutaway';
import { kswPeel } from '../../src/diorama/designTokens';

const cfg: PeelCfg = { storeyCount: 4, storeyH: 3.5, baseY: 0, startR: kswPeel.startR, endR: kswPeel.endR };
const rAt = (p: number): number => cfg.startR - (p / cfg.storeyCount) * (cfg.startR - cfg.endR);

describe('peelState', () => {
  it('is fully closed at and above startR', () => {
    for (const r of [cfg.startR, cfg.startR + 1, 500, 1500]) {
      const s = peelState(r, cfg);
      expect(s.p).toBe(0);
      expect(s.roofFade).toBe(1);
      expect(s.discardAbove).toBe(1e6);
      expect(s.bandFade).toBe(0);
      expect(s.storeyFades).toEqual([0, 0, 0, 0]);
    }
  });

  it('closedPeel equals the closed state', () => {
    expect(closedPeel(cfg)).toEqual(peelState(cfg.startR + 100, cfg));
  });

  it('unit 0: roof fades out while ONLY the top storey interior fades in', () => {
    const s = peelState(rAt(0.5), cfg);
    expect(s.p).toBeCloseTo(0.5, 5);
    expect(s.roofFade).toBeCloseTo(0.5, 5);
    expect(s.discardAbove).toBe(1e6); // no wall slicing during the roof unit
    expect(s.storeyFades[3]).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[0]).toBe(0);
    expect(s.storeyFades[1]).toBe(0);
    expect(s.storeyFades[2]).toBe(0);
  });

  it('unit j dissolves the band of storey L−j with coordinated interior swap', () => {
    // p = 1.5 → unit 1, frac 0.5: storey 3 (top) shell band half-dissolved,
    // interior 3 half-out (1−0.5), interior 2 half-in (0.5).
    const s = peelState(rAt(1.5), cfg);
    expect(s.roofFade).toBe(0);
    expect(s.bandLo).toBeCloseTo(0 + 3 * 3.5, 5);       // baseY+(L−j)·H, j=1
    expect(s.discardAbove).toBeCloseTo(0 + 4 * 3.5, 5); // baseY+(L−j+1)·H
    expect(s.bandFade).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[3]).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[2]).toBeCloseTo(0.5, 5);
    expect(s.storeyFades[1]).toBe(0);
    expect(s.storeyFades[0]).toBe(0);
  });

  it('fully open at endR: only EG interior at 1, shell cut above storey 1', () => {
    const s = peelState(cfg.endR, cfg);
    expect(s.p).toBeCloseTo(4, 5);
    expect(s.storeyFades).toEqual([1, 0, 0, 0]);
    expect(s.bandFade).toBeCloseTo(1, 5);
    expect(s.bandLo).toBeCloseTo(3.5, 5);       // band of storey 1 fully gone
    expect(s.discardAbove).toBeCloseTo(7, 5);   // = baseY + 2·H
  });

  it('every storeyFade stays in [0,1] and EG is monotonic non-decreasing', () => {
    let prevEg = -1;
    for (let r = cfg.endR - 5; r <= cfg.startR + 5; r += 0.5) {
      const s = peelState(r, cfg);
      for (const f of s.storeyFades) {
        expect(f).toBeGreaterThanOrEqual(0);
        expect(f).toBeLessThanOrEqual(1);
      }
      expect(s.p).toBeGreaterThanOrEqual(0);
      expect(s.p).toBeLessThanOrEqual(cfg.storeyCount);
    }
    for (let r = cfg.startR + 5; r >= cfg.endR - 5; r -= 0.5) {
      const eg = peelState(r, cfg).storeyFades[0];
      expect(eg).toBeGreaterThanOrEqual(prevEg - 1e-9);
      prevEg = eg;
    }
  });

  it('single-storey building: roof fade IS the whole peel, never any wall cut', () => {
    const c1: PeelCfg = { ...cfg, storeyCount: 1 };
    for (const p of [0, 0.3, 0.7, 1]) {
      const r = c1.startR - p * (c1.startR - c1.endR);
      const s = peelState(r, c1);
      expect(s.roofFade).toBeCloseTo(1 - p, 5);
      expect(s.storeyFades).toHaveLength(1);
      expect(s.storeyFades[0]).toBeCloseTo(p, 5);
      expect(s.discardAbove).toBe(1e6);
    }
  });

  it('is deterministic', () => {
    for (const r of [45, 60, 77.5, 110, 400]) expect(peelState(r, cfg)).toEqual(peelState(r, cfg));
  });
});

describe('storeyLayout', () => {
  it('derives count from eave height at the nominal pitch, clamped', () => {
    expect(storeyLayout(3.0)).toEqual({ storeyCount: 1, storeyH: 3.0 });
    expect(storeyLayout(14)).toEqual({ storeyCount: 4, storeyH: 3.5 });
    expect(storeyLayout(17)).toEqual({ storeyCount: 5, storeyH: 3.4 });
  });
  it('clamps storeyH into [minStoreyH, maxStoreyH] via the count', () => {
    const tall = storeyLayout(100); // would be 29 storeys at nominal → capped
    expect(tall.storeyCount).toBeLessThanOrEqual(12);
    const low = storeyLayout(2.0); // below minStoreyH → still 1 storey
    expect(low.storeyCount).toBe(1);
    expect(low.storeyH).toBe(2.0);
  });
  it('enforces the max pitch by adding storeys (5 m eave → 2×2.5 m, not 1×5 m)', () => {
    expect(storeyLayout(5)).toEqual({ storeyCount: 2, storeyH: 2.5 });
  });
  it('best effort when bounds are unsatisfiable: the count bound wins', () => {
    const s = storeyLayout(100); // 12 storeys max → 8.33 m pitch, outside maxStoreyH
    expect(s.storeyCount).toBe(12);
    expect(s.storeyH).toBeCloseTo(100 / 12, 6);
  });
});
