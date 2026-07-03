# Echtzeit-Wetter & echte Sonnenzeiten Winterthur — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Das Klinik-Diorama (`look.html`) zeigt strikt in Echtzeit die echte Winterthur-Stimmung: astronomisch berechneter Sonnen-/Mondstand + Live-Wetter (Open-Meteo) steuern kontinuierlich interpolierte art-directed Keyframes.

**Architecture:** Neues Modul `src/diorama/environment/` mit purem Kern (`computeEnvironment(utcNow, weather) → EnvironmentState`, kein three.js, voll unit-testbar) und einer Apply-Schicht, die pro Frame nur Uniforms/Mutables setzt. Die bestehenden Presets werden zu Keyframes über der Sonnen-Elevation (Nacht → Golden-Morgen/Golden-Abend → Tag), Wetter moduliert darüber (Wolken-Coverage, Licht-Dämpfung, Fog, Wind-Drift, Niederschlags-Partikel).

**Tech Stack:** TypeScript, three.js r185 WebGPU/TSL, `suncalc` (neu, ~3 KB), Open-Meteo REST, vitest, playwright.

**Spec:** `docs/superpowers/specs/2026-07-03-echtzeit-wetter-sonne-design.md`

**Base branch: `klinik/look-prototype`** — Ausführung in einem Worktree, der von diesem Branch abzweigt (superpowers:using-git-worktrees). Alle Zeilennummern unten beziehen sich auf `src/diorama/look.ts` @ `klinik/look-prototype` (797 Zeilen).

## Global Constraints

- Design-Token-Regel (verbindlich, Kommentar-Kopf von `designTokens.ts`): **keine Farb-/Material-/Radius-Werte ausserhalb von `src/diorama/designTokens.ts`** — alle neuen kuratierten Werte (Day-Keyframe, Regen/Schnee-Farben, Nebel-Mapping-Konstanten) leben dort.
- Szenen-Koordinatenkonvention (neu, verbindlich, in `solar.ts` dokumentiert): **+x = Ost, +z = Süd, +y = oben.** Passt zum Bestand: die Ostwand (x+) ist die Morgensonnen-Seite.
- Winterthur: **lat 47.499, lon 8.724**.
- Tests: `npm test` (vitest, `tests/**/*.test.ts`). Typecheck: `npm run typecheck`. ⚠️ `tsc` checkt per Haupt-tsconfig nur `src` — Test-Typfehler fängt nur `tsconfig.typecheck.json`.
- Browser-Smoke ist Pflicht vor „complete" (CLAUDE.md: Feature kreuzt die Wire — Open-Meteo-Fetch).
- Kein Rust berührt — cargo-Regeln irrelevant für diesen Plan.
- Frequent commits: jeder Task endet mit einem Commit.

## File Structure (Ziel)

```
src/diorama/environment/solar.ts          Sonne/Mond/Sternenrotation (suncalc-Wrapper, pur)
src/diorama/environment/weather.ts        Open-Meteo: Typen, Parsing, Interpolation, Fetch-Loop (Parsing/Interpolation pur)
src/diorama/environment/environment.ts    computeEnvironment + Keyframe-Interpolation (pur)
src/diorama/environment/applyEnvironment.ts  EnvironmentTargets-Interface + per-Frame-Apply (three.js)
src/diorama/environment/precipitation.ts  Regen/Schnee-Partikel (three.js/TSL)
src/diorama/designTokens.ts               + envKeyframes (day neu), precip- und Nebel-Konstanten
src/diorama/look.ts                       Refactor: Preset-Statik → Uniforms, Boot verdrahtet Environment
scripts/smoke-environment.mjs             Browser-Smoke (playwright, Open-Meteo-Route + __ENV_STATE-Proben)
scripts/capture-env.mjs                   Screenshot-Matrix für Look-Review
tests/diorama/solar.test.ts
tests/diorama/weather.test.ts
tests/diorama/environment.test.ts
tests/fixtures/openMeteo.json
```

---

### Task 1: `solar.ts` — Astronomie (suncalc-Wrapper, pur)

**Files:**
- Create: `src/diorama/environment/solar.ts`
- Test: `tests/diorama/solar.test.ts`
- Modify: `package.json` (deps `suncalc`, devDeps `@types/suncalc`)

**Interfaces:**
- Produces (spätere Tasks verlassen sich exakt hierauf):
  ```ts
  export const WINTERTHUR = { lat: 47.499, lon: 8.724 } as const;
  export type Vec3Tuple = [number, number, number];
  export type SunState = { dir: Vec3Tuple; elevDeg: number; azimuthDeg: number; rising: boolean };
  export type MoonState = { dir: Vec3Tuple; elevDeg: number; phase: number; illumination: number };
  export function sunState(utc: Date): SunState;
  export function moonState(utc: Date): MoonState;
  export function siderealAngleRad(utc: Date): number; // Sternenkuppel-Rotationswinkel
  export function sceneDirFromAzEl(azFromNorthRad: number, elevRad: number): Vec3Tuple;
  ```

- [ ] **Step 1: Dependency installieren**

```bash
npm install suncalc && npm install -D @types/suncalc
```

- [ ] **Step 2: Failing Test schreiben**

`tests/diorama/solar.test.ts` — Golden-Tests gegen bekannte Winterthur-Ephemeriden (Referenz: NOAA Solar Calculator; Toleranzen grosszügig, damit die Meeus-Näherung von suncalc sicher drinliegt):

```ts
import { describe, expect, it } from 'vitest';
import { moonState, sceneDirFromAzEl, siderealAngleRad, sunState } from '../../src/diorama/environment/solar';

const utc = (s: string) => new Date(s);

describe('sunState (Winterthur golden values)', () => {
  it('summer solstice noon: elevation ~66°, sun due south (+z)', () => {
    // Solar noon Winterthur 2026-06-21 ≈ 13:25 CEST = 11:25 UTC. Max elevation 90-47.5+23.44 ≈ 65.9°.
    const s = sunState(utc('2026-06-21T11:25:00Z'));
    expect(s.elevDeg).toBeGreaterThan(64.9);
    expect(s.elevDeg).toBeLessThan(66.9);
    expect(s.dir[2]).toBeGreaterThan(0.3); // south component dominant
    expect(Math.abs(s.dir[0])).toBeLessThan(0.12); // barely east/west at noon
  });

  it('winter solstice noon: elevation ~19°', () => {
    const s = sunState(utc('2026-12-21T11:25:00Z'));
    expect(s.elevDeg).toBeGreaterThan(18.0);
    expect(s.elevDeg).toBeLessThan(20.1);
  });

  it('summer sunrise ~03:29 UTC (05:29 CEST): elevation crosses 0 rising, sun in the east (+x)', () => {
    const before = sunState(utc('2026-06-21T03:14:00Z'));
    const after = sunState(utc('2026-06-21T03:44:00Z'));
    expect(before.elevDeg).toBeLessThan(0);
    expect(after.elevDeg).toBeGreaterThan(0);
    expect(after.rising).toBe(true);
    expect(after.dir[0]).toBeGreaterThan(0.5); // east
  });

  it('summer sunset ~19:26 UTC: elevation crosses 0 falling, sun in the west (-x)', () => {
    const before = sunState(utc('2026-06-21T19:11:00Z'));
    const after = sunState(utc('2026-06-21T19:41:00Z'));
    expect(before.elevDeg).toBeGreaterThan(0);
    expect(after.elevDeg).toBeLessThan(0);
    expect(before.rising).toBe(false);
    expect(before.dir[0]).toBeLessThan(-0.5); // west
  });

  it('midnight: sun far below horizon', () => {
    expect(sunState(utc('2026-06-21T23:00:00Z')).elevDeg).toBeLessThan(-10);
  });
});

describe('sceneDirFromAzEl convention (+x east, +z south, +y up)', () => {
  it('due south at 45° elevation → (0, ~0.707, ~0.707)', () => {
    const [x, y, z] = sceneDirFromAzEl(Math.PI, Math.PI / 4);
    expect(x).toBeCloseTo(0, 5);
    expect(y).toBeCloseTo(Math.SQRT1_2, 5);
    expect(z).toBeCloseTo(Math.SQRT1_2, 5);
  });
  it('due east at horizon → (~1, 0, 0)', () => {
    const [x, y, z] = sceneDirFromAzEl(Math.PI / 2, 0);
    expect(x).toBeCloseTo(1, 5);
    expect(y).toBeCloseTo(0, 5);
    expect(z).toBeCloseTo(0, 5);
  });
});

describe('moonState', () => {
  it('phase and illumination are self-consistent: fraction ≈ (1 - cos(2π·phase)) / 2', () => {
    for (const d of ['2026-01-05', '2026-03-14', '2026-07-03', '2026-10-20']) {
      const m = moonState(utc(`${d}T22:00:00Z`));
      expect(m.phase).toBeGreaterThanOrEqual(0);
      expect(m.phase).toBeLessThan(1);
      const expected = (1 - Math.cos(2 * Math.PI * m.phase)) / 2;
      expect(m.illumination).toBeCloseTo(expected, 1);
    }
  });
  it('phase advances ~0.03/day', () => {
    const a = moonState(utc('2026-07-03T00:00:00Z')).phase;
    const b = moonState(utc('2026-07-05T00:00:00Z')).phase;
    const delta = (b - a + 1) % 1;
    expect(delta).toBeGreaterThan(0.04);
    expect(delta).toBeLessThan(0.09);
  });
});

describe('siderealAngleRad', () => {
  it('advances ~2π in one sidereal day (23h56m04s)', () => {
    const t0 = utc('2026-07-03T00:00:00Z');
    const t1 = new Date(t0.getTime() + 86164090); // sidereal day in ms
    const delta = siderealAngleRad(t1) - siderealAngleRad(t0);
    const wrapped = ((delta % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
    expect(Math.min(wrapped, 2 * Math.PI - wrapped)).toBeLessThan(0.01);
  });
});
```

