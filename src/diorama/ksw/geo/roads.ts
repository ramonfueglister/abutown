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

/** One per-point miter offset: the unit miter normal (mx,mz) perpendicular to
 * the averaged tangent, and the `scale` = 1/cos(θ/2) that keeps a mitred corner
 * on the offset ribbon edge (capped at 60° so a hairpin never spikes). */
export interface MiterOffset {
  mx: number;
  mz: number;
  scale: number;
}

/** SHARED miter math for miterStrip AND skirtStrip (Task 5e refactor). Given a
 * centreline `pts`, returns one `{mx, mz, scale}` per point so BOTH the ribbon
 * top surface and its side-skirt use byte-identical edge offsets and can never
 * drift. Pure over `pts`: does NOT subdivide (callers subdivide first with the
 * same groundYAt so the point arrays line up). A single-point path yields one
 * degenerate offset; callers already guard `n < 2`. */
export function miterOffsets(pts: number[][]): MiterOffset[] {
  const n = pts.length;
  const out: MiterOffset[] = [];
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
    out.push({ mx, mz, scale });
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
  const offs = miterOffsets(pts);
  for (let i = 0; i < n; i++) {
    const [cx, cz] = pts[i];
    const { mx, mz, scale } = offs[i];
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

/** Vertical side-skirts along BOTH edges of a ribbon (spec §5 terrain-discard,
 * Task 5e). The terrain shader discards every fragment inside a road/rail
 * corridor, leaving an open hole between the ribbon edge and the terrain the
 * discard removed. The skirt is a pair of vertical apron strips — one down each
 * ribbon edge, from the ribbon-edge top y down to `groundYAt − dropM` (i.e.
 * `profile − 1.5 m`, since inside a corridor groundYAt returns the profile) —
 * that close that hole so you never see through the world under a road.
 *
 * Geometry mirrors miterStrip exactly (same subdivideForDrape, same miter
 * offsets) so the skirt top edge coincides with the ribbon edge with no seam.
 * Per edge, per centreline point: a top vertex at the ribbon edge (draped y +
 * layer offset `y`) and a bottom vertex at `groundYAt(cx,cz) − dropM`. Merged
 * into one geometry per layer in buildRoads — no per-frame cost.
 *
 * Vertex ys span [profile − dropM, profile + y] on a draped corridor (unit
 * tested). With no sampler the ribbon is flat at `y` and the skirt drops to
 * `−dropM` (still a valid apron; only used near the anchor where drape ≈ 0). */
export function skirtStrip(
  pts: number[][],
  width: number,
  y: number,
  groundYAt?: GroundYAt,
  dropM = 1.5,
): { positions: number[]; indices: number[] } {
  const positions: number[] = [];
  const indices: number[] = [];
  const half = width / 2;
  if (groundYAt) pts = subdivideForDrape(pts, groundYAt);
  const n = pts.length;
  if (n < 2) return { positions, indices };
  // SHARED miter offsets (Task 5e refactor): the SAME per-point {mx,mz,scale}
  // miterStrip uses, so the skirt top edge coincides exactly with the ribbon
  // edge and the two can never drift.
  const offs = miterOffsets(pts);
  // Two skirts: side = +1 (left edge) and −1 (right edge). Each is a vertical
  // quad strip: top follows the ribbon edge, bottom is dropM below the ground.
  for (const side of [1, -1]) {
    const base0 = positions.length / 3;
    for (let i = 0; i < n; i++) {
      const [cx, cz] = pts[i];
      const { mx, mz, scale } = offs[i];
      const ground = groundYAt ? groundYAt(cx, cz) : 0;
      const topY = ground + y;
      const botY = ground - dropM;
      const ex = cx + side * mx * half * scale;
      const ez = cz + side * mz * half * scale;
      positions.push(ex, topY, ez, ex, botY, ez); // top then bottom
      if (i > 0) {
        const a = base0 + (i - 1) * 2;
        // one quad per segment (two tris). The apron material is DoubleSide
        // (skirtMat) so it reads from outside the corridor AND from above where
        // the discarded terrain used to be — no doubled geometry needed.
        indices.push(a, a + 1, a + 2, a + 2, a + 1, a + 3);
      }
    }
  }
  return { positions, indices };
}

/** Dedicated ribbon material = the shared clay look PLUS a depth-bias
 * (polygonOffset). Fix 2 belt-and-braces: the layer height ladder
 * (designTokens.roadYs) already separates the coplanar ribbons, but at a
 * rail/road crossing the two layers drape onto independent DEM samples, so we
 * also bias depth per layer (more-negative `units` = wins the depth test /
 * drawn "closer"). Ladder units bottom→top: railBed 0 < carriage −1 <
 * footway −1 < rail −3 so rails always resolve on top of the carriage. We
 * clone rather than mutate `clayMat` because that cache is shared with props. */
function ribbonMat(color: number, polygonOffsetUnits: number): THREE.MeshPhysicalMaterial {
  const m = clayMat(color).clone();
  if (polygonOffsetUnits !== 0) {
    m.polygonOffset = true;
    m.polygonOffsetFactor = -1;
    m.polygonOffsetUnits = polygonOffsetUnits;
  }
  return m;
}

function stripsMesh(
  name: string,
  paths: RoadPath[],
  widthOf: (p: RoadPath, i: number) => number,
  color: number,
  y: number,
  polygonOffsetUnits: number,
  groundYAt?: GroundYAt,
): THREE.Mesh {
  const positions: number[] = [];
  const indices: number[] = [];
  for (let idx = 0; idx < paths.length; idx++) {
    const p = paths[idx];
    const s = miterStrip(p.pts, widthOf(p, idx), y, groundYAt);
    const base = positions.length / 3;
    positions.push(...s.positions);
    for (const i of s.indices) indices.push(base + i);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  const mesh = new THREE.Mesh(geo, ribbonMat(color, polygonOffsetUnits));
  mesh.name = name;
  mesh.receiveShadow = true;
  mesh.castShadow = false;
  return mesh;
}

/** Darken a packed 0xRRGGBB color by a factor (skirt = ribbon color ×0.8). */
function darken(color: number, factor: number): number {
  const r = Math.round(((color >> 16) & 0xff) * factor);
  const g = Math.round(((color >> 8) & 0xff) * factor);
  const b = Math.round((color & 0xff) * factor);
  return (r << 16) | (g << 8) | b;
}

/** Apron material for the skirts: the ribbon clay color darkened ×0.8, rendered
 * DoubleSide so the vertical strip is visible from outside the corridor and
 * from above (where the terrain shader discarded the ground). No polygonOffset
 * — the skirt is genuinely vertical, not coplanar with any ribbon. */
function skirtMat(color: number): THREE.MeshPhysicalMaterial {
  const m = clayMat(darken(color, 0.8)).clone();
  m.side = THREE.DoubleSide;
  return m;
}

/** Build the merged side-skirt mesh for a ribbon layer (spec §5 terrain-discard,
 * Task 5e): every ribbon in `paths` contributes two vertical apron strips down
 * its edges, closing the hole the terrain-discard shader opens under it. */
function skirtsMesh(
  name: string,
  paths: RoadPath[],
  widthOf: (p: RoadPath, i: number) => number,
  color: number,
  y: number,
  groundYAt?: GroundYAt,
): THREE.Mesh {
  const positions: number[] = [];
  const indices: number[] = [];
  for (let idx = 0; idx < paths.length; idx++) {
    const p = paths[idx];
    const s = skirtStrip(p.pts, widthOf(p, idx), y, groundYAt);
    const base = positions.length / 3;
    positions.push(...s.positions);
    for (const i of s.indices) indices.push(base + i);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  const mesh = new THREE.Mesh(geo, skirtMat(color));
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
  // Ribbons draw at their real OSM width. The traffic kernel bakes
  // width-aware lane offsets (trafficnet.mjs :: laneOffsets), so cars fit the
  // real tarmac by construction — the former FIX-D1 width floor is gone: it
  // widened the world to match a 3.0 m-lane kernel assumption and swallowed
  // street-tree verges / grazed facades.
  // polygonOffset ladder (units) matches the roadYs height ladder bottom→top:
  // railBed 0 < carriage/footway −1 < rail −3 (more negative = drawn on top).
  // #134: ribbons render at their real OSM width — the traffic kernel bakes
  // width-aware lane offsets, so no render-width floor. (The terrain CORRIDOR
  // still uses corridorWidths — lane-floored — but only for grading/mask/
  // sampler, never for these ribbons.)
  const carriageWidthOf = (p: RoadPath): number => p.width;
  const railBedWidthOf = (p: RoadPath): number => p.width + 2.2;
  const railWidthOf = (p: RoadPath): number => p.width;
  const footWidthOf = (p: RoadPath): number => p.width;
  group.add(stripsMesh('carriageRibbons', carriage, carriageWidthOf, kswCity.roadColors.carriage, kswCity.roadYs.carriage, -1, groundYAt));
  group.add(stripsMesh('footwayRibbons', foot, footWidthOf, kswCity.roadColors.footway, kswCity.roadYs.footway, -1, groundYAt));
  group.add(stripsMesh('railBeds', rails, railBedWidthOf, kswCity.roadColors.railBed, kswCity.roadYs.railBed, 0, groundYAt));
  group.add(stripsMesh('railRibbons', rails, railWidthOf, kswCity.roadColors.rail, kswCity.roadYs.rail, -3, groundYAt));
  // Side-skirts (spec §5 terrain-discard): the terrain shader discards fragments
  // inside every corridor; these vertical aprons close the hole at each ribbon
  // edge (carriage + footway + the ballast bed — the widest rail layer). Only
  // built when draping (a sampler is present); the flat pre-#119 anchor look
  // has no discarded terrain to close. Rails skirt the BED edge (the outermost
  // rail geometry today; railLook lands in PR 3).
  if (groundYAt) {
    group.add(skirtsMesh('carriageSkirts', carriage, carriageWidthOf, kswCity.roadColors.carriage, kswCity.roadYs.carriage, groundYAt));
    group.add(skirtsMesh('footwaySkirts', foot, footWidthOf, kswCity.roadColors.footway, kswCity.roadYs.footway, groundYAt));
    group.add(skirtsMesh('railBedSkirts', rails, railBedWidthOf, kswCity.roadColors.railBed, kswCity.roadYs.railBed, groundYAt));
  }
  return group;
}
