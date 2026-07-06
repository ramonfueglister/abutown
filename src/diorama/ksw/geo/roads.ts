// src/diorama/ksw/geo/roads.ts
// OSM ways as flat clay ribbons v2: continuous miter-joined strips (no wedge
// gaps, no overlapping quads), one visual layer per class — carriageways,
// footpaths, rail on its ballast band — each on its own height so junctions
// never flicker.
import * as THREE from 'three/webgpu';
import { kswCity } from '../../designTokens';
import { clayMat } from '../props';
import type { RoadPath } from './geoData';
import { corridorWidths, type TrafficNetDoc } from '../../traffic/corridorWidths';
import trafficNetJson from '../../../data/winterthur/trafficnet.json';
import { roadMaskHalfWidth, railMaskHalfWidth } from './groundSampler';

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

/** APRON (Swiss "Bankett"/verge) strip — the flat road-platform surface BEYOND
 * the ribbon edge, filling the annulus from the ribbon edge (`ribbonWidth/2`) to
 * the DISCARD-MASK edge (`maskWidth/2`) at profile height (spec §5 platform).
 *
 * The discard mask floors every way's stamping radius at the mask cell size
 * (2.5 m), so for the 54 % of ways narrower than a cell the mask edge sits
 * BEYOND the ribbon edge — leaving a see-through band the ribbon-edge skirt did
 * not cover from above. The apron IS the road platform out to that mask edge:
 * rendered at profile height, so no void is visible from above; the skirt then
 * drops from the APRON (mask) outer edge, not the ribbon edge.
 *
 * Two flat quad strips (one per side), from the ribbon edge to the mask edge,
 * both at draped profile y + layer offset. Uses the SHARED `miterOffsets` so the
 * inner apron edge coincides byte-identically with the ribbon edge (no seam) and
 * the outer edge coincides with the skirt top edge. Degenerate (mask edge ≤
 * ribbon edge, i.e. wide ways where renderHW ≥ 2.5 m): emits nothing — the
 * caller skips it and the skirt sits directly at the ribbon edge. */
export function apronStrip(
  pts: number[][],
  ribbonWidth: number,
  maskWidth: number,
  y: number,
  groundYAt?: GroundYAt,
): { positions: number[]; indices: number[] } {
  const positions: number[] = [];
  const indices: number[] = [];
  const innerHalf = ribbonWidth / 2;
  const outerHalf = maskWidth / 2;
  // Degenerate: nothing to fill when the mask edge is at (or inside) the ribbon
  // edge — 1 cm slack so float noise never emits a zero-area sliver.
  if (outerHalf <= innerHalf + 0.01) return { positions, indices };
  if (groundYAt) pts = subdivideForDrape(pts, groundYAt);
  const n = pts.length;
  if (n < 2) return { positions, indices };
  const offs = miterOffsets(pts);
  // Two aprons: side = +1 (left) and −1 (right). Each is a flat quad strip from
  // the ribbon edge (inner) to the mask edge (outer) at profile height.
  for (const side of [1, -1]) {
    const base0 = positions.length / 3;
    for (let i = 0; i < n; i++) {
      const [cx, cz] = pts[i];
      const { mx, mz, scale } = offs[i];
      const topY = (groundYAt ? groundYAt(cx, cz) : 0) + y;
      const ox = side * mx * scale;
      const oz = side * mz * scale;
      // inner (ribbon edge) then outer (mask edge)
      positions.push(cx + ox * innerHalf, topY, cz + oz * innerHalf, cx + ox * outerHalf, topY, cz + oz * outerHalf);
      if (i > 0) {
        const a = base0 + (i - 1) * 2;
        indices.push(a, a + 1, a + 2, a + 2, a + 1, a + 3);
      }
    }
  }
  return { positions, indices };
}

/** Vertical side-skirts along BOTH edges of a road platform (spec §5
 * terrain-discard platform, Task 5e). The terrain shader discards every fragment
 * inside a road/rail corridor out to the MASK edge; these skirts drop from the
 * platform's outer (mask) edge down to the real terrain so you never see through
 * the world under a road, on both embankments (fill slope) and cuts (the terrain
 * rises against the skirt as a cut bank).
 *
 * Top edge: at the MASK edge (`maskWidth/2`), draped profile y + layer offset —
 * the outer edge of the apron. Bottom edge: PER-VERTEX at `tileGround(cx,cz) −
 * 0.5 m` (the tile ground sampler, NOT the corridor profile) so the skirt foot
 * ALWAYS reaches the terrain regardless of embankment/cut depth — no constant
 * drop budget. `tileGround` is the runtime `tileGroundYAt` (main.ts), threaded
 * in per way.
 *
 * Geometry uses the SHARED `miterOffsets` at the mask half-width so the skirt top
 * edge coincides byte-identically with the apron outer edge (no seam). Merged
 * into one geometry per layer in buildRoads — no per-frame cost. With no
 * groundYAt/tileGround (near the anchor, drape ≈ 0) the skirt drops a nominal
 * 0.5 m, still a valid apron foot. */