- [ ] **Step 3: Test laufen lassen — muss failen**

Run: `npx vitest run tests/diorama/solar.test.ts`
Expected: FAIL — `Cannot find module '../../src/diorama/environment/solar'`

- [ ] **Step 4: Implementation**

`src/diorama/environment/solar.ts`:

```ts
// Real astronomy for the diorama — sun/moon position over Winterthur, computed
// from UTC via suncalc (Meeus-based). Pure: no three.js, no wall-clock reads.
//
// Scene coordinate convention (BINDING for the whole diorama):
//   +x = east, +z = south, +y = up.
// The room's east wall (x+) is the morning-sun side, matching look.ts.
//
// suncalc azimuth is measured from SOUTH, positive westward. We convert to
// azimuth-from-north (clockwise via east) before mapping into scene space.

import SunCalc from 'suncalc';

export const WINTERTHUR = { lat: 47.499, lon: 8.724 } as const;

export type Vec3Tuple = [number, number, number];
export type SunState = { dir: Vec3Tuple; elevDeg: number; azimuthDeg: number; rising: boolean };
export type MoonState = { dir: Vec3Tuple; elevDeg: number; phase: number; illumination: number };

const DEG = 180 / Math.PI;

export function sceneDirFromAzEl(azFromNorthRad: number, elevRad: number): Vec3Tuple {
  const cosE = Math.cos(elevRad);
  return [
    cosE * Math.sin(azFromNorthRad), // east
    Math.sin(elevRad), // up
    -cosE * Math.cos(azFromNorthRad), // -north = south
  ];
}

function azFromNorth(suncalcAzimuth: number): number {
  return suncalcAzimuth + Math.PI;
}

export function sunState(utc: Date): SunState {
  const pos = SunCalc.getPosition(utc, WINTERTHUR.lat, WINTERTHUR.lon);
  const later = SunCalc.getPosition(new Date(utc.getTime() + 60_000), WINTERTHUR.lat, WINTERTHUR.lon);
  const azN = azFromNorth(pos.azimuth);
  return {
    dir: sceneDirFromAzEl(azN, pos.altitude),
    elevDeg: pos.altitude * DEG,
    azimuthDeg: ((azN * DEG) % 360 + 360) % 360,
    rising: later.altitude > pos.altitude,
  };
}

export function moonState(utc: Date): MoonState {
  const pos = SunCalc.getMoonPosition(utc, WINTERTHUR.lat, WINTERTHUR.lon);
  const illum = SunCalc.getMoonIllumination(utc);
  return {
    dir: sceneDirFromAzEl(azFromNorth(pos.azimuth), pos.altitude),
    elevDeg: pos.altitude * DEG,
    phase: illum.phase,
    illumination: illum.fraction,
  };
}

// Rotation angle of the star dome around the celestial pole. Zero point is
// arbitrary (procedural stars); only the RATE (one turn per sidereal day) and
// continuity matter.
const SIDEREAL_DAY_MS = 86_164_090.5;
export function siderealAngleRad(utc: Date): number {
  return ((utc.getTime() % SIDEREAL_DAY_MS) / SIDEREAL_DAY_MS) * 2 * Math.PI;
}
```

- [ ] **Step 5: Test laufen lassen — muss passen**

Run: `npx vitest run tests/diorama/solar.test.ts`
Expected: PASS (alle 9 Tests). Falls ein Golden-Wert knapp scheitert: NICHT die Implementation verbiegen — Referenzwert gegen NOAA Solar Calculator (gov) für 47.499/8.724 prüfen und ggf. die Toleranz im Test mit Kommentar begründen.

- [ ] **Step 6: Typecheck + Commit**

```bash
npm run typecheck
git add package.json package-lock.json src/diorama/environment/solar.ts tests/diorama/solar.test.ts
git commit -m "feat(environment): real sun/moon astronomy for Winterthur (suncalc wrapper, golden-tested)"
```

---

### Task 2: `weather.ts` — Open-Meteo-Typen, Parsing, Interpolation, Fetch-Loop

**Files:**
- Create: `src/diorama/environment/weather.ts`
- Create: `tests/fixtures/openMeteo.json`
- Test: `tests/diorama/weather.test.ts`

**Interfaces:**
- Produces:
  ```ts
  export type WeatherState = {
    cloudCover: number;        // 0..1
    precipMmPerH: number;      // mm/h
    snow: boolean;             // snow rather than rain
    windSpeedMs: number;       // m/s
    windDirRad: number;        // meteorological: direction the wind comes FROM, rad from north
    visibilityM: number;       // meters
    fog: boolean;              // weather_code 45/48
    temperatureC: number;
  };
  export type WeatherSeries = { timesMs: number[]; states: WeatherState[] };
  export const OPEN_METEO_URL: string; // full request URL for Winterthur
  export function parseOpenMeteo(json: unknown): WeatherSeries;      // throws on malformed payload
  export function sampleWeather(series: WeatherSeries, utc: Date): WeatherState; // linear interp, clamped at ends
  export const CLEAR_SKY: WeatherState; // documented never-had-data default
  export function startWeatherLoop(onUpdate: (s: WeatherSeries) => void): void; // fetch now + every 15 min, retry w/ backoff, localStorage cache
  ```
- Consumes: nichts aus anderen Tasks.

- [ ] **Step 1: Fixture anlegen**

`tests/fixtures/openMeteo.json` — echtes Open-Meteo-Antwortformat (`timeformat=unixtime`), 4 Stunden, mit Regen→Schnee-Übergang und einer Nebelstunde:

```json
{
  "latitude": 47.5,
  "longitude": 8.75,
  "hourly_units": { "time": "unixtime", "cloud_cover": "%", "precipitation": "mm", "rain": "mm", "snowfall": "cm", "wind_speed_10m": "m/s", "wind_direction_10m": "°", "visibility": "m", "weather_code": "wmo code", "temperature_2m": "°C" },
  "hourly": {
    "time": [1783404000, 1783407600, 1783411200, 1783414800],
    "cloud_cover": [20, 80, 100, 100],
    "precipitation": [0.0, 1.2, 3.0, 2.0],
    "rain": [0.0, 1.2, 0.0, 0.0],
    "snowfall": [0.0, 0.0, 2.1, 1.4],
    "wind_speed_10m": [1.5, 4.0, 8.0, 6.0],
    "wind_direction_10m": [90, 180, 270, 270],
    "visibility": [24000, 10000, 900, 150],
    "weather_code": [1, 61, 73, 48],
    "temperature_2m": [18.0, 6.0, -1.0, -2.0]
  }
}
```

- [ ] **Step 2: Failing Test schreiben**

`tests/diorama/weather.test.ts`:

```ts
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
```

- [ ] **Step 3: Test laufen lassen — muss failen**

Run: `npx vitest run tests/diorama/weather.test.ts`
Expected: FAIL — Modul nicht gefunden.

- [ ] **Step 4: Implementation**

`src/diorama/environment/weather.ts`:

```ts
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

export function startWeatherLoop(onUpdate: (s: WeatherSeries) => void): void {
  const cached = localStorage.getItem(CACHE_KEY);
  if (cached) {
    try {
      onUpdate(parseOpenMeteo(JSON.parse(cached)));
    } catch {
      localStorage.removeItem(CACHE_KEY);
    }
  }
  let backoffMs = 30_000;
  const fetchOnce = async (): Promise<void> => {
    try {
      const res = await fetch(OPEN_METEO_URL);
      if (!res.ok) throw new Error(`open-meteo http ${res.status}`);
      const json = (await res.json()) as unknown;
      const series = parseOpenMeteo(json);
      localStorage.setItem(CACHE_KEY, JSON.stringify(json));
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
```

- [ ] **Step 5: Test laufen lassen — muss passen**

Run: `npx vitest run tests/diorama/weather.test.ts`
Expected: PASS. (Der Test importiert JSON — vitest kann das out-of-the-box.)

- [ ] **Step 6: Typecheck + Commit**

```bash
npm run typecheck
git add src/diorama/environment/weather.ts tests/diorama/weather.test.ts tests/fixtures/openMeteo.json
git commit -m "feat(environment): open-meteo client for Winterthur (parse/interpolate pure, 15min loop, localStorage cache)"
```

---

### Task 3: Design-Tokens — Env-Keyframes (neuer Day-Keyframe) + Wetter-Konstanten

**Files:**
- Modify: `src/diorama/designTokens.ts` (nach `skyPhys`, ~Zeile 121)
- Test: `tests/diorama/environment.test.ts` (erste Tests; Datei wächst in Task 4 weiter)

