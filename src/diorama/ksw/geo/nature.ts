// src/diorama/ksw/geo/nature.ts
// The living layer of the real city: OSM parks/woods as soft green patches,
// the Eulach and ponds as calm water, and every individually mapped tree as
// a chunky clay tree (instanced — thousands of trees, two draw calls).
// Deterministic per-tree size/tint variation, no RNG.
import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { kswCity } from '../../designTokens';
import { clayMat } from '../props';
import type { CityNature, GreenArea } from './geoData';

const mergeGeos = (g: THREE.BufferGeometry[]) => mergeGeometries(g)!;

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

  // trees: chunky clay originals, instanced. Broadleaf = merged 4-puff crown
  // (props.ts `tree()` vocabulary), conifer = 2-stage cone stack, both share
  // one trunk InstancedMesh. Far LOD ring (Task 10 toggles it): TWO impostor
  // meshes — a low-poly merge of the SAME 4-puff layout for broadleaf and a
  // single low-poly cone for conifers — so the far silhouette matches the
  // near form (debug addendum Task 14: the old single-ball impostor read as
  // uniform structureless blobs in the establishing view).
  const ex = opts.excludeRect;
  const spots = nature.trees.filter(
    ({ x, z }) => !ex || Math.abs(x - ex.x) > ex.w / 2 || Math.abs(z - ex.z) > ex.d / 2,
  );
  const broadSpots = spots.filter((t) => t.kind !== 'conifer');
  const coniferSpots = spots.filter((t) => t.kind === 'conifer');

  // named consts, tuned once against the hero plate so silhouettes read like
  // the original hand-authored tree() prop
  const BROAD_TRUNK_MIN_H = 1; // broad trunk height floor (m)
  const BROAD_TRUNK_R_FACTOR = 1.6; // trunk shrinks as canopy radius grows
  const BROAD_CANOPY_Y_FACTOR = 0.72; // crown center above trunk top, in canopy radii
  const TRUNK_H_CAP = 2.2; // shared trunk height cap (all trees)
  const TRUNK_H_FACTOR = 0.22; // trunk height ≈ h * this, capped
  const CONIFER_TRUNK_H_CAP = 1.2; // conifer trunk is short — cones start low
  const CONIFER_TRUNK_H_FACTOR = 0.15;
  const CONIFER_Y_SCALE_DIVISOR = 2.5; // conifer height growth per extra meter of h
  // deterministic per-tree variety (Task 14) — visual-only, derived from x/z
  // so full trees and impostors agree per spot and LOD swaps never pop
  const SQUASH_MIN = 0.85; // y-squash band for crown/impostor silhouettes
  const SQUASH_SPAN = 0.3; // 0.85 – 1.15
  const TINT_L_SPREAD = 0.16; // lightness varies ±0.08

  const trunkGeo = new THREE.CylinderGeometry(0.14, 0.2, 1, 6);
  const broadCanopyGeo = broadCanopyGeometry();
  const coniferGeo = coniferGeometry();
  const impostorGeo = broadImpostorGeometry();
  const impostorConiferGeo = coniferImpostorGeometry();

  // Zero-instance InstancedMeshes create zero-size GPU instance buffers, which
  // WebGPU rejects (GPUValidationError — the bug class already hit and fixed
  // in windows.ts, Task 6). windows.ts could just skip an empty mesh, but
  // these names can't be: Task 10's LOD does getObjectByName on all of
  // treeCanopies/treeConifers/treeImpostors/treeImpostorsConifer/treeTrunks
  // regardless of which kinds a given bake actually contains. So allocate at
  // least 1 instance (non-zero buffer) and set `.count` to the real number
  // afterward — a mesh with count 0 draws nothing but still exists and still
  // has a valid buffer.
  const broad = new THREE.InstancedMesh(broadCanopyGeo, clayMat(kswCity.treeGreen).clone(), Math.max(1, broadSpots.length));
  const conifers = new THREE.InstancedMesh(coniferGeo, clayMat(kswCity.woodGreen).clone(), Math.max(1, coniferSpots.length));
  const trunks = new THREE.InstancedMesh(trunkGeo, clayMat(kswCity.treeTrunk), Math.max(1, spots.length));
  const impostors = new THREE.InstancedMesh(impostorGeo, clayMat(kswCity.treeGreen).clone(), Math.max(1, broadSpots.length));
  const impostorsConifer = new THREE.InstancedMesh(
    impostorConiferGeo,
    clayMat(kswCity.woodGreen).clone(),
    Math.max(1, coniferSpots.length),
  );
  broad.count = broadSpots.length;
  conifers.count = coniferSpots.length;
  trunks.count = spots.length;
  impostors.count = broadSpots.length;
  impostorsConifer.count = coniferSpots.length;
  broad.name = 'treeCanopies';
  conifers.name = 'treeConifers';
  trunks.name = 'treeTrunks';
  impostors.name = 'treeImpostors';
  impostorsConifer.name = 'treeImpostorsConifer';

  const m = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const baseGreen = new THREE.Color(kswCity.treeGreen);
  const baseWood = new THREE.Color(kswCity.woodGreen);
  const tint = new THREE.Color();

  // per-spot deterministic variety: hue nudge (hh), y-squash + lightness (hv)
  const spotHashes = (x: number, z: number): { hh: number; squash: number; dL: number } => {
    const hh = hash01(x * 3.1 + z * 7.7);
    const hv = hash01(x * 7.3 + z * 3.9);
    return { hh, squash: SQUASH_MIN + SQUASH_SPAN * hv, dL: (hash01(x * 5.7 + z * 11.3) - 0.5) * TINT_L_SPREAD };
  };

  let bi = 0;
  for (const spot of broadSpots) {
    const { x, z, h, r } = spot;
    const { hh, squash, dL } = spotHashes(x, z);
    const trunkH = Math.max(BROAD_TRUNK_MIN_H, h - r * BROAD_TRUNK_R_FACTOR);
    const scale = r / 0.75;
    m.compose(
      new THREE.Vector3(x, trunkH + r * BROAD_CANOPY_Y_FACTOR, z),
      q,
      new THREE.Vector3(scale, scale * squash, scale),
    );
    broad.setMatrixAt(bi, m);
    // impostor: SAME transform + tint as the full crown → LOD swap can't pop
    impostors.setMatrixAt(bi, m);
    tint.copy(baseGreen).offsetHSL((hh - 0.5) * 0.04, 0, dL);
    broad.setColorAt(bi, tint);
    impostors.setColorAt(bi, tint);
    bi++;
  }

  let ci = 0;
  for (const spot of coniferSpots) {
    const { x, z, h, r } = spot;
    const { hh, squash, dL } = spotHashes(x, z);
    const trunkH = Math.min(CONIFER_TRUNK_H_CAP, h * CONIFER_TRUNK_H_FACTOR);
    const scaleXZ = r / 0.75;
    const scaleY = ((h - 1) / CONIFER_Y_SCALE_DIVISOR) * squash;
    m.compose(new THREE.Vector3(x, trunkH, z), q, new THREE.Vector3(scaleXZ, scaleY, scaleXZ));
    conifers.setMatrixAt(ci, m);
    impostorsConifer.setMatrixAt(ci, m);
    tint.copy(baseWood).offsetHSL((hh - 0.5) * 0.04, 0, dL);
    conifers.setColorAt(ci, tint);
    impostorsConifer.setColorAt(ci, tint);
    ci++;
  }

  for (let i = 0; i < spots.length; i++) {
    const { x, z, h } = spots[i];
    const trunkH = Math.min(TRUNK_H_CAP, h * TRUNK_H_FACTOR);
    m.compose(new THREE.Vector3(x, trunkH / 2, z), q, new THREE.Vector3(1, trunkH, 1));
    trunks.setMatrixAt(i, m);
  }

  broad.castShadow = opts.treeShadows ?? false;
  broad.receiveShadow = true;
  conifers.castShadow = opts.treeShadows ?? false;
  conifers.receiveShadow = true;
  trunks.castShadow = false;
  trunks.receiveShadow = true;
  for (const imp of [impostors, impostorsConifer]) {
    imp.castShadow = false;
    imp.receiveShadow = true;
    imp.visible = false;
  }

  group.add(broad);
  group.add(conifers);
  group.add(trunks);
  group.add(impostors);
  group.add(impostorsConifer);
  return group;
}

