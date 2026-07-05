// src/diorama/ksw/geo/treeArchetypes.ts
// Deterministic procedural tree archetypes in the clay vocabulary: seeded
// branch skeleton + puff crown for broadleaf families, stacked cones for
// conifers. Normalized (y 0..1) so instances scale to real h/r meters.
// Geometry contract (Tasks 3/4/5 depend on it): position/normal/aPuff(vec4),
// aPuff = (cx,cy,cz,puffIndex) for crown vertices, (0,0,0,-1) for wood.
import * as THREE from 'three/webgpu';
import { mergeGeometries, mergeVertices } from 'three/addons/utils/BufferGeometryUtils.js';

export type TreeFamily = 'spreading' | 'oval' | 'tall' | 'conic' | 'slender';
export const BROAD_FAMILIES = ['spreading', 'oval', 'tall'] as const satisfies readonly TreeFamily[];
export const CONIFER_FAMILIES = ['conic', 'slender'] as const satisfies readonly TreeFamily[];
export const SEEDS_PER_FAMILY = 4;

// splitmix32-style avalanche so nearby inputs decorrelate (stationary-age-seed
// lesson: naive hashes cluster in high bits)
export function hash01(n: number): number {
  let x = (n * 0x9e3779b9) | 0;
  x = Math.imul(x ^ (x >>> 16), 0x21f0aaad);
  x = Math.imul(x ^ (x >>> 15), 0x735a2d97);
  x = x ^ (x >>> 15);
  return (x >>> 0) / 4294967296;
}

export type TreeArchetype = {
  family: TreeFamily;
  seed: number;
  geometry: THREE.BufferGeometry;
  crownRadius: number; // max horizontal radius in normalized units
  crownBaseY: number; // normalized y where the crown starts
};

type Rng = () => number;

type FamilyParams = {
  crownBaseY: number;
  crownRadius: number; // target horizontal reach (pre-normalize ≈ post-normalize)
  puffMax: number; // cap on crown puff count (anchors provide the minimum)
  puffR: [min: number, max: number]; // puff radius band (normalized)
  branches: [min: number, max: number]; // level-1 branch count
  elevation: [min: number, max: number]; // level-1 branch pitch above horizontal (rad)
  apexY: number; // pre-normalize height of the apex puff center
  envelope: (t: number) => number; // horizontal reach at crown height fraction t (0..1), ×crownRadius
};

// Family silhouettes: spreading = wide/flat, oval = egg, tall = narrow-tall,
// conic = classic christmas tree, slender = tight spire. Broad families
// differ in crown base, reach, branch pitch and envelope curvature.
const FAMILY_PARAMS: Record<TreeFamily, FamilyParams> = {
  // Puff radii are deliberately LARGE relative to the branch-tip offsets (puff
  // radius ≈ crownRadius, offsets ≈ crownRadius) so the 6–12 puffs heavily
  // overlap into ONE cohesive clay mass that swallows the branch skeleton —
  // the deleted nature.ts BROAD_PUFFS vocabulary (radii 0.42..0.75 at ±0.45
  // offsets, i.e. radius ≈ offset). Only the trunk + a few branch stubs below
  // the crown base should read as bare wood.
  spreading: {
    crownBaseY: 0.35,
    crownRadius: 0.55,
    puffMax: 12,
    // radius floor kept HIGH so even the smallest puff still overlaps its
    // neighbours after the per-instance jitter shrink — no gaps → no exposed
    // branches. Raised elevation (below) also keeps the low branches tucked up
    // inside the crown mass rather than fanning out flat beneath it.
    puffR: [0.42, 0.54],
    branches: [4, 5],
    elevation: [0.35, 0.6],
    apexY: 0.78,
    // fat mid, flattened top — widest just above the crown base
    envelope: (t) => 1 - 0.45 * Math.abs(t - 0.35) - 0.25 * t,
  },
  oval: {
    crownBaseY: 0.3,
    crownRadius: 0.4,
    puffMax: 10,
    puffR: [0.36, 0.46],
    branches: [3, 5],
    elevation: [0.5, 0.9],
    apexY: 0.86,
    // ellipse: widest at mid-crown, closes at both ends
    envelope: (t) => Math.sqrt(Math.max(0.05, 1 - (2 * t - 1) ** 2)),
  },
  tall: {
    crownBaseY: 0.25,
    crownRadius: 0.3,
    puffMax: 9,
    puffR: [0.3, 0.38],
    branches: [3, 4],
    elevation: [0.9, 1.2],
    apexY: 0.94,
    // narrow column, gently tapering toward the top
    envelope: (t) => 0.9 - 0.35 * t,
  },
  // conifers ignore branches/elevation/apexY (cone stacks, no skeleton)
  conic: {
    crownBaseY: 0.1,
    crownRadius: 0.3,
    puffMax: 4, // = max cone count
    puffR: [0, 0],
    branches: [0, 0],
    elevation: [0, 0],
    apexY: 1,
    envelope: (t) => 1 - t,
  },
  slender: {
    crownBaseY: 0.08,
    crownRadius: 0.2,
    puffMax: 3,
    puffR: [0, 0],
    branches: [0, 0],
    elevation: [0, 0],
    apexY: 1,
    envelope: (t) => 1 - t,
  },
};

