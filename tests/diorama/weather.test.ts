import { afterEach, describe, expect, it, vi } from 'vitest';
import fixture from '../fixtures/openMeteo.json';
import { CLEAR_SKY, OPEN_METEO_URL, parseOpenMeteo, sampleWeather, startWeatherLoop } from '../../src/diorama/environment/weather';

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

describe('startWeatherLoop', () => {
  const CACHE_KEY = 'abutown.openMeteo.v1';
  const REFRESH_MS = 15 * 60 * 1000;

  function makeMemoryStorage(initial: Record<string, string> = {}): Storage {
    const store = new Map<string, string>(Object.entries(initial));
    return {
      getItem: (key: string) => (store.has(key) ? store.get(key)! : null),
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
      removeItem: (key: string) => {
        store.delete(key);
      },
      clear: () => store.clear(),
      key: (index: number) => Array.from(store.keys())[index] ?? null,
      get length() {
        return store.size;
      },
    } as Storage;
  }

  function makeThrowingStorage(): Storage {
    return {
      getItem: () => {
        throw new Error('storage disabled');
      },
      setItem: () => {
        throw new Error('storage disabled');
      },
      removeItem: () => {
        throw new Error('storage disabled');
      },
      clear: () => {
        throw new Error('storage disabled');
      },
      key: () => {
        throw new Error('storage disabled');
      },
      get length(): number {
        throw new Error('storage disabled');
      },
    } as Storage;
  }

  function makeFetchResponse(body: unknown, ok = true, status = 200): Response {
    return {
      ok,
      status,
      json: async () => body,
    } as Response;
  }

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('fetches, calls onUpdate with the parsed series, and caches the raw JSON', async () => {
    vi.useFakeTimers();
    const storage = makeMemoryStorage();
    vi.stubGlobal('localStorage', storage);
    const fetchMock = vi.fn().mockResolvedValue(makeFetchResponse(fixture));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    startWeatherLoop(onUpdate);
    await vi.waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(onUpdate).toHaveBeenCalledWith(parseOpenMeteo(fixture));
    expect(JSON.parse(storage.getItem(CACHE_KEY)!)).toEqual(fixture);
  });

  it('fires onUpdate synchronously from cache before any fetch resolves', () => {
    vi.useFakeTimers();
    const storage = makeMemoryStorage({ [CACHE_KEY]: JSON.stringify(fixture) });
    vi.stubGlobal('localStorage', storage);
    // fetch never resolves within this test, proving the cached onUpdate fired independently of it.
    const fetchMock = vi.fn().mockReturnValue(new Promise(() => {}));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    startWeatherLoop(onUpdate);

    expect(onUpdate).toHaveBeenCalledTimes(1);
    expect(onUpdate).toHaveBeenCalledWith(parseOpenMeteo(fixture));
  });

  it('on fetch failure: no onUpdate, warns, retries with backoff, then resets backoff after success', async () => {
    vi.useFakeTimers();
    const storage = makeMemoryStorage();
    vi.stubGlobal('localStorage', storage);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fetchMock = vi
      .fn()
      .mockRejectedValueOnce(new Error('network down'))
      .mockRejectedValueOnce(new Error('network down again'))
      .mockResolvedValueOnce(makeFetchResponse(fixture));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    startWeatherLoop(onUpdate);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(warnSpy).toHaveBeenCalledTimes(1));
    expect(onUpdate).not.toHaveBeenCalled();

    // advance by the initial 30s backoff: still failing, so backoff should double to 60s
    await vi.advanceTimersByTimeAsync(30_000);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(warnSpy).toHaveBeenCalledTimes(2));
    expect(onUpdate).not.toHaveBeenCalled();

    // a retry at 30s again (rather than the doubled 60s) would fire the 3rd (successful)
    // fetch too early; confirm it has NOT happened yet
    await vi.advanceTimersByTimeAsync(30_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);

    // advancing the remaining 30s of the doubled 60s backoff triggers the retry, which
    // succeeds and resets backoff to 30s
    await vi.advanceTimersByTimeAsync(30_000);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    await vi.waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));
    expect(onUpdate).toHaveBeenCalledWith(parseOpenMeteo(fixture));
  });

  it('does not multiply retry chains when failures span interval boundaries', async () => {
    vi.useFakeTimers();
    const storage = makeMemoryStorage();
    vi.stubGlobal('localStorage', storage);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    // Always fails: the retry chain backs off (30s → 60s → … → capped at
    // REFRESH_MS) while the 15-min interval keeps ticking. With a single-chain
    // guard the interval SKIPS while a retry is pending, so attempts follow the
    // one backoff chain rather than one chain per interval tick.
    const fetchMock = vi.fn().mockRejectedValue(new Error('always down'));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    startWeatherLoop(onUpdate);
    // Warn wording when nothing has ever succeeded (no cache, no prior success).
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(warnSpy.mock.calls[0][0]).toContain('rendering clear-sky default');

    // Advance two full refresh intervals of continuous failure.
    await vi.advanceTimersByTimeAsync(2 * REFRESH_MS);

    // A per-interval-spawned chain would compound: after 2 intervals it would be
    // dozens of concurrent retries. Single-chain behavior over 30 min of 30s→
    // capped backoff is bounded well under this.
    expect(fetchMock.mock.calls.length).toBeLessThanOrEqual(12);
    expect(onUpdate).not.toHaveBeenCalled();
  });

  it('removes a corrupt cache entry without throwing', () => {
    vi.useFakeTimers();
    const storage = makeMemoryStorage({ [CACHE_KEY]: 'not-json{{{' });
    vi.stubGlobal('localStorage', storage);
    const fetchMock = vi.fn().mockReturnValue(new Promise(() => {}));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    expect(() => startWeatherLoop(onUpdate)).not.toThrow();

    expect(onUpdate).not.toHaveBeenCalled();
    expect(storage.getItem(CACHE_KEY)).toBeNull();
  });

  it('refetches after the 15-minute interval elapses', async () => {
    vi.useFakeTimers();
    const storage = makeMemoryStorage();
    vi.stubGlobal('localStorage', storage);
    const fetchMock = vi.fn().mockResolvedValue(makeFetchResponse(fixture));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    startWeatherLoop(onUpdate);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
  });

  it('degrades to no-cache behavior when storage throws on every call, but fetch still succeeds', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('localStorage', makeThrowingStorage());
    const fetchMock = vi.fn().mockResolvedValue(makeFetchResponse(fixture));
    vi.stubGlobal('fetch', fetchMock);
    const onUpdate = vi.fn();

    expect(() => startWeatherLoop(onUpdate)).not.toThrow();
    await vi.waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));

    expect(onUpdate).toHaveBeenCalledWith(parseOpenMeteo(fixture));
  });
});
