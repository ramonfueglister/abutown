import { describe, expect, it } from 'vitest';
import { kswCamera } from '../../src/diorama/designTokens';
import {
  applyDrag,
  applyPan,
  applyZoom,
  edgePanVelocity,
  keyboardPanVelocity,
  rigFromLookAt,
  rigPosition,
  roofFade,
  type CameraRigState,
  type PanKeys,
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
  panMarginPx: 40,
  panSpeed: 26,
  panBoundsX: 34,
  panBoundsZ: 26,
  keyRotateSpeed: 1.2,
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

describe('kswCamera contract (navigierbarer Zoom)', () => {
  it('roofs are gone well before close-up: fade completes at radius >= 55', () => {
    expect(kswCamera.roofFadeNear).toBeGreaterThanOrEqual(55);
  });
  it('a screen-filling hospital view (radius 55) is fully roofless, not translucent', () => {
    expect(roofFade(55, kswCamera)).toBe(0);
  });
  it('roofs are fully on at the overview framing (radius ~111)', () => {
    expect(roofFade(111, kswCamera)).toBe(1);
  });
  it('interior presets stay roofless', () => {
    expect(roofFade(14, kswCamera)).toBe(0);
  });
  it('allows zooming far out', () => {
    expect(kswCamera.radiusMax).toBeGreaterThanOrEqual(300);
  });
});

describe('edgePanVelocity (Age-of-Empires edge scrolling)', () => {
  const W = 1280;
  const H = 800;

  it('is zero away from the edges', () => {
    expect(edgePanVelocity(W / 2, H / 2, W, H, 0, cfg)).toEqual([0, 0]);
    expect(edgePanVelocity(100, 100, W, H, 0, cfg)).toEqual([0, 0]);
  });

  it('right edge pans screen-right (world +x at yaw 0)', () => {
    const [dx, dz] = edgePanVelocity(W - 1, H / 2, W, H, 0, cfg);
    expect(dx).toBeGreaterThan(0);
    expect(Math.abs(dz)).toBeLessThan(1e-9);
  });

  it('top edge pans screen-up (world -z at yaw 0)', () => {
    const [dx, dz] = edgePanVelocity(W / 2, 1, W, H, 0, cfg);
    expect(dz).toBeLessThan(0);
    expect(Math.abs(dx)).toBeLessThan(1e-9);
  });

  it('pan direction rotates with the camera yaw', () => {
    // yaw 90°: screen-right becomes world -z
    const [dx, dz] = edgePanVelocity(W - 1, H / 2, W, H, Math.PI / 2, cfg);
    expect(Math.abs(dx)).toBeLessThan(1e-6);
    expect(dz).toBeLessThan(0);
  });

  it('ramps up toward the edge', () => {
    const mid = edgePanVelocity(W - cfg.panMarginPx / 2, H / 2, W, H, 0, cfg)[0];
    const edge = edgePanVelocity(W - 1, H / 2, W, H, 0, cfg)[0];
    expect(edge).toBeGreaterThan(mid);
    expect(mid).toBeGreaterThan(0);
  });

  it('corners pan diagonally', () => {
    const [dx, dz] = edgePanVelocity(W - 1, 1, W, H, 0, cfg);
    expect(dx).toBeGreaterThan(0);
    expect(dz).toBeLessThan(0);
  });
});

describe('applyPan', () => {
  it('moves the target and keeps its height', () => {
    const s = rigFromLookAt([0, 20, 30], [0, 0.6, 0]);
    const out = applyPan(s, 5, -3, 1, cfg);
    expect(out.target[0]).toBeCloseTo(5);
    expect(out.target[2]).toBeCloseTo(-3);
    expect(out.target[1]).toBe(0.6);
    expect(out.radius).toBe(s.radius);
    expect(out.yaw).toBe(s.yaw);
  });

  it('scales with dt', () => {
    const s = rigFromLookAt([0, 20, 30], [0, 0.6, 0]);
    const out = applyPan(s, 10, 0, 0.5, cfg);
    expect(out.target[0]).toBeCloseTo(5);
  });

  it('clamps the target to the pan bounds', () => {
    let s = rigFromLookAt([0, 20, 30], [0, 0.6, 0]);
    for (let i = 0; i < 100; i++) s = applyPan(s, 50, 50, 1, cfg);
    expect(s.target[0]).toBe(cfg.panBoundsX);
    expect(s.target[2]).toBe(cfg.panBoundsZ);
    for (let i = 0; i < 100; i++) s = applyPan(s, -50, -50, 1, cfg);
    expect(s.target[0]).toBe(-cfg.panBoundsX);
    expect(s.target[2]).toBe(-cfg.panBoundsZ);
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

describe('keyboardPanVelocity (WASD/arrow keyboard pan)', () => {
  const none: PanKeys = { up: false, down: false, left: false, right: false };

  it('is zero when nothing is held', () => {
    expect(keyboardPanVelocity(none, 0, cfg)).toEqual([0, 0]);
  });

  it('is zero when opposing keys cancel (up+down)', () => {
    expect(keyboardPanVelocity({ ...none, up: true, down: true }, 0, cfg)).toEqual([0, 0]);
  });

  it('W (up) pans screen-up = world -z at yaw 0', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, up: true }, 0, cfg);
    expect(dz).toBeLessThan(0);
    expect(Math.abs(dx)).toBeLessThan(1e-9);
  });

  it('D (right) pans world +x at yaw 0', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, right: true }, 0, cfg);
    expect(dx).toBeGreaterThan(0);
    expect(Math.abs(dz)).toBeLessThan(1e-9);
  });

  it('pan direction rotates with the camera yaw (90 deg: screen-right -> world -z)', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, right: true }, Math.PI / 2, cfg);
    expect(Math.abs(dx)).toBeLessThan(1e-6);
    expect(dz).toBeLessThan(0);
  });

  it('diagonal (up+right) has both components non-zero', () => {
    const [dx, dz] = keyboardPanVelocity({ ...none, up: true, right: true }, 0, cfg);
    expect(dx).toBeGreaterThan(0);
    expect(dz).toBeLessThan(0);
  });
});