const TRUNK_R: [number, number] = [0.022, 0.035]; // trunk radius band (normalized)
const BRANCH_R = 0.014; // level-1 branch radius; level 2 is thinner
const BRANCH_LEN: [number, number] = [0.24, 0.38]; // level-1 length band (kept
// short enough that the branch stubs stay tucked inside the puff crown mass)
const L2_LEN_FACTOR = 0.5; // level-2 length relative to its parent
const PUFF_DETAIL = 1; // icosahedron detail (42 welded verts/puff)
const STRUT_SEGMENTS = 5; // branch cylinders — open-ended, ends hide in puffs/trunk
const WOOD_FLAG = -1; // aPuff.w for trunk/branch vertices

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
const UP = new THREE.Vector3(0, 1, 0);

// stamp aPuff on every vertex of one part BEFORE merging (mergeGeometries
// concatenates same-named attributes)
function stampPuff(g: THREE.BufferGeometry, cx: number, cy: number, cz: number, w: number): void {
  const n = g.getAttribute('position').count;
  const a = new Float32Array(n * 4);
  for (let i = 0; i < n; i++) {
    a[i * 4] = cx;
    a[i * 4 + 1] = cy;
    a[i * 4 + 2] = cz;
    a[i * 4 + 3] = w;
  }
  g.setAttribute('aPuff', new THREE.Float32BufferAttribute(a, 4));
}

// all parts must carry the same attribute set for mergeGeometries; uvs are
// unused and would break puff vertex welding at seams
function dropUv(g: THREE.BufferGeometry): THREE.BufferGeometry {
  g.deleteAttribute('uv');
  return g;
}

// branch cylinder from `from` to `to`, open-ended (ends hide inside wood/puffs)
function strut(from: THREE.Vector3, to: THREE.Vector3, r: number): THREE.BufferGeometry {
  const dir = to.clone().sub(from);
  const len = dir.length();
  const g = dropUv(new THREE.CylinderGeometry(r * 0.7, r, len, STRUT_SEGMENTS, 1, true));
  g.translate(0, len / 2, 0);
  const q = new THREE.Quaternion().setFromUnitVectors(UP, dir.normalize());
  g.applyQuaternion(q);
  g.translate(from.x, from.y, from.z);
  stampPuff(g, 0, 0, 0, WOOD_FLAG);
  return g;
}

// welded icosahedron puff (42 verts at detail 1) centered at c
function puff(c: THREE.Vector3, r: number, puffIndex: number): THREE.BufferGeometry {
  const g = mergeVertices(dropUv(new THREE.IcosahedronGeometry(r, PUFF_DETAIL)));
  g.computeVertexNormals();
  g.translate(c.x, c.y, c.z);
  stampPuff(g, c.x, c.y, c.z, puffIndex);
  return g;
}

function trunk(h: number, rng: Rng): THREE.BufferGeometry {
  const r = lerp(TRUNK_R[0], TRUNK_R[1], rng());
  const g = dropUv(new THREE.CylinderGeometry(r * 0.75, r, h, 7));
  g.translate(0, h / 2, 0);
  stampPuff(g, 0, 0, 0, WOOD_FLAG);
  return g;
}

