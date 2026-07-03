import { describe, expect, it } from 'vitest';
import fixture from '../fixtures/openMeteo.json';
import { CLEAR_SKY, OPEN_METEO_URL, parseOpenMeteo, sampleWeather } from '../../src/diorama/environment/weather';

const T0 = 1783404000 * 1000;
const H = 3_600_000;

describe('OPEN_METEO_URL', () => {
  it('targets Winterthur with all required hourly fields, unixtime, m/s wind', () => {
    expect(OPEN_METEO_URL).toContain('latitude=47.499');
    expect(OPEN_METEO_URL).toContain('longitude=8.724');
    for (const f of ['cloud_cover', 'precipitation', 'rain', 'snowfall', 'wind_speed_10m', 'wind_direction_10m', 'visibility', 'weather_code', 'temperature_2m']) {
      expect(OPEN_METEO_URL).toContain(f);
    }
    expect(OPEN_METEO_URL).toContain('timeformat=unixtime');
    expect(OPEN_METEO_URL).toContain('wind_speed_unit=ms');
    expect(OPEN_METEO_URL).toContain('past_days=1');
  });
});

describe('parseOpenMeteo', () => {
  it('parses the fixture into a 4-point series with normalized units', () => {
    const s = parseOpenMeteo(fixture);
    expect(s.timesMs).toEqual([T0, T0 + H, T0 + 2 * H, T0 + 3 * H]);
    expect(s.states[0]).toEqual({
      cloudCover: 0.2, precipMmPerH: 0, snow: false, windSpeedMs: 1.5,
      windDirRad: Math.PI / 2, visibilityM: 24000, fog: false, temperatureC: 18,
    });
    expect(s.states[2].snow).toBe(true); // snowfall > 0
    expect(s.states[3].fog).toBe(true); // weather_code 48
  });
  it('throws on malformed payload', () => {
    expect(() => parseOpenMeteo({ hourly: {} })).toThrow();
    expect(() => parseOpenMeteo(null)).toThrow();
  });
});

describe('sampleWeather', () => {
  const s = parseOpenMeteo(fixture);
  it('interpolates linearly between hours', () => {
    const mid = sampleWeather(s, new Date(T0 + H / 2));
    expect(mid.cloudCover).toBeCloseTo(0.5, 5);
    expect(mid.windSpeedMs).toBeCloseTo(2.75, 5);
    expect(mid.temperatureC).toBeCloseTo(12, 5);
  });
  it('booleans switch at the nearer hour (no interpolation)', () => {
    expect(sampleWeather(s, new Date(T0 + 1.4 * H)).snow).toBe(false);
    expect(sampleWeather(s, new Date(T0 + 1.6 * H)).snow).toBe(true);
  });
  it('clamps outside the series instead of extrapolating', () => {
    expect(sampleWeather(s, new Date(T0 - 5 * H)).cloudCover).toBeCloseTo(0.2, 5);
    expect(sampleWeather(s, new Date(T0 + 99 * H)).fog).toBe(true);
  });
});

describe('CLEAR_SKY', () => {
  it('is a plausible clear default', () => {
    expect(CLEAR_SKY.cloudCover).toBeLessThan(0.2);
    expect(CLEAR_SKY.precipMmPerH).toBe(0);
    expect(CLEAR_SKY.fog).toBe(false);
  });
});
