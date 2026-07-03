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

// Direction of sunlight on the moon disc in DISC-LOCAL space (disc faces the
// camera; -z = toward viewer). phase 0 = new (lit from behind), 0.5 = full.
export function moonPhaseLightDir(phase: number): [number, number, number] {
  const a = 2 * Math.PI * phase;
  return [Math.sin(a), 0, Math.cos(a)];
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
