// Live weather for Winterthur via Open-Meteo (free, no key, CORS-enabled;
// blends the MeteoSwiss ICON-CH models). Parsing + interpolation are pure;
// only startWeatherLoop touches fetch/localStorage/timers.

export type WeatherState = {
  cloudCover: number;
  precipMmPerH: number;
  snow: boolean;
  windSpeedMs: number;
  windDirRad: number;
  visibilityM: number;
  fog: boolean;
  temperatureC: number;
};

export type WeatherSeries = { timesMs: number[]; states: WeatherState[] };

const FIELDS = 'cloud_cover,precipitation,rain,snowfall,wind_speed_10m,wind_direction_10m,visibility,weather_code,temperature_2m';
export const OPEN_METEO_URL =
  `https://api.open-meteo.com/v1/forecast?latitude=47.499&longitude=8.724&hourly=${FIELDS}` +
  '&past_days=1&forecast_days=2&timeformat=unixtime&wind_speed_unit=ms';

export const CLEAR_SKY: WeatherState = {
  cloudCover: 0.1,
  precipMmPerH: 0,
  snow: false,
  windSpeedMs: 1,
  windDirRad: Math.PI * 1.5, // light westerly
  visibilityM: 30000,
  fog: false,
  temperatureC: 15,
};

type Hourly = Record<string, number[]>;

export function parseOpenMeteo(json: unknown): WeatherSeries {
  const hourly = (json as { hourly?: Hourly } | null)?.hourly;
  const time = hourly?.time;
  if (!hourly || !Array.isArray(time) || time.length === 0) {
    throw new Error('open-meteo payload malformed: missing hourly.time');
  }
  const col = (name: string): number[] => {
    const c = hourly[name];
    if (!Array.isArray(c) || c.length !== time.length) throw new Error(`open-meteo payload malformed: ${name}`);
    return c;
  };
  const cloud = col('cloud_cover');
  const precip = col('precipitation');
  const snowfall = col('snowfall');
  const wind = col('wind_speed_10m');
  const windDir = col('wind_direction_10m');
  const vis = col('visibility');
  const code = col('weather_code');
  const temp = col('temperature_2m');
  return {
    timesMs: time.map((t) => t * 1000),
    states: time.map((_, i) => ({
      cloudCover: cloud[i] / 100,
      precipMmPerH: precip[i],
      snow: snowfall[i] > 0,
      windSpeedMs: wind[i],
      windDirRad: (windDir[i] * Math.PI) / 180,
      visibilityM: vis[i],
      fog: code[i] === 45 || code[i] === 48,
      temperatureC: temp[i],
    })),
  };
}

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

export function sampleWeather(series: WeatherSeries, utc: Date): WeatherState {
  const t = utc.getTime();
  const { timesMs, states } = series;
  if (t <= timesMs[0]) return states[0];
  if (t >= timesMs[timesMs.length - 1]) return states[states.length - 1];
  let i = 0;
  while (timesMs[i + 1] < t) i++;
  const f = (t - timesMs[i]) / (timesMs[i + 1] - timesMs[i]);
  const a = states[i];
  const b = states[i + 1];
  const nearer = f < 0.5 ? a : b;
  return {
    cloudCover: lerp(a.cloudCover, b.cloudCover, f),
    precipMmPerH: lerp(a.precipMmPerH, b.precipMmPerH, f),
    snow: nearer.snow,
    windSpeedMs: lerp(a.windSpeedMs, b.windSpeedMs, f),
    windDirRad: a.windDirRad + (((b.windDirRad - a.windDirRad + 3 * Math.PI) % (2 * Math.PI)) - Math.PI) * f,
    visibilityM: lerp(a.visibilityM, b.visibilityM, f),
    fog: nearer.fog,
    temperatureC: lerp(a.temperatureC, b.temperatureC, f),
  };
}

// --- side-effectful shell -------------------------------------------------

const CACHE_KEY = 'abutown.openMeteo.v1';
const REFRESH_MS = 15 * 60 * 1000;

// localStorage can throw (Safari private mode, storage disabled, quota
// exceeded); degrade to no-cache behavior rather than crashing the loop.
function readCache(): string | null {
  try {
    return localStorage.getItem(CACHE_KEY);
  } catch {
    return null;
  }
}

function writeCache(json: unknown): void {
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(json));
  } catch {
    // no-op: cache is best-effort
  }
}

function clearCache(): void {
  try {
    localStorage.removeItem(CACHE_KEY);
  } catch {
    // no-op: cache is best-effort
  }
}

export function startWeatherLoop(onUpdate: (s: WeatherSeries) => void): void {
  const cached = readCache();
  if (cached) {
    try {
      onUpdate(parseOpenMeteo(JSON.parse(cached)));
    } catch {
      clearCache();
    }
  }
  let backoffMs = 30_000;
  const fetchOnce = async (): Promise<void> => {
    try {
      const res = await fetch(OPEN_METEO_URL);
      if (!res.ok) throw new Error(`open-meteo http ${res.status}`);
      const json = (await res.json()) as unknown;
      const series = parseOpenMeteo(json);
      writeCache(json);
      backoffMs = 30_000;
      onUpdate(series);
    } catch (err) {
      console.warn('[environment] weather fetch failed, keeping last state', err);
      setTimeout(() => void fetchOnce(), backoffMs);
      backoffMs = Math.min(backoffMs * 2, REFRESH_MS);
    }
  };
  void fetchOnce();
  setInterval(() => void fetchOnce(), REFRESH_MS);
}
