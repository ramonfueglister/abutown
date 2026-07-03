// src/diorama/ksw/geo/kswCampus.ts
// The real KSW campus (26 baked swisstopo buildings, zone==='ksw') rendered
// through the SAME clay-massing pipeline as the city (Task 15, S3a): walls
// with the TSL facade shader (fuv attribute, procedural windows), roofs,
// plinth/eave trim bands. Only the group/mesh names differ (kswCampus*
// instead of city*) so both can live in the scene side by side without name
// collisions. mainBuilding = the largest-footprint-area building (the tower
// + wing complex) — later tasks (zone decomposition, cutaway) key off it.
import * as THREE from 'three/webgpu';
import { buildCityMassing } from './cityMassing';
import type { BakedBuilding } from './geoData';

// shoelace formula, absolute value — footprint rings are simple polygons in
// local metres (x, z), winding direction doesn't matter for area comparison.
function footprintArea(fp: number[][]): number {
  let area = 0;
  for (let i = 0; i < fp.length; i++) {
    const [x1, z1] = fp[i];
    const [x2, z2] = fp[(i + 1) % fp.length];
    area += x1 * z2 - x2 * z1;
  }
  return Math.abs(area) / 2;
}

export function buildKswCampus(
  buildings: BakedBuilding[],
  opts: { lampGlow: boolean },
): { group: THREE.Group; mainBuilding: BakedBuilding } {
  const massing = buildCityMassing(buildings, opts);
  const group = new THREE.Group();
  group.name = 'kswCampus';

  // Rename the city-pipeline children to the campus namespace and re-parent —
  // keeps the two pipelines from colliding by mesh name if both are ever
  // traversed together (getObjectByName), while reusing every builder as-is.
  const rename: Record<string, string> = {
    cityWalls: 'kswCampusWalls',
    cityRoofs: 'kswCampusRoofs',
    cityPlinths: 'kswCampusPlinths',
    cityEaves: 'kswCampusEaves',
  };
  for (const child of [...massing.children]) {
    if (rename[child.name]) child.name = rename[child.name];
    group.add(child);
  }

  // Hero zone: facade detail is ALWAYS on (near ring), unlike the city LOD
  // ring toggle — exposed identically as userData.setFacadeDetail per the
  // City pipeline convention, and immediately forced on.
  const walls = group.getObjectByName('kswCampusWalls');
  (walls?.userData.setFacadeDetail as ((on: boolean) => void) | undefined)?.(true);

  let mainBuilding = buildings[0];
  let maxArea = -Infinity;
  for (const b of buildings) {
    const a = footprintArea(b.footprint);
    if (a > maxArea) {
      maxArea = a;
      mainBuilding = b;
    }
  }

  return { group, mainBuilding };
}