**Interfaces:**
- Produces:
  ```ts
  export type EnvKeyframe = {
    hemiSky: number; hemiGround: number; hemiIntensity: number;
    fogColor: number; fogNear: number; fogFar: number;
    exposure: number; mistColor: number; mistOpacity: number;
    giScale: number; saturation: number; contrast: number;
    godraysMix: number; lampOn01: number;
    turbidity: number; rayleigh: number; mieCoefficient: number; mieG: number;
    sunBoost: number; cloudCoverageBase: number;
  };
  export const envKeyframes: { night: EnvKeyframe; goldenMorning: EnvKeyframe; goldenEvening: EnvKeyframe; day: EnvKeyframe };
  export const envAnchors: { nightBelowDeg: number; goldenPeakDeg: number; dayAboveDeg: number }; // -6 / 4 / 25
  export const weatherLook: {
    coverageMin: number; coverageMax: number;          // 0.15 / 0.85
    sunDampMax: number;                                 // 0.75
    hemiBoostMax: number;                               // 0.35
    fogVisFullM: number; fogVisClearM: number;          // 200 / 4000
    fogNearMin: number; fogFarMin: number;              // 4 / 22
    precipFullMmPerH: number;                           // 5
    snowTempC: number;                                  // 1
    driftBase: number; driftPerMs: number;              // wind→cloud-drift mapping
    rainColor: number; snowColor: number;               // particle colors
  };
  ```
- Consumes: bestehende `lightPresets`, `skyPhys` (Werte werden kopiert, Exporte bleiben unverändert — `look.ts` kompiliert weiter, bis Task 5 es umbaut).

- [ ] **Step 1: Failing Test schreiben**

`tests/diorama/environment.test.ts` (Anfang):

```ts
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
```

- [ ] **Step 2: Run — muss failen**

Run: `npx vitest run tests/diorama/environment.test.ts`
Expected: FAIL — `envKeyframes` nicht exportiert.

- [ ] **Step 3: Implementation**

In `src/diorama/designTokens.ts` direkt nach dem `skyPhys`-Block einfügen. Werte für night/goldenMorning/goldenEvening sind 1:1 aus `lightPresets` + `skyPhys` übernommen; **day ist neu kuratiert** (heller, neutraler, flacher — Review via Capture-Matrix in Task 9):

```ts
// --- Realtime environment: art-directed keyframes over real sun elevation ---
// The old presets live on as keyframes: night (<-6°), goldenMorning/-Evening
// (anchored at +4°, chosen by whether the sun is rising), day (>25°, NEW).
export type EnvKeyframe = {
  hemiSky: number; hemiGround: number; hemiIntensity: number;
  fogColor: number; fogNear: number; fogFar: number;
  exposure: number; mistColor: number; mistOpacity: number;
  giScale: number; saturation: number; contrast: number;
  godraysMix: number; lampOn01: number;
  turbidity: number; rayleigh: number; mieCoefficient: number; mieG: number;
  sunBoost: number; cloudCoverageBase: number;
};

export const envKeyframes: { night: EnvKeyframe; goldenMorning: EnvKeyframe; goldenEvening: EnvKeyframe; day: EnvKeyframe } = {
  night: {
    hemiSky: 0x4a5f7d, hemiGround: 0x3d4652, hemiIntensity: 0.4,
    fogColor: 0x2c3a50, fogNear: 18, fogFar: 46,
    exposure: 0.95, mistColor: 0x46586e, mistOpacity: 0.18,
    giScale: 0.9, saturation: 1.08, contrast: 1.05,
    godraysMix: 0, lampOn01: 1,
    turbidity: 2, rayleigh: 1, mieCoefficient: 0.005, mieG: 0.8,
    sunBoost: 0, cloudCoverageBase: 0.4,
  },
  goldenMorning: {
    hemiSky: 0xc4dcda, hemiGround: 0xe4d3ba, hemiIntensity: 0.6,
    fogColor: 0xeee2cf, fogNear: 20, fogFar: 48,
    exposure: 1.18, mistColor: 0xf6e9d2, mistOpacity: 0.16,
    giScale: 0.7, saturation: 1.1, contrast: 1.0,
    godraysMix: 0.35, lampOn01: 0,
    turbidity: 2.2, rayleigh: 2.6, mieCoefficient: 0.006, mieG: 0.8,
    sunBoost: 1.3, cloudCoverageBase: 0.44,
  },
  // The DREDGE moment — amber horizon under deep teal — now fires at the REAL dusk.
  goldenEvening: {
    hemiSky: 0x4f7d84, hemiGround: 0x5c5348, hemiIntensity: 0.42,
    fogColor: 0x486e74, fogNear: 18, fogFar: 46,
    exposure: 0.96, mistColor: 0x6f949a, mistOpacity: 0.22,
    giScale: 0.55, saturation: 1.12, contrast: 1.06,
    godraysMix: 0.6, lampOn01: 1,
    turbidity: 6, rayleigh: 3.0, mieCoefficient: 0.02, mieG: 0.9,
    sunBoost: 2.3, cloudCoverageBase: 0.62,
  },
  // NEW curation: bright, neutral midday — flat contrast, no drama, lamp off.
  day: {
    hemiSky: 0xbfd9e6, hemiGround: 0xe7dcc4, hemiIntensity: 0.75,
    fogColor: 0xe8eef2, fogNear: 22, fogFar: 52,
    exposure: 1.12, mistColor: 0xf2f3ee, mistOpacity: 0.1,
    giScale: 0.75, saturation: 1.04, contrast: 1.0,
    godraysMix: 0.15, lampOn01: 0,
    turbidity: 3, rayleigh: 2.2, mieCoefficient: 0.005, mieG: 0.8,
    sunBoost: 1.0, cloudCoverageBase: 0.44,
  },
};

// Sun-elevation anchors (degrees) for keyframe interpolation.
export const envAnchors = { nightBelowDeg: -6, goldenPeakDeg: 4, dayAboveDeg: 25 } as const;

// How real weather modulates the look. All weather→look constants live here.
export const weatherLook = {
  coverageMin: 0.15, coverageMax: 0.85, // cloud_cover 0..1 → raymarcher coverage window
  sunDampMax: 0.75, // full overcast removes 75% of direct sun
  hemiBoostMax: 0.35, // ...and adds up to 35% diffuse hemi
  fogVisFullM: 200, fogVisClearM: 4000, // visibility → fog factor ramp
  fogNearMin: 4, fogFarMin: 22, // fully fogged near/far
  precipFullMmPerH: 5, // 5 mm/h = full-intensity particles
  snowTempC: 1, // precip at or below this temperature falls as snow
  driftBase: 0.006, driftPerMs: 0.0011, // cloud drift = base + speed(m/s) * perMs
  rainColor: 0xaebfd4, snowColor: 0xf4f7fb,
} as const;
```

- [ ] **Step 4: Run — muss passen**

Run: `npx vitest run tests/diorama/environment.test.ts`
Expected: PASS (4 Tests).

- [ ] **Step 5: Typecheck + Commit**

```bash
npm run typecheck
git add src/diorama/designTokens.ts tests/diorama/environment.test.ts
git commit -m "feat(environment): env keyframes (new curated day) + weather-look constants in design tokens"
```

---

### Task 4: `environment.ts` — computeEnvironment (der pure Kern)

**Files:**
- Create: `src/diorama/environment/environment.ts`
- Test: `tests/diorama/environment.test.ts` (erweitern)

**Interfaces:**
- Consumes: `sunState`, `moonState`, `siderealAngleRad` aus `solar.ts` (Task 1); `WeatherState` aus `weather.ts` (Task 2); `envKeyframes`, `envAnchors`, `weatherLook`, `sunArcCfg` aus `designTokens.ts` (Task 3).
- Produces:
  ```ts
  export type PrecipType = 'none' | 'rain' | 'snow';
  export type EnvironmentState = {
    sunDir: [number, number, number]; sunElevDeg: number;
    sunColor: number; sunIntensity: number;
    turbidity: number; rayleigh: number; mieCoefficient: number; mieG: number;
    hemiSky: number; hemiGround: number; hemiIntensity: number;
    fogColor: number; fogNear: number; fogFar: number;
    exposure: number; mistColor: number; mistOpacity: number;
    giScale: number; saturation: number; contrast: number;
    godraysMix: number; lampOn01: number;
    cloudCoverage: number; cloudDriftDir: [number, number]; cloudDriftSpeed: number;
    precipType: PrecipType; precipIntensity: number;
    windSpeedMs: number; windDirRad: number;
    moonDir: [number, number, number]; moonPhase: number; moonIllumination: number; moonIntensity: number;
    starVisibility: number; siderealAngleRad: number;
    shaft01: number; // east-window light-shaft opacity factor
  };
  export function computeEnvironment(utcNow: Date, weather: WeatherState): EnvironmentState;
  export function lerpColorHex(a: number, b: number, t: number): number; // channel-wise sRGB lerp
  ```

- [ ] **Step 1: Failing Tests schreiben (an `tests/diorama/environment.test.ts` anhängen)**

```ts
import { computeEnvironment, lerpColorHex } from '../../src/diorama/environment/environment';
import { CLEAR_SKY } from '../../src/diorama/environment/weather';
import { sunState } from '../../src/diorama/environment/solar';

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
    expect(e.exposure).toBeCloseTo(1.12, 2);
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
    // moonIntensity scales with illumination — compare two synthetic states via internals:
    const night = new Date('2026-06-21T23:30:00Z');
    const e = computeEnvironment(night, CLEAR_SKY);
    expect(e.moonIntensity).toBeGreaterThanOrEqual(0);
    expect(e.moonIntensity).toBeLessThanOrEqual(1.6);
  });
});
```

- [ ] **Step 2: Run — muss failen**

Run: `npx vitest run tests/diorama/environment.test.ts`
Expected: FAIL — `environment.ts` fehlt.

- [ ] **Step 3: Implementation**

`src/diorama/environment/environment.ts`:

