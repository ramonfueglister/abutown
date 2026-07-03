// src/diorama/ksw/geo/cityMassing.ts
// Renders the baked swisstopo LoD2 city as clay massing: every wall surface
// of every building merged into ONE mesh, every roof surface into another —
// two draw calls for ~800 real buildings, real roof shapes included. Each
// building gets a small deterministic tint around the base clay colour (baked
// into vertex colours, so the merge stays intact) — the town reads as many
// distinct handmade buildings rather than one flat block, still in the family
// of the hero hospital's palette.
import * as THREE from 'three/webgpu';
import {
  Fn, attribute, float, max as tslMax, min as tslMin, mix, positionWorld, smoothstep, uniform, vec3,
} from 'three/tsl';
import { clay, kswCityStyle, kswPalette, kswS3, palette } from '../../designTokens';
import { NIGHT_WINDOW_SHARE } from '../staticBatch';
import { clayMat } from '../props';
import type { BakedBuilding, BakedMesh, BakedWallMesh } from './geoData';

// Baked facade-UV quantization factor: 1 unit = 0.2 m. MUST match
// scripts/geo/lib/transform.mjs FUV_PER_M (can't import a .mjs into src). The
// bake stores round(metres × FUV_PER_M); the runtime divides it back to metres.
const FUV_PER_M = 5;

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

