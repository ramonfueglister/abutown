// Real astronomy for the diorama — sun/moon position over Winterthur, computed
// from UTC via suncalc (Meeus-based). Pure: no three.js, no wall-clock reads.
//
// Scene coordinate convention (BINDING for the whole diorama):
//   +x = east, +z = south, +y = up.
// The room's east wall (x+) is the morning-sun side, matching look.ts.
//
// suncalc azimuth is measured from SOUTH, positive westward. We convert to
// azimuth-from-north (clockwise via east) before mapping into scene space.

import * as SunCalc from 'suncalc';

export const WINTERTHUR = { lat: 47.499, lon: 8.724 } as const;

export type Vec3Tuple = [number, number, number];
export type SunState = { dir: Vec3Tuple; elevDeg: number; azimuthDeg: number; rising: boolean };
export type MoonState = { dir: Vec3Tuple; elevDeg: number; phase: number; illumination: number };

const RAD = Math.PI / 180;

export function sceneDirFromAzEl(azFromNorthRad: number, elevRad: number): Vec3Tuple {
  const cosE = Math.cos(elevRad);
  return [
    cosE * Math.sin(azFromNorthRad), // east
    Math.sin(elevRad), // up
    -cosE * Math.cos(azFromNorthRad), // -north = south
  ];
}

function azFromNorth(suncalcAzimuthDeg: number): number {
  return suncalcAzimuthDeg * RAD;
}

export function sunState(utc: Date): SunState {
  const pos = SunCalc.getPosition(utc, WINTERTHUR.lat, WINTERTHUR.lon);
  const later = SunCalc.getPosition(new Date(utc.getTime() + 60_000), WINTERTHUR.lat, WINTERTHUR.lon);
  const azN = azFromNorth(pos.azimuth);
  const elevRad = pos.altitude * RAD;
  return {
    dir: sceneDirFromAzEl(azN, elevRad),
    elevDeg: pos.altitude,
    azimuthDeg: ((azN / RAD) % 360 + 360) % 360,
    rising: later.altitude > pos.altitude,
  };
}

export function moonState(utc: Date): MoonState {
  const pos = SunCalc.getMoonPosition(utc, WINTERTHUR.lat, WINTERTHUR.lon);
  const illum = SunCalc.getMoonIllumination(utc);
  const elevRad = pos.altitude * RAD;
  return {
    dir: sceneDirFromAzEl(azFromNorth(pos.azimuth), elevRad),
    elevDeg: pos.altitude,
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