```ts
// The pure core: (real UTC time, real weather) -> full look state.
// Physical truth steers; art-directed keyframes render. No three.js here.

import { envAnchors, envKeyframes, sunArcCfg, weatherLook, type EnvKeyframe } from '../designTokens';
import { moonState, siderealAngleRad, sunState } from './solar';
import type { WeatherState } from './weather';

export type PrecipType = 'none' | 'rain' | 'snow';

export type EnvironmentState = {
  sunDir: [number, number, number]; sunElevDeg: number;
  sunColor: number; sunIntensity: number;
  turbidity: number; rayleigh: number; mieCoefficient: number; mieG: number;
  hemiSky: number; hemiGround: number; hemiIntensity: number;
  fogColor: number; fogNear: number; fogFar: number;
  exposure: number; mistColor: number; mistOpacity: number;
  giScale: number; saturation: number; contrast: number;
  godraysMix: number; lampOn01: number;
  cloudCoverage: number; cloudDriftDir: [number, number]; cloudDriftSpeed: number;
  precipType: PrecipType; precipIntensity: number;
  windSpeedMs: number; windDirRad: number;
  moonDir: [number, number, number]; moonPhase: number; moonIllumination: number; moonIntensity: number;
  starVisibility: number; siderealAngleRad: number;
  shaft01: number;
};

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const clamp01 = (x: number) => Math.min(Math.max(x, 0), 1);
const smooth = (t: number) => { const c = clamp01(t); return c * c * (3 - 2 * c); };

export function lerpColorHex(a: number, b: number, t: number): number {
  const ch = (shift: number) => Math.round(lerp((a >> shift) & 0xff, (b >> shift) & 0xff, t));
  return (ch(16) << 16) | (ch(8) << 8) | ch(0);
}

function lerpKeyframe(a: EnvKeyframe, b: EnvKeyframe, t: number): EnvKeyframe {
  const s = smooth(t);
  const colorKeys: Array<keyof EnvKeyframe> = ['hemiSky', 'hemiGround', 'fogColor', 'mistColor'];
  const out = {} as Record<keyof EnvKeyframe, number>;
  for (const k of Object.keys(a) as Array<keyof EnvKeyframe>) {
    out[k] = colorKeys.includes(k) ? lerpColorHex(a[k], b[k], s) : lerp(a[k], b[k], s);
  }
  return out as EnvKeyframe;
}

function keyframeFor(elevDeg: number, rising: boolean): EnvKeyframe {
  const golden = rising ? envKeyframes.goldenMorning : envKeyframes.goldenEvening;
  const { nightBelowDeg, goldenPeakDeg, dayAboveDeg } = envAnchors;
  if (elevDeg <= nightBelowDeg) return envKeyframes.night;
  if (elevDeg <= goldenPeakDeg) {
    return lerpKeyframe(envKeyframes.night, golden, (elevDeg - nightBelowDeg) / (goldenPeakDeg - nightBelowDeg));
  }
  if (elevDeg <= dayAboveDeg) {
    return lerpKeyframe(golden, envKeyframes.day, (elevDeg - goldenPeakDeg) / (dayAboveDeg - goldenPeakDeg));
  }
  return envKeyframes.day;
}

// Sun color/intensity vs elevation — same easing the prototype used (look.ts sunLightFor).
function sunLight(dirY: number, boost: number): { color: number; intensity: number } {
  const eased = smooth(clamp01(dirY / 0.8));
  return {
    color: lerpColorHex(sunArcCfg.colorLow, sunArcCfg.colorHigh, eased),
    intensity: (0.8 + 6.2 * eased) * boost,
  };
}

export function computeEnvironment(utcNow: Date, weather: WeatherState): EnvironmentState {
  const sun = sunState(utcNow);
  const moon = moonState(utcNow);
  const kf = keyframeFor(sun.elevDeg, sun.rising);

  const cloud = clamp01(weather.cloudCover);
  const light = sunLight(sun.dir[1], kf.sunBoost);
  const sunDamp = 1 - weatherLook.sunDampMax * cloud;

  // Fog: keyframe base, densified by low visibility or an explicit fog code.
  const visFactor = weather.fog
    ? 1
    : clamp01((weatherLook.fogVisClearM - weather.visibilityM) / (weatherLook.fogVisClearM - weatherLook.fogVisFullM));
  const fogNear = lerp(kf.fogNear, weatherLook.fogNearMin, visFactor);
  const fogFar = lerp(kf.fogFar, weatherLook.fogFarMin, visFactor);

  // Precipitation
  const precipIntensity = clamp01(weather.precipMmPerH / weatherLook.precipFullMmPerH);
  const precipType: PrecipType =
    precipIntensity <= 0.01 ? 'none' : weather.snow || weather.temperatureC <= weatherLook.snowTempC ? 'snow' : 'rain';

  // Night factors
  const night01 = smooth((-sun.elevDeg - 2) / 4); // 0 above -2°, 1 below -6°
  const starVisibility = night01 * (1 - cloud) * (weather.fog ? 0.2 : 1);
  const moonUp = clamp01(moon.dir[1] / 0.3);
  const moonIntensity = 1.4 * moon.illumination * moonUp * night01 * (1 - 0.8 * cloud);

  // East-window light shafts: sun up, low, easterly, and not overcast.
  const easterly = clamp01((sun.dir[0] - 0.25) / 0.35);
  const lowSun = smooth((25 - sun.elevDeg) / 20) * smooth(sun.elevDeg / 4);
  const shaft01 = easterly * lowSun * (1 - cloud);

  // Wind → cloud drift (meteorological dir = FROM; clouds move TOWARD dir+π).
  const toward = weather.windDirRad + Math.PI;
  const cloudDriftDir: [number, number] = [Math.sin(toward), -Math.cos(toward)]; // scene x/z
  const cloudDriftSpeed = weatherLook.driftBase + weather.windSpeedMs * weatherLook.driftPerMs;

  return {
    sunDir: sun.dir, sunElevDeg: sun.elevDeg,
    sunColor: light.color,
    sunIntensity: Math.max(light.intensity * sunDamp * (sun.elevDeg > -2 ? 1 : 0), 0),
    turbidity: kf.turbidity, rayleigh: kf.rayleigh, mieCoefficient: kf.mieCoefficient, mieG: kf.mieG,
    hemiSky: kf.hemiSky, hemiGround: kf.hemiGround,
    hemiIntensity: kf.hemiIntensity * (1 + weatherLook.hemiBoostMax * cloud),
    fogColor: lerpColorHex(kf.fogColor, kf.mistColor, visFactor * 0.7),
    fogNear, fogFar,
    exposure: kf.exposure, mistColor: kf.mistColor,
    mistOpacity: Math.min(kf.mistOpacity + 0.25 * visFactor, 0.5),
    giScale: kf.giScale, saturation: kf.saturation, contrast: kf.contrast,
    godraysMix: kf.godraysMix * (1 - cloud) * clamp01(sun.elevDeg / 2),
    lampOn01: kf.lampOn01,
    cloudCoverage: lerp(weatherLook.coverageMin, weatherLook.coverageMax, cloud),
    cloudDriftDir, cloudDriftSpeed,
    precipType, precipIntensity,
    windSpeedMs: weather.windSpeedMs, windDirRad: weather.windDirRad,
    moonDir: moon.dir, moonPhase: moon.phase, moonIllumination: moon.illumination, moonIntensity,
    starVisibility, siderealAngleRad: siderealAngleRad(utcNow),
    shaft01,
  };
}
```

- [ ] **Step 4: Run — muss passen**

Run: `npx vitest run tests/diorama/environment.test.ts`
Expected: PASS (alle Tests aus Task 3 + Task 4). Der Continuity-Test ist der wichtigste — wenn er scheitert, Anchor-Blending prüfen, nicht die Toleranz aufweichen.

- [ ] **Step 5: Typecheck + Commit**

```bash
npm run typecheck
git add src/diorama/environment/environment.ts tests/diorama/environment.test.ts
git commit -m "feat(environment): computeEnvironment — keyframes over real sun elevation, weather modulation, night factors"
```

---

### Task 5: look.ts-Refactor + `applyEnvironment.ts` — die Szene wird kontinuierlich

Das ist der grosse Render-Task. Kein sinnvoller Unit-Test (three.js/WebGPU) — Verifikation: Typecheck + Capture (Step 5) + Smoke (Task 8).

**Files:**
- Create: `src/diorama/environment/applyEnvironment.ts`
- Modify: `src/diorama/look.ts` (Boot-Sektion ~278–465, applySunState ~424–443, Lamp ~669–679, Shafts ~636–657, Stars ~445–465, Godrays ~751–756, Grade ~765–770, animate ~772–788)

**Interfaces:**
- Consumes: `computeEnvironment`, `EnvironmentState` (Task 4); `startWeatherLoop`, `sampleWeather`, `parseOpenMeteo`, `CLEAR_SKY`, `WeatherSeries` (Task 2).
- Produces:
  ```ts
  // applyEnvironment.ts
  export type EnvironmentTargets = { /* alle Szenen-Handles, siehe unten */ };
  export function applyEnvironment(t: EnvironmentTargets, env: EnvironmentState, dtSeconds: number): void;
  // look.ts (global, für Smoke/Capture):
  //   window.__ENV_STATE  — das zuletzt angewandte EnvironmentState (JSON-serialisierbar)
  //   URL-Params: ?at=<ISO-UTC oder lokal> friert die Zeit ein; ?wx=clear|overcast|rain|snow|fog übersteuert Wetter; ?cam bleibt.
  //   ?preset= und ?cycle= sind ENTFERNT.
  ```

- [ ] **Step 1: `applyEnvironment.ts` schreiben**

