import { describe, expect, it } from 'vitest';
import { envAnchors, envKeyframes, weatherLook } from '../../src/diorama/designTokens';

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