// clamp a point's horizontal distance to the family envelope at its height
function clampToEnvelope(p: THREE.Vector3, params: FamilyParams): THREE.Vector3 {
  const t = Math.min(1, Math.max(0, (p.y - params.crownBaseY) / (1 - params.crownBaseY)));
  const maxR = params.envelope(t) * params.crownRadius;
  const d = Math.hypot(p.x, p.z);
  if (d > maxR && d > 1e-6) {
    p.x *= maxR / d;
    p.z *= maxR / d;
  }
  return p;
}

// broadleaf: trunk + 2-level branch skeleton; branch tips + trunk top + apex
// are the puff anchors, so every puff sits ON the skeleton (no floaters)
function buildBroad(params: FamilyParams, rng: Rng): THREE.BufferGeometry {
  const parts: THREE.BufferGeometry[] = [trunk(params.crownBaseY, rng)];
  const top = new THREE.Vector3(0, params.crownBaseY, 0);

  const anchors: THREE.Vector3[] = [];
  // apex first so slicing to puffMax never drops the crown top; a central
  // leader strut ties its puff to the skeleton (no floating crown top)
  const apex = new THREE.Vector3((rng() - 0.5) * 0.06, params.apexY, (rng() - 0.5) * 0.06);
  anchors.push(apex);
  anchors.push(top.clone());
  parts.push(strut(top, apex, BRANCH_R * 0.8));

  const n = Math.round(lerp(params.branches[0], params.branches[1], rng()));
  for (let j = 0; j < n; j++) {
    const az = ((j + 0.3 + rng() * 0.4) / n) * Math.PI * 2;
    const elev = lerp(params.elevation[0], params.elevation[1], rng());
    const len = lerp(BRANCH_LEN[0], BRANCH_LEN[1], rng());
    const dir = new THREE.Vector3(
      Math.cos(az) * Math.cos(elev),
      Math.sin(elev),
      Math.sin(az) * Math.cos(elev),
    );
    const tip = clampToEnvelope(top.clone().addScaledVector(dir, len), params);
    parts.push(strut(top, tip, BRANCH_R));
    anchors.push(tip.clone());

    // level 2: 1–2 short continuations per level-1 tip
    const n2 = 1 + (rng() < 0.5 ? 1 : 0);
    for (let k = 0; k < n2; k++) {
      const az2 = az + (rng() - 0.5) * 1.6;
      const elev2 = elev + lerp(0.15, 0.5, rng()); // curl upward
      const len2 = len * L2_LEN_FACTOR * lerp(0.7, 1.1, rng());
      const dir2 = new THREE.Vector3(
        Math.cos(az2) * Math.cos(elev2),
        Math.sin(elev2),
        Math.sin(az2) * Math.cos(elev2),
      );
      const tip2 = clampToEnvelope(tip.clone().addScaledVector(dir2, len2), params);
      parts.push(strut(tip, tip2, BRANCH_R * 0.7));
      anchors.push(tip2.clone());
    }
  }

  // Puffs cluster into ONE cohesive crown mass. Instances scale non-uniformly
  // (XZ by r/crownRadius ≈ 2-4, Y by h ≈ 3-16), so a puff at a far branch tip
  // stretches into a thin vertical sliver that no longer overlaps its
  // neighbours horizontally — the branch skeleton then shows through (the
  // "naked-skeleton" failure). To keep the crown reading as a single chunky
  // clay mass under ANY h/r aspect, pull each puff's CENTER a fraction of the
  // way back toward the crown's central axis (x=z=0) so the large puffs pile
  // up and overlap horizontally, while the full-reach struts stay as the
  // branch stubs poking out of the mass. The apex (i=0) and trunk-top (i=1)
  // anchors are already near-axial, so this mostly tightens the outer tips.
  const PUFF_INSET = 0.55; // keep 55% of the horizontal offset; 0 = all on axis
  const puffCount = Math.min(anchors.length, params.puffMax);
  for (let i = 0; i < puffCount; i++) {
    const r = lerp(params.puffR[0], params.puffR[1], rng());
    const a = anchors[i];
    const c = new THREE.Vector3(a.x * PUFF_INSET, a.y, a.z * PUFF_INSET);
    parts.push(puff(c, r, i));
  }
  return mergeGeometries(parts)!;
}

