import { describe, expect, it } from 'vitest';
import {
  makeCorridorGround,
  projectToSegment,
  interpolateProfile,
} from '../../src/diorama/ksw/geo/groundSampler';
import type { RoadPath } from '../../src/diorama/ksw/geo/geoData';

// A straight road along +x, from (0,0) to (100,0), profile stations every
// 10 m rising linearly from 0 to 10 (so profileY(x) === x for x in [0,100]).
function straightRoad(overrides: Partial<RoadPath> = {}): RoadPath {
  const ys = Array.from({ length: 12 }, (_, k) => Math.min(10 * k, 100));
  return {
    class: 'unclassified',
    width: 4,
    pts: [[0, 0], [100, 0]],
    profile: { stepM: 10, ys },
    ...overrides,
  };
}

describe('interpolateProfile', () => {
  it('interpolates linearly between two stations on a synthetic 2-station profile', () => {
    const profile = { stepM: 10, ys: [0, 20] };
    expect(interpolateProfile(profile, 0)).toBeCloseTo(0, 6);
    expect(interpolateProfile(profile, 10)).toBeCloseTo(20, 6);
    expect(interpolateProfile(profile, 5)).toBeCloseTo(10, 6);
    expect(interpolateProfile(profile, 2.5)).toBeCloseTo(5, 6);
  });

  it('clamps arc-length outside the profile range to the end stations', () => {
    const profile = { stepM: 10, ys: [0, 20] };
    expect(interpolateProfile(profile, -5)).toBeCloseTo(0, 6);
    expect(interpolateProfile(profile, 1000)).toBeCloseTo(20, 6);
  });

  it('handles multi-station profiles with exact station hits', () => {
    const profile = { stepM: 10, ys: [0, 10, 20, 30] };
    expect(interpolateProfile(profile, 20)).toBeCloseTo(20, 6);
    expect(interpolateProfile(profile, 25)).toBeCloseTo(25, 6);
  });
});

describe('projectToSegment', () => {
  it('projects a point onto a segment and returns distance + arc position', () => {
    const proj = projectToSegment(50, 3, 0, 0, 100, 0);
    expect(proj.dist).toBeCloseTo(3, 6);
    expect(proj.arc).toBeCloseTo(50, 6);
  });

  it('clamps projection to segment endpoints', () => {
    const proj = projectToSegment(-10, 0, 0, 0, 100, 0);
    expect(proj.arc).toBeCloseTo(0, 6);
    expect(proj.dist).toBeCloseTo(10, 6);
  });
});

describe('makeCorridorGround', () => {
  const tileGround = (x: number, _z: number): number => 1000 + x * 0; // flat tile ground at y=1000

  it('returns the profile height inside the road corridor', () => {
    const road = straightRoad();
    const groundYAt = makeCorridorGround([road], [], tileGround);
    // width 4 -> halfWidth 2 (no traffic net match -> osm width used), well inside at z=0
    const y = groundYAt(50, 0);
    expect(y).toBeCloseTo(50, 1); // profileY(50) === 50
  });

  it('falls back to tileGround well outside any corridor', () => {
    const road = straightRoad();
    const groundYAt = makeCorridorGround([road], [], tileGround);
    const y = groundYAt(50, 50); // far from the road (halfWidth ~2 + blend 3)
    expect(y).toBeCloseTo(1000, 6);
  });

  it('blends smoothly (monotonically) from profile to tileGround across the blend band', () => {
    const road = straightRoad();
    const groundYAt = makeCorridorGround([road], [], tileGround);
    // halfWidth = 4/2 + 1.5 = 3.5 (no traffic net match). Blend band z in [3.5, 6.5].
    const samples = [3.5, 4, 4.5, 5, 5.5, 6, 6.5, 7].map((z) => groundYAt(50, z));
    // Profile at (50, z-projected-onto-road) is ~50 (road is straight so
    // projection arc stays ~50 for all these z offsets since road is along x).
    // Monotonic approach toward tileGround (1000) as z increases.
    for (let i = 1; i < samples.length; i++) {
      expect(samples[i]).toBeGreaterThanOrEqual(samples[i - 1] - 1e-9);
    }
    expect(samples[0]).toBeCloseTo(50, 0);
    expect(samples[samples.length - 1]).toBeCloseTo(1000, 6);
  });

  it('interpolates arc-length position exactly on a synthetic 2-station profile', () => {
    const road: RoadPath = {
      class: 'unclassified',
      width: 2,
      pts: [[0, 0], [10, 0]],
      profile: { stepM: 10, ys: [5, 15] },
    };
    const groundYAt = makeCorridorGround([road], [], tileGround);
    expect(groundYAt(0, 0)).toBeCloseTo(5, 6);
    expect(groundYAt(10, 0)).toBeCloseTo(15, 6);
    expect(groundYAt(5, 0)).toBeCloseTo(10, 6);
  });

  it('uses rail halfWidth = (width + 2.2)/2 + 2 for rails', () => {
    const rail: RoadPath = {
      class: 'rail',
      width: 3.2,
      pts: [[0, 0], [100, 0]],
      profile: { stepM: 10, ys: Array.from({ length: 12 }, (_, k) => Math.min(10 * k, 100)) },
    };
    const groundYAt = makeCorridorGround([], [rail], tileGround);
    // halfWidth = (3.2+2.2)/2 + 2 = 4.7; inside at z=4 should still be profile-dominated
    const yInside = groundYAt(50, 4);
    expect(yInside).toBeCloseTo(50, 0);
    // well outside (z=20) should be tileGround
    const yOutside = groundYAt(50, 20);
    expect(yOutside).toBeCloseTo(1000, 6);
  });

  it('throws at construction listing offenders when a way lacks a profile', () => {
    const noProfileRoad: RoadPath = {
      class: 'unclassified',
      width: 4,
      pts: [[0, 0], [10, 0]],
    };
    expect(() => makeCorridorGround([noProfileRoad], [], tileGround)).toThrow();
    try {
      makeCorridorGround([noProfileRoad], [], tileGround);
      expect.unreachable();
    } catch (e) {
      const msg = (e as Error).message;
      expect(msg).toContain('profile');
    }
  });

  it('is deterministic across repeated construction and queries', () => {
    const road = straightRoad();
    const g1 = makeCorridorGround([road], [], tileGround);
    const g2 = makeCorridorGround([road], [], tileGround);
    for (const [x, z] of [[50, 0], [50, 3], [50, 50], [0, 0], [100, 0]]) {
      expect(g1(x, z)).toBeCloseTo(g2(x, z), 9);
    }
  });

  it('query is well-defined (expected O(1)) across many roads without throwing', () => {
    const roads: RoadPath[] = [];
    for (let i = 0; i < 50; i++) {
      roads.push({
        class: 'unclassified',
        width: 4,
        pts: [[i * 20, 0], [i * 20 + 15, 0]],
        profile: { stepM: 10, ys: [i, i, i] },
      });
    }
    const groundYAt = makeCorridorGround(roads, [], tileGround);
    expect(() => groundYAt(500, 500)).not.toThrow();
    expect(groundYAt(500, 500)).toBeCloseTo(1000, 6);
  });
});
