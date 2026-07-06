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
import { clay, facadeLook, kswCityStyle, kswPalette, kswS3, palette } from '../../designTokens';
import { NIGHT_WINDOW_SHARE } from '../staticBatch';
import { clayMat } from '../props';
import { lampGlowU } from '../glowUniform';
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
  const buildingIdx = new Float32Array(vtx);
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  const baseColor = new THREE.Color(base);
  let vo = 0;
  let io = 0;
  let seed = 0;
  let bi = 0;
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
      buildingIdx[base3 + i / 3] = bi;
    }
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base3 + p.idx[i];
    vo += p.pos.length;
    io += p.idx.length;
    bi++;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.setAttribute('buildingIdx', new THREE.BufferAttribute(buildingIdx, 1));
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
  const buildingIdx = new Float32Array(vtx);
  const indices = vtx > 65535 ? new Uint32Array(tri) : new Uint16Array(tri);
  const baseColor = new THREE.Color(base);
  let vo = 0;
  let uo = 0;
  let io = 0;
  let seed = 0;
  let bi = 0;
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
      buildingIdx[base3 + i / 3] = bi;
    }
    // Window-clamp height: the raw eaveH is right for pitched-roof houses
    // (no windows in the roof), but on flat-roofed/stacked complexes the
    // baked eave sits at the LOWEST roof junction — the KSW tower (eaveH
    // 13.2, ridge 69.9) rendered 57 m of blank clay. When the roof delta is
    // bigger than a real attic, raster windows up to just below the wall top
    // instead.
    const winClampH = b.height - b.eaveH > 4.5 ? b.height - 1.2 : b.eaveH;
    for (let v = 0; v < nVerts; v++) {
      fuv[uo + v * 2] = p.fuv[v * 2] / FUV_PER_M; // 2-dm units → m
      fuv[uo + v * 2 + 1] = p.fuv[v * 2 + 1] / FUV_PER_M;
      eave[uo / 2 + v] = winClampH;
    }
    for (let i = 0; i < p.idx.length; i++) indices[io + i] = base3 + p.idx[i];
    vo += p.pos.length;
    uo += nVerts * 2;
    io += p.idx.length;
    bi++;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.setAttribute('fuv', new THREE.BufferAttribute(fuv, 2));
  geo.setAttribute('eaveH', new THREE.BufferAttribute(eave, 1));
  geo.setAttribute('buildingIdx', new THREE.BufferAttribute(buildingIdx, 1));
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
// Exported for tileContent.ts (Task 4/M3): with out=0 the two side strips
// coincide into a double-sided extruded wall — the cheap massing-prism wall.
// Returns cm-int positions (BakedMesh convention), composable via mergeTinted.
export function ringBand(fp: number[][], y0: number, y1: number, out: number): { pos: number[]; idx: number[] } {
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
// eaveH−0.2, so nothing ever paints above the real eave (root cause B). A
// deterministic per-cell hash < NIGHT_WINDOW_SHARE lights the glass warm via
// emissiveNode, scaled by the shared lampGlowU uniform (0 by day, 1 at night).
// `facadeDetail` (0/1 uniform, driven by the
// LOD ring) fades the whole raster out for the far ring.
type FacadeMaterial = THREE.MeshPhysicalNodeMaterial & {
  facadeDetail: ReturnType<typeof uniform>;
};

// Cutaway-enabled facade material (Phase A): additionally carries
// `discardAbove` + `bandLo` + `bandFade` uniforms so the MAIN
// KSW building can be peeled open storey by storey. At rest (discardAbove =
// bandLo = 1e6, bandFade = 0) every node is a no-op →
// byte-identical to the plain facade material.
export type CutawayFacadeMaterial = FacadeMaterial & {
  discardAbove: ReturnType<typeof uniform>;
  bandLo: ReturnType<typeof uniform>;
  bandFade: ReturnType<typeof uniform>;
};

export function facadeMaterial(base: number, opts: { cutaway?: boolean } = {}): FacadeMaterial {
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

  // Per-building hashes (split-grammar seeds): buildingIdx is constant across
  // a building's vertices, so these gate whole-building rules.
  const bIdxN = attribute<'float'>('buildingIdx', 'float');
  const bSin = bIdxN.mul(float(91.17)).sin().mul(float(43758.5453));
  const bHash = bSin.sub(bSin.floor()); // night lit share (below)
  const bSin2 = bIdxN.mul(float(57.31)).sin().mul(float(43758.5453));
  const bHash2 = bSin2.sub(bSin2.floor()); // shopfront gate
  const bSin3 = bIdxN.mul(float(23.77)).sin().mul(float(43758.5453));
  const bHash3 = bSin3.sub(bSin3.floor()); // balcony gate

  // Ground-floor shopfront rule: storey 0 of `shopShare` of the buildings
  // becomes a near-full-width glazed front (taller, wider pane) — the
  // "ground floor is a different grammar rule" split that makes streets read
  // inhabited.
  const isGF = storeyIdx.lessThan(float(0.5));
  const shopM = isGF.and(bHash2.lessThan(float(facadeLook.shopShare))).select(float(1), float(0));

  // window rect centred in the cell horizontally, sill-offset vertically;
  // shopfronts override toward full-cell glazing.
  const winX0 = mix(spacing.sub(winW).mul(0.5), spacing.mul(float(0.08)), shopM);
  const winX1 = mix(spacing.sub(winW).mul(0.5).add(winW), spacing.mul(float(0.92)), shopM);
  const winY0 = mix(sill, float(0.45), shopM);
  const winY1 = mix(sill.add(winH), float(2.7), shopM);

  // signed inset from the window rect edges (>0 inside)
  const insetX = tslMin(localX.sub(winX0), winX1.sub(localX));
  const insetY = tslMin(localY.sub(winY0), winY1.sub(localY));
  const inset = tslMin(insetX, insetY); // >0 inside the window opening

  // eave clamp: the WINDOW top (not the full storey top) must sit below
  // eaveH−0.15. The old full-storey test (storeyTop < eaveH−0.2) silently
  // dropped the top row on every building — a 2-storey house (eave ≈ 6 m,
  // storeyH 3) rendered exactly one row of windows ("nur im ersten Stock").
  const winTopAbs = storeyIdx.mul(storeyH).add(sill).add(winH);
  const underEave = winTopAbs.lessThan(eaveH.sub(float(0.15)));

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
  // Lintel AO: the top reveal shadows the pane — the classic inset depth cue.
  const topShade = mix(float(0.62), float(1), smoothstep(float(0.02), float(0.4), winY1.sub(localY)));
  // Curtain variety: a share of panes soften toward a warm interior tone so
  // the glass grid doesn't read as one dead material.
  const curtSin = colIdx.mul(float(29.7)).add(storeyIdx.mul(float(13.3))).add(bIdxN.mul(float(7.1))).sin().mul(float(43758.5453));
  const curtHash = curtSin.sub(curtSin.floor());
  const curtM = curtHash.lessThan(float(facadeLook.curtainShare)).select(float(0.55), float(0));
  const curtain = new THREE.Color(facadeLook.curtain);
  const glassTone = mix(
    vec3(glass.r, glass.g, glass.b),
    vec3(curtain.r, curtain.g, curtain.b),
    curtM,
  ).mul(edgeDark).mul(topShade);
  const frameTone = vec3(frameCol.r, frameCol.g, frameCol.b);
  const windowTone = mix(glassTone, frameTone, frameMask);

  // Balcony relief (shader-drawn): on balconyShare of buildings, balconyCol-
  // Share of window columns get a parapet balcony on every upper storey —
  // slab band + solid balustrade under the window, with a contact shadow at
  // the storey floor. Stacked per column, like real balcony risers.
  const isUpper = storeyIdx.greaterThan(float(0.5));
  const colSin = colIdx.mul(float(3.77)).add(bIdxN.mul(float(11.13))).sin().mul(float(43758.5453));
  const colHash = colSin.sub(colSin.floor());
  const balcGate = isUpper
    .and(bHash3.lessThan(float(facadeLook.balconyShare)))
    .and(colHash.lessThan(float(facadeLook.balconyColShare)))
    .select(float(1), float(0))
    .mul(gate)
    .mul(float(1).sub(shopM));
  const bx0 = winX0.sub(float(0.35));
  const bx1 = winX1.add(float(0.35));
  const inBalcX = smoothstep(float(0), float(0.04), localX.sub(bx0)).mul(smoothstep(float(0), float(0.04), bx1.sub(localX)));
  const slabM = inBalcX.mul(smoothstep(float(0.04), float(0.1), localY)).mul(smoothstep(float(0), float(0.06), float(0.34).sub(localY)));
  const parapetM = inBalcX
    .mul(smoothstep(float(0), float(0.05), localY.sub(float(0.34))))
    .mul(smoothstep(float(0), float(0.05), winY0.sub(float(0.1)).sub(localY)));
  const shadowM = inBalcX.mul(smoothstep(float(0.1), float(0.02), localY)); // contact shadow at storey floor
  const slabC = new THREE.Color(facadeLook.slab);
  const parapetC = new THREE.Color(facadeLook.parapet);
  const withBalcony = mix(
    mix(
      mix(clayBase, clayBase.mul(float(0.72)), shadowM.mul(balcGate)),
      vec3(slabC.r, slabC.g, slabC.b),
      slabM.mul(balcGate),
    ),
    vec3(parapetC.r, parapetC.g, parapetC.b),
    parapetM.mul(balcGate),
  );

  const facadeColor = mix(withBalcony, windowTone, windowMask.mul(gate));

  // Storey-peel cutaway (Phase A): three uniforms. Fragments above
  // `discardAbove` are gone (hard cut — everything above the currently
  // dissolving storey). Fragments between `bandLo` and `discardAbove` are the
  // dissolving storey's shell: a deterministic world-position hash against
  // `bandFade` gives a stable screen-door dissolve (no transparency sorting).
  // A warm seam band caps the remaining solid shell just below `bandLo`.
  // At rest (discardAbove = 1e6, bandFade = 0) every node is a no-op — the
  // closed building renders byte-identical to the plain facade material.
  const discardAbove = uniform(1e6);
  const bandLo = uniform(1e6);
  const bandFade = uniform(0);
  if (opts.cutaway) {
    const seam = new THREE.Color(kswS3.seamColor);
    const seamTone = vec3(seam.r, seam.g, seam.b);
    m.colorNode = Fn(() => {
      const wy = positionWorld.y;
      wy.greaterThan(discardAbove).discard();
      // stable per-fragment hash from world position (same trick as the
      // night-window hash below — sin-fract, deterministic, no RNG)
      const n = positionWorld.x.mul(12.9898).add(wy.mul(78.233)).add(positionWorld.z.mul(37.719)).sin().mul(43758.5453);
      const h = n.sub(n.floor());
      wy.greaterThan(bandLo).and(h.lessThan(bandFade)).discard();
      const seamTop = bandLo.add(float(kswS3.cutSeam).mul(bandFade)); // seam grows in as the band dissolves
      const inSeam = wy.greaterThan(bandLo.sub(float(kswS3.cutSeam))).and(wy.lessThanEqual(seamTop)).and(bandFade.greaterThan(float(0.05)));
      return mix(facadeColor, seamTone, inSeam.select(float(1), float(0)));
    })();
  } else {
    m.colorNode = facadeColor;
  }

  // Night glow is ALWAYS built now; its intensity rides the shared lampGlowU
  // uniform (0 = day, no glow; 1 = full night). A deterministic per-cell hash
  // on the facade grid cell (u-cell + storey), mirroring nightWindowHash's
  // sin-fract formula but on cell indices so the choice is stable per window,
  // decides which panes glow. Only the glass area glows.
  //
  // Variance pass (SOTA 2026-07-06): a uniform 55% lit share read as one flat
  // speckle carpet over the whole city. Real night cities live from variance —
  // each BUILDING gets its own lit share (hash(buildingIdx) → ~10%..75%, mean
  // ≈ NIGHT_WINDOW_SHARE) and each lit pane its own brightness (0.55..1.0),
  // so some houses glow, some sleep, and no two windows bloom identically.
  const cellHash = colIdx.mul(float(12.9898)).add(storeyIdx.mul(float(78.233))).sin().mul(float(43758.5453));
  const hash = cellHash.sub(cellHash.floor()); // 0..1 fract via sin, like nightWindowHash
  // per-building share 0.2×..1.8× the base; shopfronts glow more reliably
  const share = float(NIGHT_WINDOW_SHARE)
    .mul(float(0.2).add(bHash.mul(float(1.6))))
    .mul(mix(float(1), float(1.5), shopM));
  const lit = hash.lessThan(share).select(float(1), float(0));
  const paneSin = hash.mul(float(7.13)).add(bHash.mul(float(3.7))).sin().mul(float(43758.5453));
  const paneBright = float(0.55).add(paneSin.sub(paneSin.floor()).mul(float(0.45)));
  // 2.6 peak pushes lit panes past the bloom threshold (kswPost.bloomThreshold
  // 1.05) so night windows read as light sources, not painted-on decals —
  // the old 0.9 peak stayed under the threshold and never bloomed.
  const glow = glassMask.mul(gate).mul(lit).mul(paneBright);
  m.emissiveNode = vec3(warm.r, warm.g, warm.b).mul(glow.mul(float(2.6)).mul(lampGlowU));

  return Object.assign(m, { facadeDetail, discardAbove, bandLo, bandFade }) as CutawayFacadeMaterial;
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

  // Walls: same tinted clay, but a TSL node material that paints the procedural
  // window raster in-shader (Task 13). setFacadeDetail flips the LOD uniform.
  const wallMat = facadeMaterial(palette.creamBase);
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
    // Band top = the baked eave (where walls meet the roof). b.height is the
    // RIDGE — a height-derived guess floats the band mid-air on steep roofs
    // and multi-part swisstopo UUIDs (worst case ~93 m above the real eave).
    const eave = Math.max(b.eaveH, kswCityStyle.plinthH + 0.5);
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
