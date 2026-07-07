// src/diorama/ksw/geo/kswCampus.ts
// The real KSW campus (26 baked swisstopo buildings, zone==='ksw') rendered
// through the SAME clay-massing pipeline as the city (Task 15, S3a): walls
// with the TSL facade shader (fuv attribute, procedural windows), roofs,
// plinth/eave trim bands. Only the group/mesh names differ (kswCampus*
// instead of city*) so both can live in the scene side by side without name
// collisions. mainBuilding = the largest-footprint-area building (the tower
// + wing complex) — later tasks (zone decomposition, cutaway) key off it.
//
// Dollhouse cutaway (Task 18, S3c; storey-peel v2 Phase A): the MAIN
// building's wall+roof are split off from the other 25 into their own meshes
// so the cutaway uniforms slice ONLY the main building. Its walls get the
// cutaway facade material (hard discard above `discardAbove`, a dissolving
// shell band between `bandLo` and `discardAbove`, a bright seam band); its
// roof + eave band fade out with `roofFade` BEFORE the hard slice engages.
// The other 25 campus buildings stay closed — they render through the plain
// city pipeline exactly as in T15.
// `group.userData.setCutaway({ discardAbove, bandLo, bandFade, roofFade })`
// drives it every frame.
import * as THREE from 'three/webgpu';
import { kswCityStyle, kswPalette, palette, terrainLook } from '../../designTokens';
import { float, mix, vec3 } from 'three/tsl';
import { clayMat } from '../props';
import { snowU } from '../glowUniform';
import {
  buildCityMassing,
  facadeMaterial,
  mergeTinted,
  mergeWalls,
  ringBandParts,
  type CutawayFacadeMaterial,
} from './cityMassing';
import type { BakedBuilding } from './geoData';

// shoelace formula, absolute value — footprint rings are simple polygons in
// local metres (x, z), winding direction doesn't matter for area comparison.
export function footprintArea(fp: number[][]): number {
  let area = 0;
  for (let i = 0; i < fp.length; i++) {
    const [x1, z1] = fp[i];
    const [x2, z2] = fp[(i + 1) % fp.length];
    area += x1 * z2 - x2 * z1;
  }
  return Math.abs(area) / 2;
}

export function largestBuilding(buildings: BakedBuilding[]): BakedBuilding {
  let mainBuilding = buildings[0];
  let maxArea = -Infinity;
  for (const b of buildings) {
    const a = footprintArea(b.footprint);
    if (a > maxArea) {
      maxArea = a;
      mainBuilding = b;
    }
  }
  return mainBuilding;
}

export type CutawayUniforms = { discardAbove: number; bandLo: number; bandFade: number; roofFade: number };

export function buildKswCampus(
  buildings: BakedBuilding[],
): { group: THREE.Group; mainBuilding: BakedBuilding } {
  const group = new THREE.Group();
  group.name = 'kswCampus';

  const mainBuilding = largestBuilding(buildings);
  const others = buildings.filter((b) => b.id !== mainBuilding.id);

  // ── the 25 secondary buildings: plain city pipeline, always closed ──────
  const secondary = buildCityMassing(others);
  const rename: Record<string, string> = {
    cityWalls: 'kswCampusWalls',
    cityRoofs: 'kswCampusRoofs',
    cityPlinths: 'kswCampusPlinths',
    cityEaves: 'kswCampusEaves',
  };
  for (const child of [...secondary.children]) {
    if (rename[child.name]) child.name = rename[child.name];
    group.add(child);
  }
  const walls = group.getObjectByName('kswCampusWalls');
  (walls?.userData.setFacadeDetail as ((on: boolean) => void) | undefined)?.(true);

  // ── the main building: split meshes with the cutaway material ────────────
  // Walls carry the cutaway facade shader (discard above cutH + seam band).
  const mainWallMat = facadeMaterial(palette.creamBase, { cutaway: true }) as CutawayFacadeMaterial;
  mainWallMat.facadeDetail.value = 1; // hero/near: full window raster
  const mainWalls = new THREE.Mesh(mergeWalls([mainBuilding], palette.creamBase), mainWallMat);
  mainWalls.name = 'kswMainWalls';
  mainWalls.castShadow = true;
  mainWalls.receiveShadow = true;
  group.add(mainWalls);

  // Roof: fades out (opacity = roofFade) BEFORE the slice engages so the top
  // is gone by the time the wall is cut open.
  const mainRoofMat = clayMat(kswPalette.roofClay).clone();
  mainRoofMat.transparent = true;
  mainRoofMat.depthWrite = true; // opaque at rest; opacity ramps to 0 on open
  // Snow cover like the city roofs (tintedClay snow variant) — without this
  // the hero roof stayed terracotta in a snowed-in city (2026-07-07).
  {
    const roofC = new THREE.Color(kswPalette.roofClay);
    const snowC = new THREE.Color(terrainLook.snow);
    mainRoofMat.colorNode = mix(
      vec3(roofC.r, roofC.g, roofC.b),
      vec3(snowC.r, snowC.g, snowC.b),
      snowU.mul(float(0.85)),
    ) as THREE.MeshPhysicalMaterial['colorNode'];
  }
  const mainRoof = new THREE.Mesh(mergeTinted([mainBuilding], (b) => b.roof, kswPalette.roofClay), mainRoofMat);
  mainRoof.name = 'kswMainRoof';
  mainRoof.castShadow = true;
  mainRoof.receiveShadow = true;
  group.add(mainRoof);

  // Plinth stays (ground level, below the cut — always visible).
  const plinthParts = ringBandParts(mainBuilding.footprint, -kswCityStyle.plinthSink, kswCityStyle.plinthH, kswCityStyle.plinthOut);
  const mainPlinth = new THREE.Mesh(plinthParts, clayMat(palette.white));
  mainPlinth.name = 'kswMainPlinth';
  mainPlinth.castShadow = false;
  mainPlinth.receiveShadow = true;
  group.add(mainPlinth);

  // Eave band sits near the top — fade it with the roof so it doesn't poke
  // through the open cut.
  // Anchor at the baked eave: mainBuilding.height is the RIDGE (the tower),
  // so a height-derived band floats far above the footprint volume's real eave.
  const eaveY = Math.max(mainBuilding.eaveH, kswCityStyle.plinthH + 0.5);
  const eaveParts = ringBandParts(mainBuilding.footprint, eaveY - kswCityStyle.eaveBandH, eaveY, kswCityStyle.eaveBandOut);
  const mainEaveMat = clayMat(kswPalette.roofTrim).clone();
  mainEaveMat.transparent = true;
  mainEaveMat.depthWrite = true;
  const mainEave = new THREE.Mesh(eaveParts, mainEaveMat);
  mainEave.name = 'kswMainEave';
  mainEave.castShadow = false;
  mainEave.receiveShadow = true;
  group.add(mainEave);

  // Per-frame cutaway driver (T18). At rest (cutH 1e6, fade 1) every write is
  // a no-op → the closed campus is pixel-identical to T15.
  group.userData.setCutaway = (u: CutawayUniforms): void => {
    mainWallMat.discardAbove.value = u.discardAbove;
    mainWallMat.bandLo.value = u.bandLo;
    mainWallMat.bandFade.value = u.bandFade;
    mainRoofMat.opacity = u.roofFade;
    mainRoof.visible = u.roofFade > 0.001;
    mainEaveMat.opacity = u.roofFade;
    mainEave.visible = u.roofFade > 0.001;
  };

  return { group, mainBuilding };
}
