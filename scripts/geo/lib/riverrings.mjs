// scripts/geo/lib/riverrings.mjs
//
// Buffered-centreline water rings for rivers that OSM maps only as a
// `waterway=river`/`stream` LINE (no water POLYGON) — spec 2026-07-06 §4.4
// names the Eulach, which has 24 named centreline segments but no
// natural=water area, so terrain grading would happily bulldoze straight
// across it without this. For each such centreline we build a closed ring by
// offsetting the polyline ±halfWidthM (default 3 m), mitered at interior
// joints exactly like trafficnet.mjs's offsetRight, then stitching the right
// edge (forward) to the left edge (reversed) into one closed loop. These rings
// ADD to the polygon-derived waterRings passed to gradeDem so the kernel skips
// (and flags as bridge sites) any corridor cell over the buffered channel.
//
// Deterministic and pure: same input line → byte-identical ring. No fallbacks —
// a line with < 2 points throws rather than silently producing a degenerate
// ring.

/** Right-of-travel unit normal for direction (dx,dz): clockwise 90° rotation. */
function rightNormal(dx, dz) {
  const len = Math.hypot(dx, dz) || 1;
  return [-dz / len, dx / len];
}

// Offset a polyline to the right by `dist` metres (negative = left), mitering
// interior joints so the offset stays `|dist|` from each segment. Straight
// line-by-line copy of trafficnet.mjs offsetRight so the two agree exactly.
function offsetRight(pts, dist) {
  const n = pts.length;
  if (n < 2) return pts.map(([x, z]) => [x, z]);
  const segN = [];
  for (let i = 0; i < n - 1; i++) {
    const dx = pts[i + 1][0] - pts[i][0];
    const dz = pts[i + 1][1] - pts[i][1];
    segN.push(rightNormal(dx, dz));
  }
  const out = [];
  for (let i = 0; i < n; i++) {
    let nx;
    let nz;
    if (i === 0) {
      [nx, nz] = segN[0];
    } else if (i === n - 1) {
      [nx, nz] = segN[n - 2];
    } else {
      const [ax, az] = segN[i - 1];
      const [bx, bz] = segN[i];
      let mx = ax + bx;
      let mz = az + bz;
      const ml = Math.hypot(mx, mz);
      if (ml < 1e-6) {
        mx = ax;
        mz = az;
      } else {
        mx /= ml;
        mz /= ml;
        const cos = mx * bx + mz * bz;
        const scale = Math.min(1 / Math.max(cos, 0.2), 4);
        mx *= scale;
        mz *= scale;
      }
      nx = mx;
      nz = mz;
    }
    out.push([pts[i][0] + nx * dist, pts[i][1] + nz * dist]);
  }
  return out;
}

/** Drop consecutive duplicate points (< 1 mm apart) so a river line with a
 *  repeated vertex doesn't produce a zero-length segment / NaN normal. */
function dedupe(pts) {
  const out = [];
  for (const p of pts) {
    const last = out[out.length - 1];
    if (last && Math.hypot(p[0] - last[0], p[1] - last[1]) < 1e-3) continue;
    out.push([p[0], p[1]]);
  }
  return out;
}

/**
 * Buffer one centreline polyline into a closed ring at ±halfWidthM.
 * `pts` = [[x,z], ...] in local metres. Returns [[x,z], ...] wound as
 * right-edge-forward then left-edge-reversed, closing back on itself.
 * A straight 2-point line yields a 4-corner rectangle strip.
 */
export function bufferCenterline(pts, halfWidthM = 3) {
  const clean = dedupe(pts);
  if (!Array.isArray(clean) || clean.length < 2) {
    throw new Error('bufferCenterline: line must have >= 2 distinct points');
  }
  if (!(halfWidthM > 0)) throw new Error('bufferCenterline: halfWidthM must be > 0');
  const right = offsetRight(clean, halfWidthM);
  const left = offsetRight(clean, -halfWidthM);
  // right edge forward, then left edge reversed → a single closed loop.
  return [...right, ...left.slice().reverse()];
}

/**
 * Build buffered-centreline water rings from transformNature's `rivers`
 * (each { width, pts:[[x,z],...] } already in local metres). One ring per
 * river line, buffered by ±halfWidthM. Deterministic (input order preserved).
 */
export function riverCenterlineRings(rivers, halfWidthM = 3) {
  if (!Array.isArray(rivers)) throw new Error('riverCenterlineRings: rivers must be an array');
  const rings = [];
  for (const r of rivers) {
    if (!r || !Array.isArray(r.pts) || dedupe(r.pts).length < 2) continue;
    rings.push(bufferCenterline(r.pts, halfWidthM));
  }
  return rings;
}