export function skirtStrip(
  pts: number[][],
  maskWidth: number,
  y: number,
  groundYAt?: GroundYAt,
  tileGround?: GroundYAt,
  footM = 0.5,
): { positions: number[]; indices: number[] } {
  const positions: number[] = [];
  const indices: number[] = [];
  const half = maskWidth / 2;
  if (groundYAt) pts = subdivideForDrape(pts, groundYAt);
  const n = pts.length;
  if (n < 2) return { positions, indices };
  // SHARED miter offsets (Task 5e refactor): the SAME per-point {mx,mz,scale}
  // the ribbon and apron use, so the skirt top edge coincides exactly with the
  // apron (mask) outer edge and the three can never drift.
  const offs = miterOffsets(pts);
  // Two skirts: side = +1 (left edge) and −1 (right edge). Each is a vertical
  // quad strip: top at the mask edge (profile height), bottom PER-VERTEX at the
  // tile ground − footM so it always reaches terrain (fill slope on embankments,
  // cut bank on cuts).
  for (const side of [1, -1]) {
    const base0 = positions.length / 3;
    for (let i = 0; i < n; i++) {
      const [cx, cz] = pts[i];
      const { mx, mz, scale } = offs[i];
      const topY = (groundYAt ? groundYAt(cx, cz) : 0) + y;
      const ex = cx + side * mx * half * scale;
      const ez = cz + side * mz * half * scale;
      // Bottom per-vertex from the TILE ground at the skirt-foot position (not
      // the centreline, not the profile): the terrain the skirt must reach.
      const botY = (tileGround ? tileGround(ex, ez) : 0) - footM;
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

/** Apron (Bankett/verge) material: the ribbon clay color darkened ×0.9 (spec §5
 * — the platform verge is a touch darker than the carriage), a flat surface at
 * profile height. Belt-and-braces depth bias like the ribbon so it resolves
 * coplanar with, but visually distinct from, the ribbon layer. Rendered
 * DoubleSide so the verge reads from above regardless of strip winding (it is a
 * genuine top surface, not a thin wall — a flipped normal must not hide it). */
function apronMat(color: number, polygonOffsetUnits: number): THREE.MeshPhysicalMaterial {
  const m = ribbonMat(darken(color, 0.9), polygonOffsetUnits);
  m.side = THREE.DoubleSide;
  return m;
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

/** Build the merged APRON mesh for a layer (spec §5 platform): each way
 * contributes two flat verge strips from the ribbon edge to the mask edge at
 * profile height, so the road platform (not void) fills every cell the discard
 * mask removed. `maskWidthOf` returns the FULL mask width (2×maskHW). */
function apronsMesh(
  name: string,
  paths: RoadPath[],
  ribbonWidthOf: (p: RoadPath, i: number) => number,
  maskWidthOf: (p: RoadPath, i: number) => number,
  color: number,
  y: number,
  polygonOffsetUnits: number,
  groundYAt?: GroundYAt,
): THREE.Mesh {
  const positions: number[] = [];
  const indices: number[] = [];
  for (let idx = 0; idx < paths.length; idx++) {
    const p = paths[idx];
    const s = apronStrip(p.pts, ribbonWidthOf(p, idx), maskWidthOf(p, idx), y, groundYAt);
    const base = positions.length / 3;
    positions.push(...s.positions);
    for (const i of s.indices) indices.push(base + i);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setIndex(positions.length / 3 > 65535 ? new THREE.BufferAttribute(new Uint32Array(indices), 1) : new THREE.BufferAttribute(new Uint16Array(indices), 1));
  geo.computeVertexNormals();
  const mesh = new THREE.Mesh(geo, apronMat(color, polygonOffsetUnits));
  mesh.name = name;
  mesh.receiveShadow = true;
  mesh.castShadow = false;
  return mesh;
}

/** Build the merged side-skirt mesh for a layer (spec §5 terrain-discard
 * platform, Task 5e): every way contributes two vertical apron strips from its
 * platform (MASK) outer edge down to the tile ground − 0.5 m, closing the hole
 * the terrain-discard shader opens under it. `maskWidthOf` returns the FULL mask
 * width (2×maskHW); `tileGround` is the tile-ground sampler for the skirt foot. */
function skirtsMesh(
  name: string,
  paths: RoadPath[],
  maskWidthOf: (p: RoadPath, i: number) => number,
  color: number,
  y: number,
  groundYAt: GroundYAt,
  tileGround: GroundYAt,
): THREE.Mesh {
  const positions: number[] = [];
  const indices: number[] = [];
  for (let idx = 0; idx < paths.length; idx++) {
    const p = paths[idx];
    const s = skirtStrip(p.pts, maskWidthOf(p, idx), y, groundYAt, tileGround);
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

/** Build the KSW city road/rail platform (spec §5 platform). Roads own a
 * platform = ribbon + apron out to the DISCARD-MASK edge, with skirts dropping
 * from that mask edge to the tile terrain. `groundYAt` is the corridor-aware
 * profile sampler (drapes ribbon/apron/skirt-top); `tileGround` is the raw tile
 * ground sampler (main.ts `tileGroundYAt`) used for the per-vertex skirt foot.
 * Both are required together to build the platform — the flat pre-#119 anchor
 * look (no sampler) has no discarded terrain and renders bare ribbons. */
export function buildRoads(
  roads: RoadPath[],
  rails: RoadPath[],
  groundYAt?: GroundYAt,
  tileGround?: GroundYAt,
): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityRoads';
  const carriage = roads.filter((r) => !FOOT.has(r.class));
  const foot = roads.filter((r) => FOOT.has(r.class));
  // #134: ribbons render at their real OSM width — the traffic kernel bakes
  // width-aware lane offsets (trafficnet.mjs :: laneOffsets), so cars fit the
  // real tarmac by construction; the former FIX-D1 render-width floor is gone
  // (it widened the world and swallowed street-tree verges / grazed facades).
  // The terrain CORRIDOR still uses the lane-floored corridorWidths — the SAME
  // widths the bake mask stamps (Finding 1a) — but ONLY for the mask/platform
  // extents below, never for the drawn ribbon; the apron bridges the visible
  // ribbon→mask gap (Bankett).
  // polygonOffset ladder (units) matches the roadYs height ladder bottom→top:
  // railBed 0 < carriage/footway −1 < rail −3 (more negative = drawn on top).
  const carriageWidths = corridorWidths(carriage, trafficNetJson as unknown as TrafficNetDoc);
  const carriageWidthOf = (p: RoadPath): number => p.width;
  const railBedWidthOf = (p: RoadPath): number => p.width + 2.2;
  const railWidthOf = (p: RoadPath): number => p.width;
  const footWidthOf = (p: RoadPath): number => p.width;
  // PLATFORM widths (2×platform half-width) — the apron outer edge and skirt top
  // edge. The bake mask footprint (renderHW floored at the 2.5 m cell) PLUS the
  // raster-quantization margin (cell·√2/2) so the platform covers the full
  // discretised discard extent — no fringe void. apron fills ribbon→platform
  // edge, skirt drops from platform edge→terrain. Shared helpers in groundSampler
  // (MIRROR-pinned to bake-world.mjs) so nothing drifts.
  const carriageMaskWidthOf = (p: RoadPath, i: number): number =>
    2 * roadMaskHalfWidth(p.width, carriageWidths[i] ?? p.width);
  const footMaskWidthOf = (p: RoadPath): number => 2 * roadMaskHalfWidth(p.width, p.width);
  const railBedMaskWidthOf = (p: RoadPath): number => 2 * railMaskHalfWidth(p.width);
  group.add(stripsMesh('carriageRibbons', carriage, carriageWidthOf, kswCity.roadColors.carriage, kswCity.roadYs.carriage, -1, groundYAt));
  group.add(stripsMesh('footwayRibbons', foot, footWidthOf, kswCity.roadColors.footway, kswCity.roadYs.footway, -1, groundYAt));
  group.add(stripsMesh('railBeds', rails, railBedWidthOf, kswCity.roadColors.railBed, kswCity.roadYs.railBed, 0, groundYAt));
  group.add(stripsMesh('railRibbons', rails, railWidthOf, kswCity.roadColors.rail, kswCity.roadYs.rail, -3, groundYAt));
  // Platform apron + terrain-grounded skirts (spec §5 platform). The terrain
  // shader discards fragments inside every corridor OUT TO THE MASK EDGE; the
  // apron fills that annulus (ribbon edge → mask edge) at profile height so no
  // void shows from above (closes the Finding-1a see-through band on the 54 % of
  // ways narrower than a mask cell), and the skirt drops from the mask edge to
  // the tile terrain (fill slope on embankments, cut bank on cuts). Only built
  // when draping AND a tile-ground sampler is present (both from main.ts). Rails
  // use the ballast BED as the platform (the outermost rail geometry today;
  // railLook lands in PR 3).
  if (groundYAt && tileGround) {
    group.add(apronsMesh('carriageAprons', carriage, carriageWidthOf, carriageMaskWidthOf, kswCity.roadColors.carriage, kswCity.roadYs.carriage, -1, groundYAt));
    group.add(apronsMesh('footwayAprons', foot, footWidthOf, footMaskWidthOf, kswCity.roadColors.footway, kswCity.roadYs.footway, -1, groundYAt));
    group.add(apronsMesh('railBedAprons', rails, railBedWidthOf, railBedMaskWidthOf, kswCity.roadColors.railBed, kswCity.roadYs.railBed, 0, groundYAt));
    group.add(skirtsMesh('carriageSkirts', carriage, carriageMaskWidthOf, kswCity.roadColors.carriage, kswCity.roadYs.carriage, groundYAt, tileGround));
    group.add(skirtsMesh('footwaySkirts', foot, footMaskWidthOf, kswCity.roadColors.footway, kswCity.roadYs.footway, groundYAt, tileGround));
    group.add(skirtsMesh('railBedSkirts', rails, railBedMaskWidthOf, kswCity.roadColors.railBed, kswCity.roadYs.railBed, groundYAt, tileGround));
  }
  return group;
}