export function mergeTinted(buildings: BakedBuilding[], pick: (b: BakedBuilding) => BakedMesh, base: number): THREE.BufferGeometry {
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

// Wall merge (Task 13): like mergeTinted, but also carries the baked facade-UV
// attribute (fuv, dm ints → metres) and a per-vertex eave height so the TSL
// facade shader can draw an eave-clamped window raster. One extra vec2 + float
// per vertex; the tint stays the base clay colour (window tone is mixed in the
// shader, not baked).
export function mergeWalls(buildings: BakedBuilding[], base: number): THREE.BufferGeometry {
  let vtx = 0;
  let tri = 0;
  for (const b of buildings) {
    vtx += b.wall.pos.length / 3;
    tri += b.wall.idx.length;
  }
  const positions = new Float32Array(vtx * 3);
  const colors = new Float32Array(vtx * 3);
  const fuv = new Float32Array(vtx * 2); // metres (u, v)
  const eave = new Float32Array(vtx); // metres, per vertex (constant per building)
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  const baseColor = new THREE.Color(base);
  let vo = 0;
  let uo = 0;
  let io = 0;
  let seed = 0;
  for (const b of buildings) {
    const p: BakedWallMesh = b.wall;
    const base3 = vo / 3;
    const tint = tintFor(baseColor, ++seed);
    const nVerts = p.pos.length / 3;
    for (let i = 0; i < p.pos.length; i += 3) {
      positions[vo + i] = p.pos[i] / 100;
      positions[vo + i + 1] = p.pos[i + 1] / 100;
      positions[vo + i + 2] = p.pos[i + 2] / 100;
      colors[vo + i] = tint.r;
      colors[vo + i + 1] = tint.g;
      colors[vo + i + 2] = tint.b;
    }
    for (let v = 0; v < nVerts; v++) {
      fuv[uo + v * 2] = p.fuv[v * 2] / FUV_PER_M; // 2-dm units → m
      fuv[uo + v * 2 + 1] = p.fuv[v * 2 + 1] / FUV_PER_M;
      eave[uo / 2 + v] = b.eaveH;
    }
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base3 + p.idx[i];
    vo += p.pos.length;
    uo += nVerts * 2;
    io += p.idx.length;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.setAttribute('fuv', new THREE.BufferAttribute(fuv, 2));
  geo.setAttribute('eaveH', new THREE.BufferAttribute(eave, 1));
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

// A single footprint's trim band (plinth/eave) as a ready BufferGeometry —
// used by kswCampus (T18) to build the split main-building trim bands with the
// same recipe buildCityMassing uses internally.
export function ringBandParts(fp: number[][], y0: number, y1: number, out: number): THREE.BufferGeometry {
  return mergeBakedParts([ringBand(fp, y0, y1, out)]);
}

// A clay material whose diffuse comes from the per-vertex tint (color = white
// so vertexColors passes the baked colour straight through). Cloned from the
// shared clayMat so the sheen/roughness recipe matches the hero, without
// mutating the cached hero material.
export function tintedClay(base: number): THREE.MeshPhysicalMaterial {
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

// TSL facade shader (Task 13): a procedural window raster drawn IN the wall,
// replacing the 154k instanced window boxes. Grid tokens from kswCityStyle
// (storeyH, windowSpacing, windowW/H, sillFrac). Inside a window cell the clay
// mixes toward palette.glass with a white frame border and a slight edge
// darkening for inset depth. Clamp: no window whose storey TOP would sit above
// eaveH−0.2, so nothing ever paints above the real eave (root cause B). At
// night (lampGlow) a deterministic per-cell hash < NIGHT_WINDOW_SHARE lights
// the glass warm via emissiveNode. `facadeDetail` (0/1 uniform, driven by the
// LOD ring) fades the whole raster out for the far ring.
type FacadeMaterial = THREE.MeshPhysicalNodeMaterial & {
  facadeDetail: ReturnType<typeof uniform>;
};

// Cutaway-enabled facade material (T18): additionally carries `cutH` +
// `upperFade` uniforms so the MAIN KSW building can be sliced open like a
// dollhouse. cutH = 1e6 / upperFade = 1 at rest → byte-identical to the plain
// facade material (the discard/seam nodes are pure no-ops at those values).
export type CutawayFacadeMaterial = FacadeMaterial & {
  cutH: ReturnType<typeof uniform>;
  upperFade: ReturnType<typeof uniform>;
};

export function facadeMaterial(base: number, opts: { lampGlow: boolean; cutaway?: boolean }): FacadeMaterial {
  const s = kswCityStyle;
  const glass = new THREE.Color(palette.glass);
  const frameCol = new THREE.Color(palette.white);
  const warm = new THREE.Color(0xffd9a0);

  const m = new THREE.MeshPhysicalNodeMaterial({
    color: palette.trueWhite, // multiplied by per-vertex tint
    roughness: clay.roughness,
    metalness: clay.metalness,
    vertexColors: true,
    flatShading: true,
  });
  m.sheenRoughness = clay.sheenRoughness;
  m.sheenColor = new THREE.Color(base).lerp(new THREE.Color(palette.trueWhite), clay.sheenLerp);

  const facadeDetail = uniform(1);

  const fuvN = attribute<'vec2'>('fuv', 'vec2');
  const u = fuvN.x;
  const v = fuvN.y;
  const eaveH = attribute<'float'>('eaveH', 'float');

  // Grid: one cell per (spacing × storeyH). Window occupies windowW×windowH,
  // vertically offset by sillFrac·storeyH from the storey floor.
  const storeyH = float(s.storeyH);
  const spacing = float(s.windowSpacing);
  const winW = float(s.windowW);
  const winH = float(s.windowH);
  const sill = float(s.sillFrac * s.storeyH);
  const frame = float(0.12); // white jamb width (m)

  // storey index & local coords
  const storeyIdx = v.div(storeyH).floor();
  const colIdx = u.div(spacing).floor();
  const localX = u.sub(colIdx.mul(spacing)); // 0..spacing along the wall
  const localY = v.sub(storeyIdx.mul(storeyH)); // 0..storeyH up the storey

  // window rect centred in the cell horizontally, sill-offset vertically
  const winX0 = spacing.sub(winW).mul(0.5);
  const winX1 = winX0.add(winW);
  const winY0 = sill;
  const winY1 = sill.add(winH);

  // signed inset from the window rect edges (>0 inside)
  const insetX = tslMin(localX.sub(winX0), winX1.sub(localX));
  const insetY = tslMin(localY.sub(winY0), winY1.sub(localY));
  const inset = tslMin(insetX, insetY); // >0 inside the window opening

  // eave clamp: the TOP of this storey must sit below eaveH−0.2, else no window
  const storeyTop = storeyIdx.add(1).mul(storeyH);
  const underEave = storeyTop.lessThan(eaveH.sub(float(0.2)));

  // masks (soft edges for a clean AA look, still essentially binary)
  const glassMask = smoothstep(float(0), float(0.03), inset.sub(frame)); // 1 in the pane
  const frameMask = smoothstep(float(0), float(0.03), inset).mul(float(1).sub(glassMask)); // 1 in the jamb ring
  const windowMask = tslMax(glassMask, frameMask);
  // gate by eave clamp and LOD detail (windowMask already zeroes outside the
  // opening, so the gate only needs the clamp + detail multipliers)
  const gate = underEave.select(float(1), float(0)).mul(facadeDetail);

  const clayBase = attribute<'vec3'>('color', 'vec3');
  // Recessed pane reads DARK in daylight (like the original glassMat pane in
  // shadow): take palette.glass down toward a deep glass tone, then darken
  // further toward the pane edges for inset depth. The white frame ring stays
  // bright — the original white-jamb language.
  const edgeDark = mix(float(0.5), float(0.72), smoothstep(float(0), float(0.35), inset));
  const glassTone = vec3(glass.r, glass.g, glass.b).mul(edgeDark);
  const frameTone = vec3(frameCol.r, frameCol.g, frameCol.b);
  const windowTone = mix(glassTone, frameTone, frameMask);

  const facadeColor = mix(clayBase, windowTone, windowMask.mul(gate));

  // Dollhouse cutaway (T18): slice the building at world-y `cutH` (discard
  // everything above), and paint a bright seam band in the `cutSeam` metres
  // just below the cut. At rest (cutH = 1e6) both nodes are no-ops, so the
  // closed building is byte-identical to the plain facade material. The
  // discard MUST run inside a Fn so it appends to the fragment stack — a bare
  // Discard() at construction time attaches to no stack and is silently dropped.
  const cutH = uniform(1e6);
  const upperFade = uniform(1);
  if (opts.cutaway) {
    const seam = new THREE.Color(kswS3.seamColor);
    const seamTone = vec3(seam.r, seam.g, seam.b);
    m.colorNode = Fn(() => {
      const wy = positionWorld.y;
      wy.greaterThan(cutH).discard();
      // seam band: cutH − cutSeam < y ≤ cutH → mix to seamColor
      const inSeam = wy.greaterThan(cutH.sub(float(kswS3.cutSeam))).and(wy.lessThanEqual(cutH));
      return mix(facadeColor, seamTone, inSeam.select(float(1), float(0)));
    })();
  } else {
    m.colorNode = facadeColor;
  }

  if (opts.lampGlow) {
    // deterministic per-cell hash on the facade grid cell (u-cell + storey),
    // mirroring nightWindowHash's sin-fract formula but on cell indices so the
    // choice is stable per window. Only the glass area glows.
    const cellHash = colIdx.mul(float(12.9898)).add(storeyIdx.mul(float(78.233))).sin().mul(float(43758.5453));
    const hash = cellHash.sub(cellHash.floor()); // 0..1 fract via sin, like nightWindowHash
    const lit = hash.lessThan(float(NIGHT_WINDOW_SHARE)).select(float(1), float(0));
    const glow = glassMask.mul(gate).mul(lit);
    m.emissiveNode = vec3(warm.r, warm.g, warm.b).mul(glow.mul(float(0.9)));
  }

  return Object.assign(m, { facadeDetail, cutH, upperFade }) as CutawayFacadeMaterial;
}

export function buildCityMassing(buildings: BakedBuilding[], opts: { lampGlow: boolean } = { lampGlow: false }): THREE.Group {
  const group = new THREE.Group();
  group.name = 'cityMassing';

  const make = (name: string, pick: (b: BakedBuilding) => BakedMesh, base: number): void => {
    const mesh = new THREE.Mesh(mergeTinted(buildings, pick, base), tintedClay(base));
    mesh.name = name;
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    group.add(mesh);
  };

  // Walls: same tinted clay, but a TSL node material that paints the procedural
  // window raster in-shader (Task 13). setFacadeDetail flips the LOD uniform.
  const wallMat = facadeMaterial(palette.creamBase, opts);
  const wallMesh = new THREE.Mesh(mergeWalls(buildings, palette.creamBase), wallMat);
  wallMesh.name = 'cityWalls';
  wallMesh.castShadow = true;
  wallMesh.receiveShadow = true;
  wallMesh.userData.setFacadeDetail = (on: boolean): void => {
    wallMat.facadeDetail.value = on ? 1 : 0;
  };
  group.add(wallMesh);

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