// conifer: short trunk + stacked cones, no branch skeleton. Cone vertices get
// aPuff = (0, coneCenterY, 0, coneIndex) so jitter/wind keep per-part identity.
function buildConifer(params: FamilyParams, rng: Rng): THREE.BufferGeometry {
  const parts: THREE.BufferGeometry[] = [trunk(params.crownBaseY + 0.02, rng)];
  const nMin = params.puffMax - 1;
  const nCones = nMin + (rng() < 0.5 ? 1 : 0);
  const span = 1 - params.crownBaseY;
  for (let i = 0; i < nCones; i++) {
    const t = i / nCones;
    const baseY = params.crownBaseY + span * t * 0.88; // overlap the cone below
    const h = (span / nCones) * lerp(1.25, 1.55, rng());
    const r = params.crownRadius * (1 - t * 0.62) * lerp(0.85, 1.05, rng());
    const g = dropUv(new THREE.ConeGeometry(r, h, 8));
    const cy = baseY + h / 2;
    g.translate(0, cy, 0);
    stampPuff(g, 0, cy, 0, i);
    parts.push(g);
  }
  return mergeGeometries(parts)!;
}

// scale/translate so min y = 0, max y = 1 (uniform scale — instances rescale
// xz by r/crownRadius anyway) and transform aPuff centers with the SAME map
function normalizeTree(geo: THREE.BufferGeometry): { crownRadius: number; s: number; minY: number } {
  const pos = geo.getAttribute('position');
  const p = pos.array as Float32Array;
  let minY = Infinity;
  let maxY = -Infinity;
  for (let i = 1; i < p.length; i += 3) {
    minY = Math.min(minY, p[i]);
    maxY = Math.max(maxY, p[i]);
  }
  const s = 1 / (maxY - minY);
  let crownRadius = 0;
  for (let i = 0; i < p.length; i += 3) {
    p[i] *= s;
    p[i + 1] = (p[i + 1] - minY) * s;
    p[i + 2] *= s;
    crownRadius = Math.max(crownRadius, Math.hypot(p[i], p[i + 2]));
  }
  pos.needsUpdate = true;
  const ap = geo.getAttribute('aPuff').array as Float32Array;
  for (let i = 0; i < ap.length; i += 4) {
    if (ap[i + 3] < 0) continue; // wood keeps (0,0,0,-1)
    ap[i] *= s;
    ap[i + 1] = (ap[i + 1] - minY) * s;
    ap[i + 2] *= s;
  }
  geo.getAttribute('aPuff').needsUpdate = true;
  geo.computeBoundingSphere();
  return { crownRadius, s, minY };
}

const ALL_FAMILIES = [...BROAD_FAMILIES, ...CONIFER_FAMILIES] as readonly TreeFamily[];

export function buildArchetype(family: TreeFamily, seed: number): TreeArchetype {
  const params = FAMILY_PARAMS[family];
  const familyIdx = ALL_FAMILIES.indexOf(family);
  const rng: Rng = (() => {
    let i = 0;
    const base = familyIdx * 1013 + seed * 7919;
    return () => hash01(base + i++);
  })();
  const geometry = CONIFER_FAMILIES.includes(family as (typeof CONIFER_FAMILIES)[number])
    ? buildConifer(params, rng)
    : buildBroad(params, rng);
  const { crownRadius, s, minY } = normalizeTree(geometry);
  return { family, seed, geometry, crownRadius, crownBaseY: (params.crownBaseY - minY) * s };
}

let cache: TreeArchetype[] | null = null;
export function allArchetypes(): TreeArchetype[] {
  if (!cache) {
    cache = [];
    for (const f of ALL_FAMILIES) {
      for (let s = 0; s < SEEDS_PER_FAMILY; s++) cache.push(buildArchetype(f, s));
    }
  }
  return cache;
}

// stable spot→archetype pick: quantized world xz → avalanche hash → id band
// for the tree's kind (broad ids come first, conifer ids after)
export function archetypeIndexFor(x: number, z: number, kind: 'broad' | 'conifer'): number {
  const h = hash01(Math.round(x * 8) * 92837111 + Math.round(z * 8) * 689287499);
  const broadN = BROAD_FAMILIES.length * SEEDS_PER_FAMILY;
  const conifN = CONIFER_FAMILIES.length * SEEDS_PER_FAMILY;
  return kind === 'broad' ? Math.floor(h * broadN) : broadN + Math.floor(h * conifN);
}
