// src/diorama/ksw/geo/roads.ts
// OSM ways as flat clay ribbons v2: continuous miter-joined strips (no wedge
// gaps, no overlapping quads), one visual layer per class — carriageways,
// footpaths, rail on its ballast band — each on its own height so junctions
// never flicker.
import * as THREE from 'three/webgpu';
import { kswCity } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';

/** Optional per-vertex ground draping. `groundYAt(x,z)` returns the visible
 * (shifted) terrain height at a world point; the ribbon vertex y becomes that
 * plus the constant layer offset `y`. When omitted the ribbon is flat at `y`,
 * reproducing the pre-#119 single-plate look (still used near the anchor, where
 * the drape is ~0, and by any caller without a world). */
export type GroundYAt = (x: number, z: number) => number;

// Terrain-follow subdivision (Fix 1). The DEM/terrain mesh tessellates at
// ~1.25 m (finest pyramid level) while raw road polyline segments span up to
// ~200 m. A straight 3-D chord between two draped vertices dives under any
// convex terrain bump between them → the road is buried. We resample each
// segment to ≤ SUBDIVIDE_M so every ribbon vertex sits on the DEM and the
// ribbon follows the surface within one terrain-cell error. ADAPTIVE_DEV keeps
// the vertex/tri count sane: a subdivided point is only inserted where the
// terrain deviates from the segment's straight chord by more than this (so flat
// stretches stay coarse; only humps get densified).
const SUBDIVIDE_M = 2.5;
const ADAPTIVE_DEV = 0.05; // m of chord-vs-terrain error that triggers a split

/** Resample a centreline so no draped segment hides the terrain under its
 * chord. Recursively bisects a segment while the terrain at its midpoint
 * departs from the straight chord midpoint by > ADAPTIVE_DEV, down to a
 * SUBDIVIDE_M floor. Returns the original pts untouched when no sampler is
 * given (flat pre-#119 look, and the unit-tested geometry). */
export function subdivideForDrape(pts: number[][], groundYAt: GroundYAt): number[][] {
  if (pts.length < 2) return pts;
  const out: number[][] = [pts[0]];
  const emit = (
    ax: number,
    az: number,
    ay: number,
    bx: number,
    bz: number,
    by: number,
    depth: number,
  ): void => {
    const len = Math.hypot(bx - ax, bz - az);
    const mx = (ax + bx) / 2;
    const mz = (az + bz) / 2;
    const chordMidY = (ay + by) / 2;
    const terrainMidY = groundYAt(mx, mz);
    const needsSplit =
      depth < 12 && len > SUBDIVIDE_M && Math.abs(terrainMidY - chordMidY) > ADAPTIVE_DEV;
    if (needsSplit) {
      emit(ax, az, ay, mx, mz, terrainMidY, depth + 1);
      emit(mx, mz, terrainMidY, bx, bz, by, depth + 1);
    } else {
      out.push([bx, bz]);
    }
  };
  for (let i = 1; i < pts.length; i++) {
    const [ax, az] = pts[i - 1];
    const [bx, bz] = pts[i];
    emit(ax, az, groundYAt(ax, az), bx, bz, groundYAt(bx, bz), 0);
  }
  return out;
}

export function miterStrip(
  pts: number[][],
  width: number,
  y: number,
  groundYAt?: GroundYAt,
): { positions: number[]; indices: number[] } {
  const positions: number[] = [];
  const indices: number[] = [];
  const half = width / 2;
  // Fix 1: when draping, resample the centreline to terrain resolution so the
  // ribbon follows the DEM instead of chording under convex bumps. No sampler
  // → untouched pts (flat ribbon, matches the pre-#119 look and the unit tests).
  if (groundYAt) pts = subdivideForDrape(pts, groundYAt);
  const n = pts.length;
  if (n < 2) return { positions, indices };
  for (let i = 0; i < n; i++) {
    const [px, pz] = pts[Math.max(0, i - 1)];
    const [cx, cz] = pts[i];
    const [nx2, nz2] = pts[Math.min(n - 1, i + 1)];
    let dx0 = cx - px;
    let dz0 = cz - pz;
    let dx1 = nx2 - cx;
    let dz1 = nz2 - cz;
    const l0 = Math.hypot(dx0, dz0) || 1;
    const l1 = Math.hypot(dx1, dz1) || 1;
    dx0 /= l0; dz0 /= l0; dx1 /= l1; dz1 /= l1;
    // averaged tangent → miter normal; scale = 1/cos(θ/2), capped at 60° kink
    const tx = dx0 + dx1;
    const tz = dz0 + dz1;
    const tl = Math.hypot(tx, tz);
    let mx: number;
    let mz: number;
    let scale = 1;
    if (tl < 1e-6) {
      mx = -dz0; mz = dx0; // 180° hairpin: fall back to segment normal
    } else {
      mx = -tz / tl; mz = tx / tl;
      const cosHalf = Math.max(0.5, mx * -dz0 + mz * dx0); // cap: ≤ 2× width spike
      scale = 1 / cosHalf;
    }
    // Drape each edge vertex onto the terrain (sampled at the centreline point
    // so both rails of the ribbon share one height and the strip stays planar
    // across its width — avoids a twisted ribbon on cross-slopes).
    const gy = groundYAt ? groundYAt(cx, cz) + y : y;
    positions.push(cx + mx * half * scale, gy, cz + mz * half * scale, cx - mx * half * scale, gy, cz - mz * half * scale);
    if (i > 0) {
      const a = (i - 1) * 2;
      indices.push(a, a + 2, a + 1, a + 1, a + 2, a + 3);
    }
  }
  return { positions, indices };
}

function stripsMesh(
  name: string,
  paths: RoadPath[],
  widthOf: (p: RoadPath) => number,
  color: number,
  y: number,
  groundYAt?: GroundYAt,
): THREE.Mesh {
  const positions: number[] = [];
  const indices: number[] = [];
  for (const p of paths) {
    const s = miterStrip(p.pts, widthOf(p), y, groundYAt);
    const base = positions.length / 3;
    positions.push(...s.positions);
    for (const i of s.indices) indices.push(base + i);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  const mesh = new THREE.Mesh(geo, clayMat(color));
  mesh.name = name;
  mesh.receiveShadow = true;
  mesh.castShadow = false;
  return mesh;
}

const FOOT = new Set(['footway', 'path', 'cycleway', 'steps', 'pedestrian', 'track']);

export function buildRoads(roads: RoadPath[], rails: RoadPath[], groundYAt?: GroundYAt): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityRoads';
  const carriage = roads.filter((r) => !FOOT.has(r.class));
  const foot = roads.filter((r) => FOOT.has(r.class));
  group.add(stripsMesh('carriageRibbons', carriage, (p) => p.width, kswCity.roadColors.carriage, kswCity.roadYs.carriage, groundYAt));
  group.add(stripsMesh('footwayRibbons', foot, (p) => p.width, kswCity.roadColors.footway, kswCity.roadYs.footway, groundYAt));
  group.add(stripsMesh('railBeds', rails, (p) => p.width + 2.2, kswCity.roadColors.railBed, kswCity.roadYs.railBed, groundYAt));
  group.add(stripsMesh('railRibbons', rails, (p) => p.width, kswCity.roadColors.rail, kswCity.roadYs.rail, groundYAt));
  return group;
}
