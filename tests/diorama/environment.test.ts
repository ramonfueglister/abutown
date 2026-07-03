import { describe, expect, it } from 'vitest';
import { envAnchors, envKeyframes, weatherLook } from '../../src/diorama/designTokens';
import { computeEnvironment, lerpColorHex, moonPhaseLightDir } from '../../src/diorama/environment/environment';
import { CLEAR_SKY } from '../../src/diorama/environment/weather';
import { moonState, sunState } from '../../src/diorama/environment/solar';

describe('envKeyframes', () => {
  it('has all four keyframes with lamp on at night, off at day', () => {
    expect(envKeyframes.night.lampOn01).toBe(1);
    expect(envKeyframes.day.lampOn01).toBe(0);
    expect(envKeyframes.goldenEvening.lampOn01).toBe(1); // DREDGE dusk keeps its warm windows
  });
  it('day is brighter and less dramatic than golden', () => {
    expect(envKeyframes.day.exposure).toBeGreaterThan(envKeyframes.goldenEvening.exposure);
    expect(envKeyframes.day.godraysMix).toBeLessThan(envKeyframes.goldenMorning.godraysMix);
    expect(envKeyframes.day.saturation).toBeLessThan(envKeyframes.goldenEvening.saturation);
  });
  it('anchors are ordered night < golden < day', () => {
    expect(envAnchors.nightBelowDeg).toBeLessThan(envAnchors.goldenPeakDeg);
    expect(envAnchors.goldenPeakDeg).toBeLessThan(envAnchors.dayAboveDeg);
  });
  it('weatherLook coverage window stays inside the raymarcher sweet spot', () => {
    expect(weatherLook.coverageMin).toBeGreaterThanOrEqual(0.1);
    expect(weatherLook.coverageMax).toBeLessThanOrEqual(0.9);
  });
});

// Find a UTC date near a target sun elevation by scanning a summer day.
function utcAtElevation(targetDeg: number, rising: boolean): Date {
  for (let m = 0; m < 1440; m++) {
    const d = new Date(Date.UTC(2026, 5, 21, 0, m));
    const s = sunState(d);
    if (Math.abs(s.elevDeg - targetDeg) < 0.5 && s.rising === rising) return d;
  }
  throw new Error(`no time found for elevation ${targetDeg} rising=${rising}`);
}

describe('lerpColorHex', () => {
  it('endpoints and midpoint', () => {
    expect(lerpColorHex(0x000000, 0xffffff, 0)).toBe(0x000000);
    expect(lerpColorHex(0x000000, 0xffffff, 1)).toBe(0xffffff);
    expect(lerpColorHex(0x000000, 0xff0000, 0.5)).toBe(0x800000);
  });
});

describe('computeEnvironment — keyframe selection', () => {
  it('deep night → night keyframe, stars out, lamp on', () => {
    const e = computeEnvironment(new Date('2026-06-21T23:30:00Z'), CLEAR_SKY);
    expect(e.lampOn01).toBe(1);
    expect(e.starVisibility).toBeGreaterThan(0.8);
    expect(e.sunIntensity).toBeLessThan(0.1);
  });
  it('morning golden hour uses goldenMorning, evening uses goldenEvening (different fog colors)', () => {
    const am = computeEnvironment(utcAtElevation(4, true), CLEAR_SKY);
    const pm = computeEnvironment(utcAtElevation(4, false), CLEAR_SKY);
    expect(am.fogColor).not.toBe(pm.fogColor);
    expect(pm.godraysMix).toBeGreaterThan(am.godraysMix); // dusk drama
  });
  it('high noon → day keyframe, lamp off, no stars', () => {
    const e = computeEnvironment(new Date('2026-06-21T11:25:00Z'), CLEAR_SKY);
    expect(e.lampOn01).toBe(0);
    expect(e.starVisibility).toBe(0);
    expect(e.exposure).toBeCloseTo(0.98, 2);
  });
  it('is continuous across the golden→day boundary (no jumps)', () => {
    let prev = computeEnvironment(utcAtElevation(3, true), CLEAR_SKY);
    for (const elev of [5, 8, 12, 16, 20, 24, 26]) {
      const cur = computeEnvironment(utcAtElevation(elev, true), CLEAR_SKY);
      expect(Math.abs(cur.exposure - prev.exposure)).toBeLessThan(0.12);
      expect(Math.abs(cur.hemiIntensity - prev.hemiIntensity)).toBeLessThan(0.12);
      prev = cur;
    }
  });
});