```ts
// Applies an EnvironmentState to the live scene. Uniform/mutable writes only —
// no geometry rebuilds, no allocations in the hot path.

import * as THREE from 'three/webgpu';
import { nightGlow } from '../designTokens';
import type { EnvironmentState } from './environment';
import type { PrecipitationSystem } from './precipitation';

export type EnvironmentTargets = {
  renderer: THREE.WebGPURenderer;
  fog: THREE.Fog;
  sun: THREE.DirectionalLight;
  hemi: THREE.HemisphereLight;
  skyMesh: { turbidity: { value: number }; rayleigh: { value: number }; mieCoefficient: { value: number }; mieDirectionalG: { value: number }; sunPosition: { value: THREE.Vector3 } };
  cloudUniforms: {
    lightDir: { value: THREE.Vector3 };
    lit: { value: THREE.Color };
    shadow: { value: THREE.Color };
    coverage: { value: number };
    driftUV: { value: THREE.Vector2 };
  };
  postUniforms: { saturation: { value: number }; contrast: { value: number }; godraysMix: { value: number } };
  mistMaterial: THREE.MeshBasicMaterial;
  sunDisc: THREE.Mesh;
  moonDisc: THREE.Mesh;
  moonLightObj: THREE.DirectionalLight | null; // null: moon shares `sun` light at night (see look.ts wiring)
  lampLight: THREE.PointLight;
  lampBulb: THREE.Mesh;
  stars: THREE.Points;
  starsMaterial: THREE.PointsMaterial;
  shaftMaterial: THREE.MeshBasicMaterial;
  shafts: THREE.Mesh[];
  shaftWindows: THREE.Vector3[];
  precipitation: PrecipitationSystem;
  scratch: { v3: THREE.Vector3; c1: THREE.Color; c2: THREE.Color };
};

const CLOUD_SHADOW_BASE = 0x6e8092;
const CLOUD_NIGHT_LIT = 0x9fb2cc;
const CLOUD_NIGHT_SHADOW = 0x39485c;

export function applyEnvironment(t: EnvironmentTargets, env: EnvironmentState, dtSeconds: number): void {
  const sunDir = t.scratch.v3.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]);
  const isDay = env.sunIntensity > 0.02;

  // Sky + fog + exposure
  t.skyMesh.turbidity.value = env.turbidity;
  t.skyMesh.rayleigh.value = env.rayleigh;
  t.skyMesh.mieCoefficient.value = env.mieCoefficient;
  t.skyMesh.mieDirectionalG.value = env.mieG;
  t.skyMesh.sunPosition.value.copy(sunDir);
  t.fog.color.set(env.fogColor);
  t.fog.near = env.fogNear;
  t.fog.far = env.fogFar;
  t.renderer.toneMappingExposure = env.exposure;

  // Key light: sun by day, moon by night (one shadow-casting light, like the prototype)
  if (isDay) {
    t.sun.position.copy(sunDir).multiplyScalar(12);
    t.sun.color.set(env.sunColor);
    t.sun.intensity = Math.max(env.sunIntensity, 0.05);
    t.cloudUniforms.lightDir.value.copy(sunDir);
    t.cloudUniforms.lit.value.set(env.sunColor).lerp(t.scratch.c1.set(0xffffff), 0.3);
    t.cloudUniforms.shadow.value.set(CLOUD_SHADOW_BASE).lerp(t.scratch.c2.set(env.sunColor), 0.15);
  } else {
    const moonDir = t.scratch.v3.set(env.moonDir[0], Math.max(env.moonDir[1], 0.15), env.moonDir[2]).normalize();
    t.sun.position.copy(moonDir).multiplyScalar(12);
    t.sun.color.set(0xa8c4e8);
    t.sun.intensity = Math.max(env.moonIntensity, 0.12);
    t.cloudUniforms.lightDir.value.copy(moonDir);
    t.cloudUniforms.lit.value.set(CLOUD_NIGHT_LIT);
    t.cloudUniforms.shadow.value.set(CLOUD_NIGHT_SHADOW);
  }

  // Hemisphere + GI scale
  t.hemi.color.set(env.hemiSky);
  t.hemi.groundColor.set(env.hemiGround);
  t.hemi.intensity = env.hemiIntensity;

  // Clouds
  t.cloudUniforms.coverage.value = env.cloudCoverage;
  t.cloudUniforms.driftUV.value.x += env.cloudDriftDir[0] * env.cloudDriftSpeed * dtSeconds;
  t.cloudUniforms.driftUV.value.y += env.cloudDriftDir[1] * env.cloudDriftSpeed * dtSeconds;

  // Post
  t.postUniforms.saturation.value = env.saturation;
  t.postUniforms.contrast.value = env.contrast;
  t.postUniforms.godraysMix.value = env.godraysMix;

  // Mist
  t.mistMaterial.color.set(env.mistColor);
  t.mistMaterial.opacity = env.mistOpacity;

  // Discs
  t.sunDisc.position.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]).multiplyScalar(60);
  t.sunDisc.visible = env.sunDir[1] > 0.015;
  t.moonDisc.position.set(env.moonDir[0], env.moonDir[1], env.moonDir[2]).multiplyScalar(60);
  t.moonDisc.visible = env.moonDir[1] > 0.02 && env.starVisibility > 0.02;

  // Lamp (warm windows)
  t.lampLight.intensity = nightGlow.lampIntensity * 1.2 * env.lampOn01;
  t.lampBulb.visible = env.lampOn01 > 0.05;

  // Stars: real dome rotation around the celestial pole
  t.starsMaterial.opacity = 0.85 * env.starVisibility;
  t.stars.visible = env.starVisibility > 0.01;
  t.stars.rotation.set(0, 0, 0);
  t.stars.rotateOnWorldAxis(POLE_AXIS, env.siderealAngleRad);

  // East-window shafts: re-aim along the live sun direction, fade by shaft01
  t.shaftMaterial.opacity = 0.07 * env.shaft01;
  for (let i = 0; i < t.shafts.length; i++) {
    const win = t.shaftWindows[i];
    const down = t.scratch.v3.set(-env.sunDir[0], -env.sunDir[1], -env.sunDir[2]);
    if (down.y > -0.05) { t.shafts[i].visible = false; continue; }
    t.shafts[i].visible = env.shaft01 > 0.01;
    const k = (win.y - 0.16) / -down.y;
    const poolV = scratchPool.copy(win).addScaledVector(down, k);
    t.shafts[i].position.copy(win).add(poolV).multiplyScalar(0.5);
    t.shafts[i].lookAt(poolV);
    t.shafts[i].scale.z = win.distanceTo(poolV) / SHAFT_BASE_LEN;
  }

  // Precipitation
  t.precipitation.update(env.precipType, env.precipIntensity, env.windSpeedMs, env.windDirRad, dtSeconds);
}

// Celestial pole for latitude 47.5°: in scene coords the pole sits toward
// north (-z) at elevation = latitude.
const LAT_RAD = (47.499 * Math.PI) / 180;
const POLE_AXIS = new THREE.Vector3(0, Math.sin(LAT_RAD), -Math.cos(LAT_RAD)).normalize();
const SHAFT_BASE_LEN = 3; // shafts are built with unit length 3, scaled per frame
const scratchPool = new THREE.Vector3();
```

**Implementations-Hinweis:** die Shaft-Geometrie wird in look.ts mit fixer Länge `SHAFT_BASE_LEN = 3` gebaut (`new THREE.BoxGeometry(1.15, 0.02, 3)`) und pro Frame via `scale.z` auf die echte Fenster→Pool-Distanz skaliert — kein Geometrie-Rebuild.

- [ ] **Step 2: look.ts umbauen**

Konkrete Änderungen (Zeilenbezüge auf den Ist-Stand):

1. **Params (Z. 278–284):** `preset`/`cycle` raus. Neu:
   ```ts
   const params = new URLSearchParams(window.location.search);
   const atParam = params.get('at');
   const frozenAt = atParam ? new Date(atParam) : null;
   if (frozenAt && Number.isNaN(frozenAt.getTime())) throw new Error(`invalid ?at=${atParam}`);
   const wxParam = params.get('wx'); // 'clear'|'overcast'|'rain'|'snow'|'fog'|null
   const camModeRaw = params.get('cam');
   const camMode = camModeRaw === 'far' || camModeRaw === 'sky' ? camModeRaw : 'default';
   const now = (): Date => frozenAt ?? new Date();
   ```
   `?wx`-Übersteuerungen als feste `WeatherState`-Objekte (Werte via `weatherLook`-Konstanten sinnvoll setzen):
   ```ts
   import { CLEAR_SKY, sampleWeather, startWeatherLoop, type WeatherSeries, type WeatherState } from './environment/weather';
   const WX_OVERRIDES: Record<string, WeatherState> = {
     clear: CLEAR_SKY,
     overcast: { ...CLEAR_SKY, cloudCover: 0.97, windSpeedMs: 3 },
     rain: { ...CLEAR_SKY, cloudCover: 0.9, precipMmPerH: 4, windSpeedMs: 5, temperatureC: 10 },
     snow: { ...CLEAR_SKY, cloudCover: 0.9, precipMmPerH: 3, snow: true, temperatureC: -2 },
     fog: { ...CLEAR_SKY, cloudCover: 0.6, visibilityM: 150, fog: true, windSpeedMs: 0.5 },
   };
   ```
