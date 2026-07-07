// src/diorama/ksw/geo/nature.ts
// The living layer of the real city: OSM parks/woods as soft green patches and
// the Eulach and ponds as calm water. Trees are a separate concern now — the
// archetype tree layer (buildTreeLayer / treeImpostors) owns them; buildNature
// no longer touches nature.trees.
import * as THREE from 'three/webgpu';
import { attribute, float, mix, mx_noise_float, normalWorld, positionWorld, vec3 } from 'three/tsl';
import { kswCity, terrainLook } from '../../designTokens';
import { clayMat } from '../props';
import { snowU } from '../glowUniform';
import type { CityNature, GreenArea } from './geoData';

// Runtime-typed TSL node (same convention as treeLayer/cityMassing — the
// @types/three node graph can't model these compositions).
type TSLNode = any;

// Terrain detail tint (SOTA 2026-07-06): flat vertex-colour fills read as
// paper cutouts. Two octaves of world-space MaterialX noise mottle the
// luminance (~9 m and ~48 m features), noise patches + slopes dry toward
// terrainLook.dry — grass gets texture without a single texture asset.
// Shared by the streamed terrain tiles (terrain.ts) and the plate greens.
export function terrainDetailTint(tint: TSLNode): TSLNode {
  const n1 = mx_noise_float(vec3(positionWorld.x.mul(float(0.11)), float(0), positionWorld.z.mul(float(0.11))));
  const n2 = mx_noise_float(vec3(positionWorld.x.mul(float(0.021)), float(7.3), positionWorld.z.mul(float(0.021))));
  const lum = float(1).add(n1.mul(float(0.05))).add(n2.mul(float(0.09)));
  const dry = new THREE.Color(terrainLook.dry);
  const dryPatch = n2.mul(float(0.5)).add(float(0.5)).mul(float(0.22)); // 0..0.22 patchiness
  const slope = float(1).sub(normalWorld.y).clamp(float(0), float(1));
  const slopeMix = slope.mul(float(1.4)).clamp(float(0), float(0.45));
  const base = tint.mul(lum);
  const dryTone = vec3(dry.r, dry.g, dry.b).mul(lum);
  const detail = mix(base, dryTone, dryPatch.add(slopeMix).clamp(float(0), float(0.55)));
  // Snow cover: flat-ish ground whitens with snowU (steep faces shed snow);
  // the noise lum keeps the blanket from reading as a flat white fill.
  const snow = new THREE.Color(terrainLook.snow);
  const snowMask = snowU
    .mul(float(1).sub(slope.mul(float(2.5))).clamp(float(0), float(1)))
    .mul(float(0.9));
  return mix(detail, vec3(snow.r, snow.g, snow.b).mul(lum), snowMask);
}

// vertexTintMat + the detail tint above — the standard ground material.
export function groundTintMat(base: number): THREE.MeshPhysicalMaterial {
  const m = vertexTintMat(base);
  // vertexColors off: the colorNode consumes the tint attribute itself — with
  // vertexColors on, the engine multiplies the tint on AGAIN after the node
  // (snow-white × park-green = green; the whitening no-ops). 2026-07-07.
  m.vertexColors = false;
  // tint² on purpose: the approved ground look was curated while the engine
  // multiplied the vertex tint in a second time (vertexColors + colorNode).
  // The double-multiply is now explicit — same pixels, honest math.
  const t = attribute('color') as TSLNode;
  m.colorNode = terrainDetailTint(t.mul(t)) as THREE.MeshPhysicalMaterial['colorNode'];
  return m;
}

function hash01(n: number): number {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
}