describe('computeEnvironment — weather modulation', () => {
  const noon = new Date('2026-06-21T11:25:00Z');
  it('overcast raises coverage, damps sun, boosts hemi', () => {
    const clear = computeEnvironment(noon, CLEAR_SKY);
    const overcast = computeEnvironment(noon, { ...CLEAR_SKY, cloudCover: 1 });
    expect(overcast.cloudCoverage).toBeGreaterThan(clear.cloudCoverage);
    expect(overcast.cloudCoverage).toBeLessThanOrEqual(0.85);
    expect(overcast.sunIntensity).toBeLessThan(clear.sunIntensity * 0.4);
    expect(overcast.hemiIntensity).toBeGreaterThan(clear.hemiIntensity);
  });
  it('low visibility / fog code densifies fog', () => {
    const foggy = computeEnvironment(noon, { ...CLEAR_SKY, visibilityM: 150, fog: true });
    const clear = computeEnvironment(noon, CLEAR_SKY);
    expect(foggy.fogFar).toBeLessThan(clear.fogFar / 1.8);
    expect(foggy.fogNear).toBeLessThan(clear.fogNear);
  });
  it('precip type: rain warm, snow cold or explicit snowfall', () => {
    expect(computeEnvironment(noon, { ...CLEAR_SKY, precipMmPerH: 2, temperatureC: 12 }).precipType).toBe('rain');
    expect(computeEnvironment(noon, { ...CLEAR_SKY, precipMmPerH: 2, temperatureC: -1 }).precipType).toBe('snow');
    expect(computeEnvironment(noon, { ...CLEAR_SKY, precipMmPerH: 2, snow: true, temperatureC: 5 }).precipType).toBe('snow');
    expect(computeEnvironment(noon, CLEAR_SKY).precipType).toBe('none');
  });
  it('precip intensity saturates at weatherLook.precipFullMmPerH', () => {
    const e = computeEnvironment(noon, { ...CLEAR_SKY, precipMmPerH: 50 });
    expect(e.precipIntensity).toBe(1);
  });
  it('wind drives cloud drift speed and direction', () => {
    const windy = computeEnvironment(noon, { ...CLEAR_SKY, windSpeedMs: 10, windDirRad: 0 });
    const calm = computeEnvironment(noon, { ...CLEAR_SKY, windSpeedMs: 0 });
    expect(windy.cloudDriftSpeed).toBeGreaterThan(calm.cloudDriftSpeed);
    const len = Math.hypot(windy.cloudDriftDir[0], windy.cloudDriftDir[1]);
    expect(len).toBeCloseTo(1, 5);
  });
  it('clouds dim the stars at night', () => {
    const night = new Date('2026-06-21T23:30:00Z');
    const clear = computeEnvironment(night, CLEAR_SKY);
    const cloudy = computeEnvironment(night, { ...CLEAR_SKY, cloudCover: 0.9 });
    expect(cloudy.starVisibility).toBeLessThan(clear.starVisibility * 0.4);
  });
  it('full moon night is brighter than new moon night', () => {
    // moonIntensity scales with illumination. Pick real dates in 2026 by scanning
    // for a near-full (>0.95) and a near-new (<0.05) moon at 23:00Z, then compare
    // the actual computed environments — not just a bounds check.
    const scanNight = (predicate: (illum: number) => boolean): Date => {
      for (let day = 0; day < 365; day++) {
        const d = new Date(Date.UTC(2026, 0, 1, 23, 0));
        d.setUTCDate(d.getUTCDate() + day);
        if (predicate(moonState(d).illumination)) return d;
      }
      throw new Error('no matching moon night found in 2026');
    };
    const fullNight = scanNight((f) => f > 0.95);
    const newNight = scanNight((f) => f < 0.05);
    const full = computeEnvironment(fullNight, CLEAR_SKY);
    const dark = computeEnvironment(newNight, CLEAR_SKY);
    expect(full.moonIntensity).toBeGreaterThan(dark.moonIntensity);
    expect(full.moonIntensity).toBeLessThanOrEqual(1.6);
    expect(dark.moonIntensity).toBeGreaterThanOrEqual(0);
  });
});

describe('moonPhaseLightDir', () => {
  it('full moon (phase 0.5): lit from the front (toward viewer of the disc)', () => {
    const d = moonPhaseLightDir(0.5);
    expect(d[2]).toBeLessThan(-0.9); // light from -z in disc-local space = fully lit face
  });
  it('new moon (phase 0): lit from behind', () => {
    expect(moonPhaseLightDir(0)[2]).toBeGreaterThan(0.9);
  });
  it('quarter (phase 0.25): lit from the side', () => {
    const d = moonPhaseLightDir(0.25);
    expect(Math.abs(d[0])).toBeGreaterThan(0.9);
  });
});
