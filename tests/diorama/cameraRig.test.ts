import { describe, expect, it } from 'vitest';
import {
  applyDrag,
  applyZoom,
  rigFromLookAt,
  rigPosition,
  roofFade,
  type CameraRigState,
  type RigConfig,
} from '../../src/diorama/ksw/cameraRig';

const cfg: RigConfig = {
  radiusMin: 6,
  radiusMax: 90,
  zoomSpeed: 0.0012,
  dragSpeed: 0.005,
  pitchMin: 0.15,
  pitchMax: 1.2,
  roofFadeNear: 14,
  roofFadeFar: 26,
};

describe('rigFromLookAt / rigPosition roundtrip', () => {
  it('reproduces the source position', () => {
    const s = rigFromLookAt([-9.2, 6.8, 10.8], [0.4, 0.9, -0.5]);
    const p = rigPosition(s);
    expect(p[0]).toBeCloseTo(-9.2, 5);
    expect(p[1]).toBeCloseTo(6.8, 5);
    expect(p[2]).toBeCloseTo(10.8, 5);
  });

  it('stores the target verbatim', () => {
    const s = rigFromLookAt([10, 10, 10], [1, 2, 3]);
    expect(s.target).toEqual([1, 2, 3]);
  });
});

describe('applyZoom', () => {
  const base: CameraRigState = rigFromLookAt([0, 20, 30], [0, 0, 0]);

  it('wheel down (positive deltaY) zooms out — radius grows', () => {
    const out = applyZoom(base, 120, cfg);
    expect(out.radius).toBeGreaterThan(base.radius);
  });

  it('wheel up (negative deltaY) zooms in — radius shrinks', () => {
    const out = applyZoom(base, -120, cfg);
    expect(out.radius).toBeLessThan(base.radius);
  });

  it('clamps to radiusMin and radiusMax', () => {
    let s = base;
    for (let i = 0; i < 200; i++) s = applyZoom(s, -500, cfg);
    expect(s.radius).toBeCloseTo(cfg.radiusMin, 6);
    for (let i = 0; i < 400; i++) s = applyZoom(s, 500, cfg);
    expect(s.radius).toBeCloseTo(cfg.radiusMax, 6);
  });

  it('does not change yaw, pitch, or target', () => {
    const out = applyZoom(base, 120, cfg);
    expect(out.yaw).toBe(base.yaw);
    expect(out.pitch).toBe(base.pitch);
    expect(out.target).toEqual(base.target);
  });
});

describe('applyDrag', () => {
  const base: CameraRigState = rigFromLookAt([0, 20, 30], [0, 0, 0]);

  it('horizontal drag changes yaw only (plus keeps radius)', () => {
    const out = applyDrag(base, 80, 0, cfg);
    expect(out.yaw).not.toBe(base.yaw);
    expect(out.pitch).toBe(base.pitch);
    expect(out.radius).toBe(base.radius);
  });

  it('vertical drag changes pitch within clamps', () => {
    let s = base;
    for (let i = 0; i < 500; i++) s = applyDrag(s, 0, 100, cfg);
    expect(s.pitch).toBeCloseTo(cfg.pitchMax, 6);
    for (let i = 0; i < 1000; i++) s = applyDrag(s, 0, -100, cfg);
    expect(s.pitch).toBeCloseTo(cfg.pitchMin, 6);
  });

  it('yaw is unbounded (no wrap glitch), position stays finite', () => {
    let s = base;
    for (let i = 0; i < 5000; i++) s = applyDrag(s, 50, 0, cfg);
    const p = rigPosition(s);
    expect(p.every((v) => Number.isFinite(v))).toBe(true);
    expect(Math.hypot(p[0] - s.target[0], p[1] - s.target[1], p[2] - s.target[2])).toBeCloseTo(s.radius, 5);
  });
});

describe('roofFade', () => {
  it('is 0 when fully zoomed in (radius <= near)', () => {
    expect(roofFade(cfg.roofFadeNear, cfg)).toBe(0);
    expect(roofFade(6, cfg)).toBe(0);
  });

  it('is 1 when zoomed out (radius >= far)', () => {
    expect(roofFade(cfg.roofFadeFar, cfg)).toBe(1);
    expect(roofFade(90, cfg)).toBe(1);
  });

  it('is smooth and monotonic in between', () => {
    let prev = roofFade(cfg.roofFadeNear, cfg);
    for (let r = cfg.roofFadeNear; r <= cfg.roofFadeFar; r += 0.5) {
      const f = roofFade(r, cfg);
      expect(f).toBeGreaterThanOrEqual(prev);
      expect(f).toBeGreaterThanOrEqual(0);
      expect(f).toBeLessThanOrEqual(1);
      prev = f;
    }
    const mid = roofFade((cfg.roofFadeNear + cfg.roofFadeFar) / 2, cfg);
    expect(mid).toBeCloseTo(0.5, 5);
  });
});
