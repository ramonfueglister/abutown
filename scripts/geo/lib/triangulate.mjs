// scripts/geo/lib/triangulate.mjs
// Triangulates the planar (walls/roofs are planar in swissBUILDINGS3D)
// 3D polygons of the LoD2 surfaces: Newell normal → drop the dominant
// axis → 2D ear-cut via three's ShapeUtils. Degenerate rings → null.
import { ShapeUtils, Vector2 } from 'three';

export function polygonNormal(ring) {
  let nx = 0, ny = 0, nz = 0;
  for (let i = 0; i < ring.length; i++) {
    const [ax, ay, az] = ring[i];
    const [bx, by, bz] = ring[(i + 1) % ring.length];
    nx += (ay - by) * (az + bz);
    ny += (az - bz) * (ax + bx);
    nz += (ax - bx) * (ay + by);
  }
  return [nx, ny, nz];
}

export function triangulatePlanarPolygon(ring) {
  // drop a duplicated closing point
  const pts =
    ring.length > 1 && ring[0].every((v, i) => Math.abs(v - ring[ring.length - 1][i]) < 1e-9)
      ? ring.slice(0, -1)
      : ring.slice();
  if (pts.length < 3) return null;

  const [nx, ny, nz] = polygonNormal(pts);
  const [ax, ay, az] = [Math.abs(nx), Math.abs(ny), Math.abs(nz)];
  if (ax + ay + az < 1e-6) return null; // zero area / collinear

  // project onto the plane's dominant axis pair, keeping winding intact
  let to2d;
  if (ay >= ax && ay >= az) to2d = ny >= 0 ? (p) => [p[0], p[2]] : (p) => [p[2], p[0]];
  else if (ax >= az) to2d = nx >= 0 ? (p) => [p[2], p[1]] : (p) => [p[1], p[2]];
  else to2d = nz >= 0 ? (p) => [p[1], p[0]] : (p) => [p[0], p[1]];

  // three r185 ShapeUtils calls Vector2 methods (.equals) on the contour —
  // plain {x,y} objects throw, so build real Vector2 instances.
  const contour = pts.map((p) => {
    const [u, v] = to2d(p);
    return new Vector2(u, v);
  });
  const tris = ShapeUtils.triangulateShape(contour, []);
  if (tris.length === 0) return null;

  return {
    positions: pts.flat(),
    indices: tris.flat(),
  };
}
