// scripts/geo/lib/project.mjs
// Local equirectangular projection around the KSW anchor. Over the ≤1.6 km
// bake bbox the distortion vs true geodesic distance is <0.1 m — verified
// against haversine in tests/geo/project.test.ts. +x = east, +z = SOUTH
// (three.js right-handed ground plane), y is height and untouched here.

export const ANCHOR = { lon: 8.7285, lat: 47.5069 }; // KSW Brauerstrasse 15
export const BBOX = { lonMin: 8.715, latMin: 47.4955, lonMax: 8.73, latMax: 47.5085 };

const R = 6371008.8; // mean earth radius, matches the haversine reference

export function makeProjector(anchor) {
  const rad = Math.PI / 180;
  const cos0 = Math.cos(anchor.lat * rad);
  return {
    anchorLon: anchor.lon,
    anchorLat: anchor.lat,
    toLocal(lon, lat) {
      const x = (lon - anchor.lon) * rad * R * cos0;
      const north = (lat - anchor.lat) * rad * R;
      return [x, -north];
    },
  };
}