// triangulate flat rings (y = const) into one merged geometry
function flatAreas(rings: Array<{ ring: number[][]; color: THREE.Color }>, y: number): THREE.BufferGeometry {
  const positions: number[] = [];
  const colors: number[] = [];
  const indices: number[] = [];
  for (const { ring, color } of rings) {
    const pts = ring.length > 1 && ring[0][0] === ring[ring.length - 1][0] && ring[0][1] === ring[ring.length - 1][1]
      ? ring.slice(0, -1)
      : ring;
    if (pts.length < 3) continue;
    const contour = pts.map(([x, z]) => new THREE.Vector2(x, z));
    const tris = THREE.ShapeUtils.triangulateShape(contour, []);
    if (tris.length === 0) continue;
    const base = positions.length / 3;
    for (const [x, z] of pts) {
      positions.push(x, y, z);
      colors.push(color.r, color.g, color.b);
    }
    for (const t of tris.flat()) indices.push(base + t);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), 3));
  geo.setAttribute('color', new THREE.BufferAttribute(new Float32Array(colors), 3));
  geo.setIndex(
    positions.length / 3 > 65535
      ? new THREE.BufferAttribute(new Uint32Array(indices), 1)
      : new THREE.BufferAttribute(new Uint16Array(indices), 1),
  );
  geo.computeVertexNormals();
  return geo;
}

// river polylines as flat ribbons, appended into the same position/color soup
function riverRibbons(rivers: CityNature['rivers'], color: THREE.Color): Array<{ ring: number[][]; color: THREE.Color }> {
  const quads: Array<{ ring: number[][]; color: THREE.Color }> = [];
  for (const r of rivers) {
    for (let i = 0; i < r.pts.length - 1; i++) {
      const [x0, z0] = r.pts[i];
      const [x1, z1] = r.pts[i + 1];
      const dx = x1 - x0;
      const dz = z1 - z0;
      const len = Math.hypot(dx, dz);
      if (len < 0.05) continue;
      const hx = (-dz / len) * (r.width / 2);
      const hz = (dx / len) * (r.width / 2);
      quads.push({ ring: [[x0 + hx, z0 + hz], [x1 + hx, z1 + hz], [x1 - hx, z1 - hz], [x0 - hx, z0 - hz]], color });
    }
  }
  return quads;
}

export function vertexTintMat(base: number): THREE.MeshPhysicalMaterial {
  const m = clayMat(base).clone();
  m.vertexColors = true;
  m.color = new THREE.Color(0xffffff);
  return m;
}

export type NatureOptions = {
  // Kept for call-site symmetry with buildTreeLayer (which consumes it) — the
  // hero plate's rect. buildNature no longer builds trees, so it ignores this.
  excludeRect?: { x: number; z: number; w: number; d: number };
};

export function buildNature(nature: CityNature, _opts: NatureOptions = {}): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityNature';

  // green patches: parks lively, woods deeper — slight per-area tint variation
  const park = new THREE.Color(kswCity.parkGreen);
  const wood = new THREE.Color(kswCity.woodGreen);
  const greenRings = nature.greens.map((g: GreenArea, i: number) => {
    const base = g.kind === 'wood' || g.kind === 'forest' || g.kind === 'scrub' ? wood : park;
    const c = base.clone().offsetHSL((hash01(i) - 0.5) * 0.02, 0, (hash01(i * 1.7) - 0.5) * 0.06);
    return { ring: g.ring, color: c };
  });
  const greens = new THREE.Mesh(flatAreas(greenRings, kswCity.greenY), groundTintMat(kswCity.parkGreen));
  greens.name = 'natureGreens';
  greens.receiveShadow = true;
  greens.castShadow = false;
  group.add(greens);

  // water: areas + river ribbons in one calm-blue mesh
  const waterColor = new THREE.Color(kswCity.water);
  const waterRings = [
    ...nature.waterAreas.map((w) => ({ ring: w.ring, color: waterColor })),
    ...riverRibbons(nature.rivers, waterColor),
  ];
  const water = new THREE.Mesh(flatAreas(waterRings, kswCity.waterY), vertexTintMat(kswCity.water));
  water.name = 'natureWater';
  water.receiveShadow = true;
  water.castShadow = false;
  group.add(water);

  return group;
}