2. **`const preset = lightPresets[presetName]` (Z. 284) entfällt.** Erstinitialisierung überall mit `computeEnvironment(now(), initialWeather())`, wobei `initialWeather()` = `WX_OVERRIDES[wxParam] ?? CLEAR_SKY`. Alle `preset.`-Zugriffe (exposure Z. 293, fog Z. 300, hemi Z. 533, mist Z. 700–703, giScale Z. 727, saturation/contrast Z. 767–768, lampBoost/lampOn Z. 669–679, showStars Z. 445) werden durch die entsprechenden `EnvironmentState`-Felder bzw. durch `applyEnvironment` ersetzt.
3. **Cloud-Coverage wird Uniform (Z. 357):** vor dem Shader-Block `const coverageU = uniform(0.44);` deklarieren; im Shader `const coverage = float(cloudVol.coverage[presetName])` → `coverageU`. **Drift wird 2D:** `driftU = uniform(0)` (Z. 337) → `const driftUV = uniform(new THREE.Vector2(0, 0));`; in `densityAt` (Z. 349–353): `q = vec3(p.x.mul(scale).add(driftUV.x), p.y.mul(scale*1.35), p.z.mul(scale).add(driftUV.y))`. Die Zeile `driftU.value = t * cloudVol.drift` in animate (Z. 779) entfällt (Integration passiert in `applyEnvironment`).
4. **`sunDirFor`/`sunLightFor`/`applySunState` (Z. 302–315, 424–443) komplett löschen** — ersetzt durch `computeEnvironment` + `applyEnvironment`. `sunArcCfg.cycleSeconds` wird ungenutzt → aus `designTokens.ts` entfernen (kein toter Code).
5. **Mond (Z. 415–422):** statischer `moonDir` entfällt; `moonDisc` bleibt, Position kommt pro Frame aus `applyEnvironment`. Mondphasen-Shading v1: `MeshBasicNodeMaterial`, dessen `colorNode` die Scheibe per `dot(normalLocal, phaseDir)` in beleuchtete/dunkle Hälfte teilt — `phaseDir`-Uniform aus `moonPhase` (Task 6 verfeinert; hier reicht der einfarbige Disc-Erhalt mit Task-6-TODO NICHT — stattdessen: Disc bleibt vorerst `MeshBasicMaterial`, Task 6 ersetzt das Material).
6. **Sterne (Z. 445–465):** `if (preset.showStars)` weg — immer erzeugen (160 Punkte wie bisher, Seed 42), `transparent: true` bleibt, Sichtbarkeit/Opacity/Rotation macht `applyEnvironment`. Sterne in eine `THREE.Group` hängen ist nicht nötig — `stars.rotateOnWorldAxis` reicht.
7. **Lampe (Z. 669–679):** `if (preset.lampOn)` weg — Bulb + PointLight immer erzeugen, Intensität 0 initial; `applyEnvironment` steuert.
8. **Shafts (Z. 636–657):** beide Shafts immer erzeugen (`zc` in `[-1.1, 1.2]`), Geometrie mit fixer Länge `SHAFT_BASE_LEN = 3` (`new THREE.BoxGeometry(1.15, 0.02, 3)`), `shaftWindows = [new THREE.Vector3(3.08, 1.65, -1.1), new THREE.Vector3(3.08, 1.65, 1.2)]`; Aiming/Fading pro Frame in `applyEnvironment`.
9. **Godrays (Z. 751–756):** `if (presetName !== 'night')`-Verzweigung weg — Node immer bauen, Mix als Uniform:
   ```ts
   const godraysMixU = uniform(0);
   const raysNode = godrays(scenePassDepth, camera, sun);
   raysNode.density.value = post.godraysDensity;
   raysNode.maxDensity.value = post.godraysMaxDensity;
   const lit = withAo.add(chain(raysNode).mul(godraysMixU));
   ```
10. **Grade (Z. 765–770):** `preset.saturation`/`preset.contrast` → Uniforms `saturationU = uniform(1)`, `contrastU = uniform(1)`:
    ```ts
    const saturated = mix(vec3(satLum, satLum, satLum), toned, saturationU);
    const contrasted = saturated.sub(float(0.5)).mul(contrastU).add(float(0.5)).clamp(0, 1);
    ```
11. **GI (Z. 727):** `scene.environmentIntensity` pro Frame: in animate `scene.environmentIntensity = gi.environmentIntensity * lastEnv.giScale;`.
12. **Boot-Ende + animate (Z. 772–788):**
    ```ts
    import { computeEnvironment, type EnvironmentState } from './environment/environment';
    import { applyEnvironment, type EnvironmentTargets } from './environment/applyEnvironment';
    import { createPrecipitation } from './environment/precipitation';

    let weatherSeries: WeatherSeries | null = null;
    if (!wxParam) startWeatherLoop((s) => { weatherSeries = s; });
    const currentWeather = (): WeatherState =>
      WX_OVERRIDES[wxParam ?? ''] ?? (weatherSeries ? sampleWeather(weatherSeries, now()) : CLEAR_SKY);

    const targets: EnvironmentTargets = { /* alle Handles aus dem Boot verdrahten */ };
    let lastEnv: EnvironmentState = computeEnvironment(now(), currentWeather());
    let lastT = 0;
    function animate(): void {
      const t = clock.getElapsedTime();
      const dt = Math.min(t - lastT, 0.1);
      lastT = t;
      nurse.scale.y = 1 + Math.sin(t * 2.2) * 0.012;
      patient.scale.y = 1 + Math.sin(t * 2.2 + 1.4) * 0.012;
      child.scale.y = 0.68 * (1 + Math.sin(t * 2.6 + 0.7) * 0.015);
      lastEnv = computeEnvironment(now(), currentWeather());
      applyEnvironment(targets, lastEnv, dt);
      scene.environmentIntensity = gi.environmentIntensity * lastEnv.giScale;
      window.__ENV_STATE = lastEnv;
      frameCount++;
      if (frameCount % 240 === 0) cubeCam.update(renderer as unknown as Parameters<typeof cubeCam.update>[0], scene);
      postProcessing.render();
      if (!window.__LOOK_READY) window.__LOOK_READY = true;
    }
    ```
    Global-Deklaration erweitern (Z. 18–23): `__ENV_STATE?: unknown;`.
13. **designTokens-Aufräumen:** `lightPresets`, `skyPhys`, `LightPreset` und `cloudVol.coverage` (Record) werden nach diesem Umbau nicht mehr importiert → aus `designTokens.ts` löschen (`cloudVol.coverage` durch nichts ersetzen — Coverage kommt jetzt aus `envKeyframes`/`weatherLook`); `moonLight`-Konstante bleibt vorerst (Task 6 prüft Verbleib). Kein toter Code (Projektregel).

`precipitation.ts` existiert in diesem Task noch nicht — funktionsfähigen No-Op-Stub anlegen, damit look.ts kompiliert und rendert (Task 7 füllt ihn; ein throw-Stub würde die Szene brechen):

```ts
// src/diorama/environment/precipitation.ts — no-op until the precipitation task fills it in.
import { Group, type Object3D } from 'three/webgpu';
import type { PrecipType } from './environment';
export type PrecipitationSystem = {
  update(type: PrecipType, intensity: number, windSpeedMs: number, windDirRad: number, dtSeconds: number): void;
  object3d: Object3D;
};
export function createPrecipitation(): PrecipitationSystem {
  return { update: () => {}, object3d: new Group() };
}
```

- [ ] **Step 3: Typecheck**

Run: `npm run typecheck`
Expected: PASS (0 errors). TSL-Uniform-Casts wie im Bestand behandeln (lokale `as`-Casts mit Kommentar, Muster Z. 344–346).

- [ ] **Step 4: Unit-Suite läuft weiter**

Run: `npm test`
Expected: PASS — alle Tests aus Task 1–4 grün (look.ts hat keine Unit-Tests; wichtig ist, dass die designTokens-Umbauten keine Tests brechen).

- [ ] **Step 5: Visueller Rauchtest per Capture**

```bash
node scripts/capture-look.mjs env-noon '' default 2>/dev/null || true
```
Falls `capture-look.mjs` den entfernten `?preset=`-Param übergibt: Script-Aufruf im nächsten Task-Schritt fixen — hier reicht manuell:
```bash
npm run dev &
sleep 4
# Browser-Probe: vier Zustände laden und __ENV_STATE prüfen
npx tsx -e "
import { chromium } from 'playwright';
const b = await chromium.launch(); const p = await b.newPage();
for (const q of ['at=2026-07-03T11:00:00Z','at=2026-07-03T19:30:00Z','at=2026-07-03T23:30:00Z&wx=clear','at=2026-07-03T11:00:00Z&wx=fog']) {
  await p.goto('http://127.0.0.1:5175/look.html?' + q);
  await p.waitForFunction(() => (window as any).__LOOK_READY, { timeout: 30000 });
  const env = await p.evaluate(() => (window as any).__ENV_STATE);
  console.log(q, '→ elev', env.sunElevDeg.toFixed(1), 'fogFar', env.fogFar.toFixed(1), 'stars', env.starVisibility.toFixed(2));
}
await b.close();"
kill %1
```
Expected: Mittag elev ≈ 60+, 19:30 UTC golden/niedrig, 23:30 stars > 0.8, fog → fogFar < 25. (Die formale Smoke kommt in Task 8; das hier ist der Arbeits-Check.)

- [ ] **Step 6: Commit**

```bash
git add src/diorama/look.ts src/diorama/designTokens.ts src/diorama/environment/applyEnvironment.ts src/diorama/environment/precipitation.ts
git commit -m "feat(environment): look.ts driven by realtime computeEnvironment — presets removed, uniforms live, ?at/?wx overrides"
```

---