// Original tree form (props.ts `tree()`): trunk + 4 clay puffs — merged into
// one canopy geometry so thousands instance in a single draw call. Conifers
// are a two-cone stack in the same vocabulary.
// crown puff layout [x, y, z, r] — shared by the full canopy (detail 2) and
// the far impostor (detail 0) so both LODs carry the same silhouette
const BROAD_PUFFS: Array<[number, number, number, number]> = [
  [0, 0.5, 0, 0.75],
  [0.45, 0.15, 0.2, 0.48],
  [-0.4, 0.25, -0.18, 0.52],
  [0.1, 0.1, -0.42, 0.42],
];

function mergedPuffs(detail: number): THREE.BufferGeometry {
  const geos = BROAD_PUFFS.map(([x, y, z, r]) => {
    const g = new THREE.IcosahedronGeometry(r, detail);
    g.translate(x, y, z);
    return g;
  });
  return mergeGeos(geos);
}

function broadCanopyGeometry(): THREE.BufferGeometry {
  return mergedPuffs(2);
}

function coniferGeometry(): THREE.BufferGeometry {
  const a = new THREE.ConeGeometry(0.75, 1.4, 8);
  a.translate(0, 0.5, 0);
  const b = new THREE.ConeGeometry(0.55, 1.1, 8);
  b.translate(0, 1.15, 0);
  return mergeGeos([a, b]);
}

// Far-LOD impostors (Task 14): same silhouettes, minimum polygons. The broad
// impostor merges the SAME 4-puff layout as broadCanopyGeometry but at
// Icosahedron detail 0 (20 faces per puff) so the far ring shows stepped
// multi-lobe crowns instead of uniform balls; the conifer impostor is one
// low-segment cone spanning the two-cone stack's envelope (y −0.2 … 1.7).
function broadImpostorGeometry(): THREE.BufferGeometry {
  return mergedPuffs(0);
}

function coniferImpostorGeometry(): THREE.BufferGeometry {
  const cone = new THREE.ConeGeometry(0.75, 1.9, 6);
  cone.translate(0, 0.75, 0);
  return cone;
}
