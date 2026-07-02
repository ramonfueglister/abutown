// src/diorama/ksw/geo/facade.ts
// Pure derivation: window/door raster from the real footprint + real height.
// floors = height/3 m (documented), columns from real facade length. No RNG.
//
// Footprint rings come from the trace/bake step and their winding order is
// NOT guaranteed (CW vs CCW depending on the source geometry). A yaw formula
// derived purely from edge direction would be wrong-side on reversed rings,
// pushing every window instance inside the wall (invisible). Instead we test
// both normal candidates against the ring centroid and pick whichever points
// away from it — robust to winding.
import { kswCityStyle } from '../../designTokens';

export type WindowSlot = { x: number; y: number; z: number; yaw: number };
export type FacadeLayout = { windows: WindowSlot[]; door: WindowSlot | null };

function centroidOf(fp: number[][]): { cx: number; cz: number } {
  let cx = 0;
  let cz = 0;
  for (const [x, z] of fp) {
    cx += x;
    cz += z;
  }
  return { cx: cx / fp.length, cz: cz / fp.length };
}

// Outward yaw for an edge from (ax,az) to (bx,bz), robust to ring winding:
// test both normal candidates by offsetting the edge midpoint and keeping
// the side farther from the footprint centroid.
function outwardYaw(ax: number, az: number, bx: number, bz: number, cx: number, cz: number): number {
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
  const d1 = Math.hypot(p1x - cx, p1z - cz);
  const d2 = Math.hypot(p2x - cx, p2z - cz);
  const [nx, nz] = d1 > d2 ? [n1x, n1z] : [n2x, n2z];
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
  const { cx, cz } = centroidOf(fp);
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
    const yaw = outwardYaw(ax, az, bx, bz, cx, cz);
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