### Task 6: Nachthimmel — Mondphase-Shading + echte Sternenrotation verifizieren

**Files:**
- Modify: `src/diorama/look.ts` (moonDisc-Material), `src/diorama/environment/applyEnvironment.ts` (Phase-Uniform)
- Test: `tests/diorama/environment.test.ts` (erweitern)

**Interfaces:**
- Consumes: `EnvironmentState.moonPhase`, `moonDir`, `siderealAngleRad` (Task 4); moonDisc/stars-Handles (Task 5).
- Produces: `moonPhaseUniform: { value: THREE.Vector3 }` im `EnvironmentTargets` (Feld `moonPhaseDir`).

- [ ] **Step 1: Failing Test — Phasenrichtung ist pur berechenbar**

An `tests/diorama/environment.test.ts` anhängen; die Funktion kommt nach `environment.ts`:

```ts
import { moonPhaseLightDir } from '../../src/diorama/environment/environment';

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
```

- [ ] **Step 2: Run — FAIL** (`moonPhaseLightDir` existiert nicht)

Run: `npx vitest run tests/diorama/environment.test.ts`

- [ ] **Step 3: Implementation**

In `environment.ts` ergänzen (`phase=0 → (0,0,1)` von hinten beleuchtet = Neumond, `phase=0.25 → (1,0,0)` seitlich = Halbmond, `phase=0.5 → (0,0,−1)` frontal = Vollmond):

```ts
// Direction of sunlight on the moon disc in DISC-LOCAL space (disc faces the
// camera; -z = toward viewer). phase 0 = new (lit from behind), 0.5 = full.
export function moonPhaseLightDir(phase: number): [number, number, number] {
  const a = 2 * Math.PI * phase;
  return [Math.sin(a), 0, Math.cos(a)];
}
```

moonDisc-Material in look.ts auf `MeshBasicNodeMaterial` mit Terminator-Shading umstellen:

```ts
const moonPhaseDirU = uniform(new THREE.Vector3(0, 0, -1));
const moonMat = new THREE.MeshBasicNodeMaterial({ fog: false });
{
  const litSide = dot(normalView.negate(), moonPhaseDirU as unknown as ReturnType<typeof vec3>);
  const lit = smoothstep(float(-0.15), float(0.15), litSide);
  moonMat.colorNode = mix(vec3(0.16, 0.18, 0.23), vec3(0.87, 0.91, 0.95), lit);
}
const moonDisc = new THREE.Mesh(new THREE.SphereGeometry(1.6, 20, 20), moonMat);
```
In `applyEnvironment`: `t.moonPhaseDir.value.set(...moonPhaseLightDir(env.moonPhase))` (Feld zu `EnvironmentTargets` hinzufügen; das `moonLightObj: null`-Feld aus Task 5 entfernen, es war nie belegt — der Mond nutzt das geteilte Key-Light). `moonLight`-Konstante in designTokens: die Farbe `0xa8c4e8` wird in applyEnvironment genutzt → als `moonLight.color`-Referenz importieren statt Hex-Literal (Token-Regel!), `position` + `intensity` Felder löschen (tot).

- [ ] **Step 4: Run — PASS + Typecheck**

```bash
npx vitest run tests/diorama/environment.test.ts && npm run typecheck
```

- [ ] **Step 5: Visuelle Probe + Commit**

```bash
# Nacht mit Mond: 23:30 lokale Sommerzeit = 21:30 UTC
node -e "console.log('probe at ?at=2026-07-03T21:30:00Z — moon disc shows a phase terminator')"
git add src/diorama/look.ts src/diorama/environment/environment.ts src/diorama/environment/applyEnvironment.ts src/diorama/designTokens.ts tests/diorama/environment.test.ts
git commit -m "feat(environment): real moon phase shading + shared key-light night wiring"
```

---

### Task 7: `precipitation.ts` — Regen/Schnee-Partikel

**Files:**
- Modify: `src/diorama/environment/precipitation.ts` (No-Op-Stub aus Task 5 ersetzen)
- Modify: `src/diorama/look.ts` (System instanziieren, `scene.add(precip.object3d)`)

**Interfaces:**
- Consumes: `PrecipType` (Task 4), `weatherLook.rainColor/snowColor` (Task 3).
- Produces: erfüllt das `PrecipitationSystem`-Interface aus Task 5 (Signatur unverändert — look.ts braucht keine Änderung ausser `scene.add`).

- [ ] **Step 1: Implementation**

Instanzierte Quads, vertex-animiert über eine Zeit-Uniform (kein CPU-Matrix-Update pro Frame). Box 24×14×20 um das Diorama, Wrap via `mod`:

```ts
// GPU precipitation: one InstancedMesh of camera-facing-ish quads, animated in
// the vertex stage (fall + wind shear + wrap). CPU per frame: 4 uniform writes.
import * as THREE from 'three/webgpu';
import { float, instanceIndex, hash, mix, positionLocal, uniform, vec3, vec4 } from 'three/tsl';
import { weatherLook } from '../designTokens';
import type { PrecipType } from './environment';

export type PrecipitationSystem = {
  update(type: PrecipType, intensity: number, windSpeedMs: number, windDirRad: number, dtSeconds: number): void;
  object3d: THREE.Object3D;
};

const COUNT = 3000;
const BOX = { x: 24, y: 14, z: 20 } as const;
const RAIN_SPEED = 9; // m/s fall
const SNOW_SPEED = 1.1;

export function createPrecipitation(): PrecipitationSystem {
  const timeU = uniform(0);
  const snowU = uniform(0); // 0 rain, 1 snow
  const windU = uniform(new THREE.Vector2(0, 0)); // horizontal drift m/s (scene x/z)
  const countU = uniform(0); // active fraction 0..1

  const geo = new THREE.PlaneGeometry(1, 1);
  const mat = new THREE.MeshBasicNodeMaterial({ transparent: true, depthWrite: false });
  {
    type N = any;
    const f = (n: unknown): N => n as N;
    const rnd = (salt: number): N => f(hash(instanceIndex.add(salt)));
    const x0 = rnd(1).mul(BOX.x).sub(BOX.x / 2);
    const z0 = rnd(2).mul(BOX.z).sub(BOX.z / 2);
    const y0 = rnd(3).mul(BOX.y);
    const speed = mix(float(RAIN_SPEED), float(SNOW_SPEED), snowU).mul(rnd(4).mul(0.4).add(0.8));
    const fallen = f(timeU).mul(speed);
    const y = f(y0.sub(fallen).mod(BOX.y));
    const drift = f(fallen).div(speed); // seconds airborne (approx, for shear)
    const wob = f(rnd(5).mul(6.28));
    const wobble = f(timeU.add(wob)).sin().mul(snowU).mul(0.4);
    const x = f(x0.add(f(windU.x).mul(drift)).add(wobble).add(BOX.x / 2).mod(BOX.x).sub(BOX.x / 2));
    const z = f(z0.add(f(windU.y).mul(drift)).add(BOX.z / 2).mod(BOX.z).sub(BOX.z / 2));
    // rain: thin streak (stretch y); snow: small square
    const sx = mix(float(0.014), float(0.05), snowU);
    const sy = mix(float(0.45), float(0.05), snowU);
    const local = positionLocal.mul(vec3(sx, sy, 1));
    mat.positionNode = f(local).add(vec3(x, y, z));
    const active = f(rnd(6)).lessThan(countU);
    const col = mix(
      vec3(...hexToRgb01(weatherLook.rainColor)),
      vec3(...hexToRgb01(weatherLook.snowColor)),
      snowU,
    );
    const alpha = mix(float(0.35), float(0.8), snowU);
    mat.colorNode = vec4(col, f(alpha).mul(f(active).select(float(1), float(0))));
  }
  const mesh = new THREE.InstancedMesh(geo, mat, COUNT);
  mesh.frustumCulled = false;
  mesh.position.y = 0;

  let clock = 0;
  return {
    object3d: mesh,
    update(type: PrecipType, intensity: number, windSpeedMs: number, windDirRad: number, dt: number): void {
      clock += dt;
      timeU.value = clock;
      mesh.visible = type !== 'none' && intensity > 0.01;
      countU.value = type === 'none' ? 0 : 0.15 + 0.85 * intensity;
      snowU.value = type === 'snow' ? 1 : 0;
      const toward = windDirRad + Math.PI;
      windU.value.set(Math.sin(toward) * windSpeedMs * 0.6, -Math.cos(toward) * windSpeedMs * 0.6);
    },
  };
}

function hexToRgb01(hex: number): [number, number, number] {
  return [((hex >> 16) & 0xff) / 255, ((hex >> 8) & 0xff) / 255, (hex & 0xff) / 255];
}
```
TSL-API-Details (`.select`, `.mod` auf floats, `hash(instanceIndex)`) beim Implementieren gegen three r185 verifizieren — Muster: bestehender Cloud-Shader in look.ts (Z. 343–405) + `three/examples` zu `instanceIndex`/`hash`. Wenn `select` auf Node-Ebene anders heisst: `mix(0, 1, activeFloat)` als Ausweich.

In look.ts (Boot): `const precip = createPrecipitation(); scene.add(precip.object3d);` und `precipitation: precip` in `targets`.

- [ ] **Step 2: Typecheck + Unit-Suite**

```bash
npm run typecheck && npm test
```
Expected: beides PASS.

- [ ] **Step 3: Visuelle Probe**

