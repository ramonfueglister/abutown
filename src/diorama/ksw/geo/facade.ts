// src/diorama/ksw/geo/facade.ts
// Pure derivation: window/door raster from the real footprint + real height.
// floors = height/3 m (documented), columns from real facade length. No RNG.
//
// Footprint rings come from the trace/bake step and their winding order is
// NOT guaranteed (CW vs CCW depending on the source geometry), and real
// traced footprints are frequently concave (L/U-shaped). A yaw formula
// derived from edge direction alone, or from a naive vertex-average
// centroid side-test, gets the WRONG side on edges adjacent to a reflex
// vertex — the centroid of a concave ring can sit on the interior side of
// such an edge, or even outside the polygon entirely. Instead we do an
// exact point-in-polygon side test: offset the edge midpoint by ±0.5m
// along each candidate normal and keep whichever offset point is NOT
// inside the footprint ring. This is correct for any simple polygon,
// convex or concave.
import { kswCityStyle } from '../../designTokens';

export type WindowSlot = { x: number; y: number; z: number; yaw: number };
export type FacadeLayout = { windows: WindowSlot[]; door: WindowSlot | null };

// Standard ray-casting point-in-polygon test. Mirrors scripts/geo/lib/join.mjs'
// pointInRing (kept duplicated on purpose: that file is a .mjs build script,
// this is TS source — do NOT import a .mjs into src).
export function pointInRing(x: number, z: number, ring: number[][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, zi] = ring[i];
    const [xj, zj] = ring[j];
    if (zi > z !== zj > z && x < ((xj - xi) * (z - zi)) / (zj - zi) + xi) inside = !inside;
  }
  return inside;
}

// Outward yaw for an edge from (ax,az) to (bx,bz), robust to ring winding
// AND to concave footprints: offset the edge midpoint by ±0.5m along each
// candidate normal and pick the one whose offset point is outside the
// footprint ring. If both are inside (sliver) or both outside (degenerate),
// return null — caller must skip emitting windows for that edge rather than
// guess.
function outwardYaw(ax: number, az: number, bx: number, bz: number, ring: number[][]): number | null {
  const ex = bx - ax;
  const ez = bz - az;
  const len = Math.hypot(ex, ez);
  const ux = ex / len;
  const uz = ez / len;
  const mx = (ax + bx) / 2;
  const mz = (az + bz) / 2;
  // Two candidate normals (perpendicular to the edge direction).
  const n1x = -uz;
  const n1z = ux;
  const n2x = uz;
  const n2z = -ux;
  const p1x = mx + n1x * 0.5;
  const p1z = mz + n1z * 0.5;
  const p2x = mx + n2x * 0.5;
  const p2z = mz + n2z * 0.5;
  const in1 = pointInRing(p1x, p1z, ring);
  const in2 = pointInRing(p2x, p2z, ring);
  if (in1 === in2) return null; // sliver or degenerate — never guess
  const [nx, nz] = in1 ? [n2x, n2z] : [n1x, n1z];
  // yaw such that (sin(yaw), cos(yaw)) == (nx, nz) — matches windows.ts'
  // position offset convention: x + sin(yaw)*out, z + cos(yaw)*out.
  return Math.atan2(nx, nz);
}

export function facadeLayout(b: {
  footprint: number[][];
  height: number;
  door?: { x: number; z: number; yaw: number };
}): FacadeLayout {
  const s = kswCityStyle;
  const floors = Math.min(24, Math.max(1, Math.round(b.height / s.storeyH)));
  const windows: WindowSlot[] = [];
  let door: WindowSlot | null = null;
  const fp = b.footprint;
  for (let i = 0; i < fp.length; i++) {
    const [ax, az] = fp[i];
    const [bx, bz] = fp[(i + 1) % fp.length];
    const ex = bx - ax;
    const ez = bz - az;
    const len = Math.hypot(ex, ez);
    const cols = Math.floor((len - 0.8) / s.windowSpacing);
    if (cols < 1) continue;
    const ux = ex / len;
    const uz = ez / len;
    const yaw = outwardYaw(ax, az, bx, bz, fp);
    if (yaw === null) continue; // sliver/degenerate edge — never guess the side
    const start = (len - (cols - 1) * s.windowSpacing) / 2;
    const isDoorEdge =
      b.door &&
      Math.abs((b.door.x - ax) * uz - (b.door.z - az) * ux) < 0.5 && // on the edge line
      (b.door.x - ax) * ux + (b.door.z - az) * uz > -0.5 &&
      (b.door.x - ax) * ux + (b.door.z - az) * uz < len + 0.5;
    for (let c = 0; c < cols; c++) {
      const t = start + c * s.windowSpacing;
      const x = ax + ux * t;
      const z = az + uz * t;
      for (let f = 0; f < floors; f++) {
        const y = f * s.storeyH + s.storeyH * s.sillFrac + s.windowH / 2;
        if (y + s.windowH / 2 > b.height - 0.4) break;
        if (f === 0 && isDoorEdge && b.door && Math.hypot(x - b.door.x, z - b.door.z) < s.windowSpacing / 2) {
          door = { x: b.door.x, y: s.doorH / 2, z: b.door.z, yaw: b.door.yaw };
          continue;
        }
        windows.push({ x, y, z, yaw });
      }
    }
  }
  if (!door && b.door) door = { x: b.door.x, y: kswCityStyle.doorH / 2, z: b.door.z, yaw: b.door.yaw };
  return { windows, door };
}
