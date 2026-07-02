// src/diorama/ksw/geo/cityMassing.ts
// Renders the baked swisstopo LoD2 city as clay massing: every wall surface
// of every building merged into ONE mesh, every roof surface into another —
// two draw calls for ~800 real buildings, real roof shapes included. Each
// building gets a small deterministic tint around the base clay colour (baked
// into vertex colours, so the merge stays intact) — the town reads as many
// distinct handmade buildings rather than one flat block, still in the family
// of the hero hospital's palette.
import * as THREE from 'three/webgpu';
import { clay, kswPalette, palette } from '../../designTokens';
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
  c.setHSL(hsl.h + (a - 0.5) * 0.025, hsl.s * (0.92 + 0.16 * b), hsl.l * (0.86 + 0.28 * a));
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
  return group;
}