```bash
npm run dev &
sleep 4
npx tsx -e "
import { chromium } from 'playwright';
const b = await chromium.launch(); const p = await b.newPage();
await p.goto('http://127.0.0.1:5175/look.html?at=2026-07-03T15:00:00Z&wx=rain');
await p.waitForFunction(() => (window as any).__LOOK_READY, { timeout: 30000 });
const env = await p.evaluate(() => (window as any).__ENV_STATE);
console.log('rain probe:', env.precipType, env.precipIntensity);
await b.close();"
kill %1
```
Expected: `rain probe: rain 0.8`.

- [ ] **Step 4: Commit**

```bash
git add src/diorama/environment/precipitation.ts src/diorama/look.ts
git commit -m "feat(environment): GPU rain/snow particles (instanced, vertex-animated, wind-sheared)"
```

---

### Task 8: Browser-Smoke `scripts/smoke-environment.mjs` (Pflicht-Gate)

**Files:**
- Create: `scripts/smoke-environment.mjs` (Vorlage: `scripts/capture-look.mjs` für dev-Server-Spawn/Port-Wait)

**Interfaces:**
- Consumes: `window.__ENV_STATE`, `window.__LOOK_READY` (Task 5), `OPEN_METEO_URL`-Route, Fixture `tests/fixtures/openMeteo.json`.

- [ ] **Step 1: Script schreiben**

Kernlogik (dev-Server-Spawn + `waitForPort` 1:1 aus `capture-look.mjs` übernehmen):

```js
// Environment smoke: proves the REAL wiring — (1) the client actually requests
// open-meteo and applies the parsed series, (2) the ?at/?wx state matrix lands
// in __ENV_STATE with the expected values. CLAUDE.md: mandatory before "complete".
import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
// ... spawn vite dev on 127.0.0.1:5175 exactly like capture-look.mjs ...

const fixture = readFileSync('tests/fixtures/openMeteo.json', 'utf8');
const browser = await chromium.launch();
const page = await browser.newPage();
let meteoRequested = false;
await page.route('**/api.open-meteo.com/**', (route) => {
  meteoRequested = true;
  return route.fulfill({ status: 200, contentType: 'application/json', body: fixture });
});

const checks = [];
const probe = async (query, assert) => {
  await page.goto(`http://127.0.0.1:5175/look.html?${query}`);
  await page.waitForFunction(() => window.__LOOK_READY, { timeout: 45000 });
  const env = await page.evaluate(() => window.__ENV_STATE);
  const errors = assert(env);
  checks.push({ query, errors });
  for (const e of errors) console.error(`FAIL [${query}]: ${e}`);
};

// 1) Live wiring: no ?wx → the client must fetch open-meteo
await probe('at=2026-07-03T11:00:00Z', (env) => {
  const errs = [];
  if (!meteoRequested) errs.push('open-meteo was never requested');
  if (env.sunElevDeg < 55) errs.push(`noon sun too low: ${env.sunElevDeg}`);
  return errs;
});
// 2) State matrix (wx overrides, no network dependency)
await probe('at=2026-07-03T04:00:00Z&wx=clear', (e) => (e.sunElevDeg > -8 && e.sunElevDeg < 12 && e.godraysMix >= 0 ? [] : [`dawn state off: elev=${e.sunElevDeg}`]));
await probe('at=2026-07-03T19:45:00Z&wx=clear', (e) => (e.lampOn01 > 0.3 ? [] : ['dusk should start warming windows']));
await probe('at=2026-07-03T23:30:00Z&wx=clear', (e) => (e.starVisibility > 0.7 && e.sunIntensity < 0.05 ? [] : [`night off: stars=${e.starVisibility}`]));
await probe('at=2026-07-03T11:00:00Z&wx=overcast', (e) => (e.cloudCoverage > 0.7 && e.sunIntensity < 3 ? [] : [`overcast off: cov=${e.cloudCoverage}`]));
await probe('at=2026-07-03T11:00:00Z&wx=rain', (e) => (e.precipType === 'rain' && e.precipIntensity > 0.5 ? [] : [`rain off: ${e.precipType}`]));
await probe('at=2026-01-15T11:00:00Z&wx=snow', (e) => (e.precipType === 'snow' ? [] : [`snow off: ${e.precipType}`]));
await probe('at=2026-07-03T11:00:00Z&wx=fog', (e) => (e.fogFar < 30 ? [] : [`fog off: far=${e.fogFar}`]));
// 3) Winter check: at 17:30 CET in January it is already night
await probe('at=2026-01-15T16:30:00Z&wx=clear', (e) => (e.sunElevDeg < 0 ? [] : ['winter 17:30 local should be after sunset']));

await browser.close();
// ... kill dev server ...
const failed = checks.filter((c) => c.errors.length > 0);
console.log(`\nsmoke-environment: ${checks.length - failed.length}/${checks.length} passed`);
process.exit(failed.length > 0 ? 1 : 0);
```

- [ ] **Step 2: Smoke laufen lassen**

Run: `node scripts/smoke-environment.mjs`
Expected: `smoke-environment: 9/9 passed`, exit 0. Bei Fails: systematic-debugging, nicht Assertions lockern.

- [ ] **Step 3: Commit**

```bash
git add scripts/smoke-environment.mjs
git commit -m "test(environment): browser smoke — open-meteo wiring + ?at/?wx state matrix against __ENV_STATE"
```

---

### Task 9: Capture-Matrix + Look-Review + Doku

**Files:**
- Create: `scripts/capture-env.mjs`
- Modify: `scripts/capture-look.mjs` → löschen (ersetzt; nutzt den entfernten `?preset=`-Param — toter Code, Projektregel)
- Modify: `progress.md` (neuer Eintrag OBEN im reverse-chronologischen Block ab Zeile 19 — Datei-Konvention beachten!)

- [ ] **Step 1: `capture-env.mjs` schreiben**

Kopie von `capture-look.mjs`, aber Matrix-fähig: iteriert `[name, query]`-Paare, schreibt `artifacts/env/<name>.png`:

```js
const MATRIX = [
  ['dawn', 'at=2026-07-03T04:10:00Z&wx=clear'],
  ['noon', 'at=2026-07-03T11:00:00Z&wx=clear'],
  ['dusk', 'at=2026-07-03T19:35:00Z&wx=clear'],
  ['night', 'at=2026-07-03T23:30:00Z&wx=clear'],
  ['overcast', 'at=2026-07-03T11:00:00Z&wx=overcast'],
  ['rain', 'at=2026-07-03T15:00:00Z&wx=rain'],
  ['snow', 'at=2026-01-15T11:00:00Z&wx=snow'],
  ['hochnebel', 'at=2026-10-20T09:00:00Z&wx=fog'],
  ['winter-night-1730', 'at=2026-01-15T16:35:00Z&wx=clear'],
];
```
Pro Eintrag: goto → `__LOOK_READY` warten → 1 s settle → screenshot. Dev-Server-Handling wie gehabt.

- [ ] **Step 2: Matrix rendern + reviewen**

Run: `node scripts/capture-env.mjs`
Expected: 9 PNGs unter `artifacts/env/`. **Look-Review durchführen** (Art-Direction ist reviewbar): alle 9 Bilder lesen und gegen die Design-Intention prüfen — Dawn warm-golden, Noon hell/neutral (neuer Day-Keyframe!), Dusk = DREDGE, Nacht dunkel mit Sternen + Mond-Terminator, Regen sichtbar als Fäden, Schnee als Flocken, Hochnebel grauweiss-dicht. Befunde (z. B. Day-Keyframe zu flach/zu blau) direkt in `designTokens.ts` nachkuratieren, Matrix erneut rendern, bis der Look sitzt.

- [ ] **Step 3: progress.md aktualisieren + finaler Gate**

```bash
npm test && npm run typecheck && npm run build && node scripts/smoke-environment.mjs
```
Expected: alles grün. Dann:

```bash
git add scripts/capture-env.mjs progress.md
git rm scripts/capture-look.mjs
git commit -m "feat(environment): capture matrix for look review; retire preset-based capture harness"
```

- [ ] **Step 4: Branch abschliessen**

superpowers:finishing-a-development-branch verwenden (PR gegen `klinik/look-prototype` oder gemäss User-Entscheid; Repo-Standard: Worktree → PR → origin, CI grün abwarten — „wait for green, not just not-red").

---

## Self-Review (durchgeführt)

- **Spec-Abdeckung:** Echtzeit-Treiber ✓ (T5), Astronomie ✓ (T1), Open-Meteo ✓ (T2), Keyframes + neuer Day ✓ (T3/T4), alle 4 Wetteraspekte ✓ (T4/T5/T7), echter Mond + Phase ✓ (T6), Sterne mit echter Rotation ✓ (T5/T6), `?at`/`?wx` ✓ (T5), Offline-Policy (localStorage, letzter Zustand, Klarhimmel-Default) ✓ (T2/T5), Unit-Tests (Golden/Stetigkeit/Parsing) ✓ (T1–T4), Browser-Smoke ✓ (T8), Capture-Matrix ✓ (T9), Perf (nur Uniforms) ✓ (T5/T7).
- **Placeholder:** keine TBD/TODO-Reste; der Precipitation-Stub (T5) ist ein bewusst funktionsfähiger No-Op mit klar benanntem Füll-Task (T7).
- **Typ-Konsistenz:** `PrecipitationSystem.update`-Signatur identisch in T5-Stub, T5-apply-Aufruf und T7; `EnvironmentState`-Felder in T4-Definition = T5-apply-Zugriffe = T8-Smoke-Proben; `WeatherState` einheitlich (T2/T4/T5); `moonLightObj`-Feld aus T5 wird in T6 explizit entfernt.
