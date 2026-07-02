// src/diorama/ksw/geo/cityMassing.ts
// Renders the baked swisstopo LoD2 city as clay massing: every wall surface
// of every building merged into ONE mesh, every roof surface into another —
// two draw calls for ~800 real buildings, real roof shapes included. Each
// building gets a small deterministic tint around the base clay colour (baked
// into vertex colours, so the merge stays intact) — the town reads as many
// distinct handmade buildings rather than one flat block, still in the family
// of the hero hospital's palette.
import * as THREE from 'three/webgpu';
import { clay, kswCityStyle, kswPalette, palette } from '../../designTokens';
import { clayMat } from '../props';
import type { BakedBuilding, BakedMesh } from './geoData';

// deterministic 0..1 hash (no RNG — same city every load)
function hash01(n: number): number {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
}

// small tint around a base colour, kept inside the clay family: gentle
// lightness spread + a whisper of hue drift, saturation barely touched.
function tintFor(base: THREE.Color, seed: number): THREE.Color {
  const hsl = { h: 0, s: 0, l: 0 };
  base.getHSL(hsl);
  const a = hash01(seed);
  const b = hash01(seed * 1.37 + 4.2);
  const c = new THREE.Color();
  c.setHSL(hsl.h + (a - 0.5) * kswCityStyle.tintHue, hsl.s * (0.96 + 0.08 * b), hsl.l * (1 - kswCityStyle.tintL + 2 * kswCityStyle.tintL * a));
  return c;
}

function mergeTinted(buildings: BakedBuilding[], pick: (b: BakedBuilding) => BakedMesh, base: number): THREE.BufferGeometry {
  let vtx = 0;
  let tri = 0;
  for (const b of buildings) {
    const p = pick(b);
    vtx += p.pos.length / 3;
    tri += p.idx.length;
  }
  const positions = new Float32Array(vtx * 3);
  const colors = new Float32Array(vtx * 3);
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  const baseColor = new THREE.Color(base);
  let vo = 0;
  let io = 0;
  let seed = 0;
  for (const b of buildings) {
    const p = pick(b);
    const base3 = vo / 3;
    const tint = tintFor(baseColor, ++seed);
    for (let i = 0; i < p.pos.length; i += 3) {
      positions[vo + i] = p.pos[i] / 100; // cm → m
      positions[vo + i + 1] = p.pos[i + 1] / 100;
      positions[vo + i + 2] = p.pos[i + 2] / 100;
      colors[vo + i] = tint.r;
      colors[vo + i + 1] = tint.g;
      colors[vo + i + 2] = tint.b;
    }
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base3 + p.idx[i];
    vo += p.pos.length;
    io += p.idx.length;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.setIndex(new THREE.BufferAttribute(indices, 1));
  geo.computeVertexNormals();
  return geo;
}

// Merge a set of pre-baked parts (cm-int positions) into one geometry, no
// per-vertex tint — used for uniform-colour trim bands (plinth/eave).
function mergeBakedParts(parts: { pos: number[]; idx: number[] }[]): THREE.BufferGeometry {
  let vtx = 0;
  let tri = 0;
  for (const p of parts) {
    vtx += p.pos.length / 3;
    tri += p.idx.length;
  }
  const positions = new Float32Array(vtx * 3);
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  let vo = 0;
  let io = 0;
  for (const p of parts) {
    const base3 = vo / 3;
    for (let i = 0; i < p.pos.length; i++) positions[vo + i] = p.pos[i] / 100; // cm → m
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base3 + p.idx[i];
    vo += p.pos.length;
    io += p.idx.length;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setIndex(new THREE.BufferAttribute(indices, 1));
  geo.computeVertexNormals();
  return geo;
}

// horizontal band following a footprint ring: a short extruded wall strip,
// outset from the facade — the original's plinth/eave-trim language.
function ringBand(fp: number[][], y0: number, y1: number, out: number): { pos: number[]; idx: number[] } {
  const pos: number[] = [];
  const idx: number[] = [];
  const n = fp.length;
  for (let i = 0; i < n; i++) {
    const [ax, az] = fp[i];
    const [bx, bz] = fp[(i + 1) % n];
    const ex = bx - ax;
    const ez = bz - az;
    const len = Math.hypot(ex, ez);
    if (len < 0.05) continue;
    const ox = (-ez / len) * out;
    const oz = (ex / len) * out;
    const base = pos.length / 3;
    // both sides + top so the band reads as a solid rim from every angle
    pos.push(
      ax + ox, y0, az + oz, bx + ox, y0, bz + oz, bx + ox, y1, bz + oz, ax + ox, y1, az + oz,
      ax - ox, y0, az - oz, bx - ox, y0, bz - oz, bx - ox, y1, bz - oz, ax - ox, y1, az - oz,
      ax + ox, y1, az + oz, bx + ox, y1, bz + oz, bx - ox, y1, bz - oz, ax - ox, y1, az - oz,
    );
    idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
    idx.push(base + 5, base + 4, base + 7, base + 5, base + 7, base + 6);
    idx.push(base + 8, base + 9, base + 10, base + 8, base + 10, base + 11);
  }
  return { pos: pos.map((v) => Math.round(v * 100)), idx };
}

// A clay material whose diffuse comes from the per-vertex tint (color = white
// so vertexColors passes the baked colour straight through). Cloned from the
// shared clayMat so the sheen/roughness recipe matches the hero, without
// mutating the cached hero material.
function tintedClay(base: number): THREE.MeshPhysicalMaterial {
  const m = clayMat(base).clone();
  m.vertexColors = true;
  m.color = new THREE.Color(palette.trueWhite);
  m.sheenColor = new THREE.Color(base).lerp(new THREE.Color(palette.trueWhite), clay.sheenLerp);
  // Baked meshes are vertex-welded for JSON size (scripts/geo/lib/transform.mjs),
  // which makes computeVertexNormals() smooth-shade across welded seams —
  // ridges/eaves would lose the crisp clay facets. Flat shading restores
  // per-face faceted normals at draw time without unwelding the JSON.
  m.flatShading = true;
  return m;
}

export function buildCityMassing(buildings: BakedBuilding[]): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityMassing';

  const make = (name: string, pick: (b: BakedBuilding) => BakedMesh, base: number): void => {
    const mesh = new THREE.Mesh(mergeTinted(buildings, pick, base), tintedClay(base));
    mesh.name = name;
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    group.add(mesh);
  };

  make('cityWalls', (b) => b.wall, palette.creamBase);
  make('cityRoofs', (b) => b.roof, kswPalette.roofClay);

  const plinths = buildings.map((b) => ringBand(b.footprint, -kswCityStyle.plinthSink, kswCityStyle.plinthH, kswCityStyle.plinthOut));
  const eaves = buildings.map((b) => {
    const eave = Math.max(b.height - 2, kswCityStyle.plinthH + 0.5); // eave≈wall top; height is ridge — band sits just below
    return ringBand(b.footprint, eave - kswCityStyle.eaveBandH, eave, kswCityStyle.eaveBandOut);
  });

  const plinthMesh = new THREE.Mesh(mergeBakedParts(plinths), clayMat(palette.white));
  plinthMesh.name = 'cityPlinths';
  plinthMesh.castShadow = false;
  plinthMesh.receiveShadow = true;
  group.add(plinthMesh);

  const eaveMesh = new THREE.Mesh(mergeBakedParts(eaves), clayMat(kswPalette.roofTrim));
  eaveMesh.name = 'cityEaves';
  eaveMesh.castShadow = false;
  eaveMesh.receiveShadow = true;
  group.add(eaveMesh);

  return group;
}
