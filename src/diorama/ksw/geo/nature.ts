// src/diorama/ksw/geo/nature.ts
// The living layer of the real city: OSM parks/woods as soft green patches,
// the Eulach and ponds as calm water, and every individually mapped tree as
// a chunky clay tree (instanced — thousands of trees, two draw calls).
// Deterministic per-tree size/tint variation, no RNG.
import * as THREE from 'three/webgpu';
import { kswCity } from '../../designTokens';
import { clayMat } from '../props';
import type { CityNature, GreenArea } from './geoData';

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

function vertexTintMat(base: number): THREE.MeshPhysicalMaterial {
  const m = clayMat(base).clone();
  m.vertexColors = true;
  m.color = new THREE.Color(0xffffff);
  return m;
}

export type NatureOptions = {
  // trees inside this rect (center cx/cz, size w/d) are skipped — the hero
  // plate has its own authored trees
  excludeRect?: { x: number; z: number; w: number; d: number };
  // canopies cast shadows onto the sun's shadow map. Default false — with
  // ~4k instanced trees this was a major frame-time cost for a barely
  // visible effect at city scale. The near-camera LOD ring (Task 10) turns
  // it back on where it actually reads.
  treeShadows?: boolean;
};

export function buildNature(nature: CityNature, opts: NatureOptions = {}): THREE.Group {
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
  const greens = new THREE.Mesh(flatAreas(greenRings, kswCity.greenY), vertexTintMat(kswCity.parkGreen));
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

  // trees: chunky clay lollipops, instanced. Canopy = icosphere, trunk = cylinder.
  const ex = opts.excludeRect;
  const spots = nature.trees.filter(
    ({ x, z }) => !ex || Math.abs(x - ex.x) > ex.w / 2 || Math.abs(z - ex.z) > ex.d / 2,
  );
  const canopyGeo = new THREE.IcosahedronGeometry(1, 1);
  const trunkGeo = new THREE.CylinderGeometry(0.14, 0.2, 1, 6);
  const canopy = new THREE.InstancedMesh(canopyGeo, clayMat(kswCity.treeGreen).clone(), spots.length);
  const trunks = new THREE.InstancedMesh(trunkGeo, clayMat(kswCity.treeTrunk), spots.length);
  canopy.name = 'treeCanopies';
  trunks.name = 'treeTrunks';
  const m = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const baseGreen = new THREE.Color(kswCity.treeGreen);
  const tint = new THREE.Color();
  for (let i = 0; i < spots.length; i++) {
    const { x, z } = spots[i];
    const h = hash01(x * 3.1 + z * 7.7);
    const trunkH = 1.0 + h * 0.9;
    const r = 1.1 + h * 1.1; // canopy radius 1.1..2.2 m
    m.compose(new THREE.Vector3(x, trunkH + r * 0.72, z), q, new THREE.Vector3(r, r * (0.92 + 0.16 * hash01(i)), r));
    canopy.setMatrixAt(i, m);
    tint.copy(baseGreen).offsetHSL((h - 0.5) * 0.04, 0, (hash01(i * 2.3) - 0.5) * 0.1);
    canopy.setColorAt(i, tint);
    m.compose(new THREE.Vector3(x, trunkH / 2, z), q, new THREE.Vector3(1, trunkH, 1));
    trunks.setMatrixAt(i, m);
  }
  canopy.castShadow = opts.treeShadows ?? false;
  canopy.receiveShadow = true;
  trunks.castShadow = false;
  trunks.receiveShadow = true;
  group.add(canopy);
  group.add(trunks);
  return group;
}
