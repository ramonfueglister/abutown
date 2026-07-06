// src/diorama/traffic/carModels.ts
//
// FIX D2 — Cities-Skylines-school clay car models: pure variant/colour
// selection (unit-tested) + the merged vertex-coloured geometry builders.
//
// Three silhouette VARIANTS — sedan, hatchback, van — each a small set of
// merged boxes in the clay vocabulary, with baked per-vertex colours:
//   * BODY faces are WHITE (1,1,1) so the per-instance body tint (setColorAt)
//     shows through unmodified;
//   * the WINDOW band and the WHEELS are baked DARK, so under the instanced
//     tint they stay dark glass / dark rubber (they pick up only a faint body
//     tint — the MeshPhysicalMaterial multiplies instanceColor × vertexColor).
// No textures — geometry + vertex colours only, matching agentMeshes.ts.
//
// Selection is a stable per-id hash so a vehicle keeps its silhouette AND its
// colour for its whole life on the wire (see carLayer.ts).

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';

/** Metres. Realistic small-car proportions (matches the kernel VEHICLE_LEN
 * 4.5 m within visual tolerance; width 1.9 m sits in a 3.0 m lane). */
const BODY_W = 1.9;

/** CS-like car body palette, tuned against screenshot references: whites and
 * silvers dominate a real car park; then greys/black, a couple of reds, an
 * ochre/brown wagon tone, taxi yellow, blues, green, purple and beige for
 * variety. setColorAt multiplies the white body vertex colour by one of
 * these. */
export const CAR_PALETTE: readonly number[] = [
  0xf2f0ea, // bright white
  0xd9d6cd, // pearl beige-white
  0xb9c0c6, // silver
  0x6f767d, // gunmetal
  0x24272b, // black
  0x8f2432, // maroon (screenshot dark red)
  0xc03a2b, // red
  0xb06a2c, // ochre/brown-orange (screenshot brown wagon)
  0xe0a91f, // taxi yellow (screenshot yellow hatch)
  0x2c4f8a, // dark blue (screenshot sedans)
  0x4a7ec2, // mid blue
  0x3f7d46, // green (screenshot compact)
  0x6b4a86, // purple (screenshot van)
  0xa8a08c, // beige (screenshot SUV)
] as const;

/** Dark glass colour baked into the window band (multiplied by the body tint —
 * stays dark, picks up a hint of the body colour). */
const GLASS = new THREE.Color(0x2a2f36);
/** Near-black rubber baked into the wheels. */
const RUBBER = new THREE.Color(0x181a1d);
/** White = fully tintable body. */
const BODY = new THREE.Color(0xffffff);
/** Slightly warm off-white roof highlight (subtle; still mostly tintable). */
const ROOF = new THREE.Color(0xf4f0ea);

/** Cheap integer hash so a vehicle's variant + colour are stable and well
 * spread across ids. */
export function hashId(id: number): number {
  let h = id >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b);
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b);
  h = h ^ (h >>> 16);
  return h >>> 0;
}

/** Stable body colour for a vehicle id (index into CAR_PALETTE). */
export function carColorForId(id: number): number {
  return CAR_PALETTE[hashId(id) % CAR_PALETTE.length];
}

/** Stable silhouette-variant index for a vehicle id (0..CAR_VARIANTS.length-1).
 * Uses a different hash mix than the colour so variant and colour don't
 * correlate. */
export function carVariantForId(id: number): number {
  // rotate the hash so variant selection is independent of the colour bucket
  const h = hashId(id ^ 0x9e3779b9);
  return h % CAR_VARIANTS.length;
}

type BoxGeo = (w: number, h: number, d: number) => THREE.BoxGeometry;

/** A single coloured box part: dimensions, centre position, and its (uniform)
 * vertex colour. */
interface Part {
  w: number;
  h: number;
  d: number;
  pos: [number, number, number];
  color: THREE.Color;
}

/** Merge coloured box parts into one indexed geometry with a per-vertex colour
 * attribute (working colour space, like agentMeshes.mergeParts). Local origin
 * at the wheel line (y=0), long axis along +z (forward = +z). */
function mergeParts(parts: Part[], boxGeo: BoxGeo, label: string): THREE.BufferGeometry {
  const prepared = parts.map((part) => {
    const g = boxGeo(part.w, part.h, part.d).clone();
    g.translate(part.pos[0], part.pos[1], part.pos[2]);
    const count = g.attributes.position.count;
    const colors = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      colors[i * 3] = part.color.r;
      colors[i * 3 + 1] = part.color.g;
      colors[i * 3 + 2] = part.color.b;
    }
    g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    return g;
  });
  const merged = mergeGeometries(prepared, false);
  if (!merged) throw new Error(`carModels: merge failed for "${label}"`);
  merged.computeVertexNormals();
  merged.computeBoundingSphere();
  return merged;
}

/** Four wheel stubs at the corners of a body of length `len`, inset from the
 * sides. Wheels are dark boxes poking just below the body (their bottom sits at
 * y≈0, the wheel line). */
function wheels(len: number): Part[] {
  const ww = 0.5; // wheel width (across car)
  const wr = 0.62; // wheel diameter (box height)
  const wd = 0.9; // wheel footprint along the car
  const xo = BODY_W / 2 - ww / 2 + 0.02; // just outside the flush body sides
  const zo = len / 2 - wd / 2 - 0.35; // inset from the bumpers
  const yo = wr / 2 - 0.02; // bottom at the wheel line
  const mk = (x: number, z: number): Part => ({ w: ww, h: wr, d: wd, pos: [x, yo, z], color: RUBBER });
  return [mk(xo, zo), mk(-xo, zo), mk(xo, -zo), mk(-xo, -zo)];
}

// ── the three silhouette builders ──────────────────────────────────────────
// Each returns the merged geometry. Proportions differ so the silhouettes read
// distinct from a typical city-camera distance: sedan has a hood+trunk step and
// a short greenhouse; hatchback is shorter with a sloped rear cabin; van is
// taller with a long boxy cabin.

const WHEEL_LINE = 0.60; // body underside sits here, wheels straddle it

function buildSedan(boxGeo: BoxGeo): THREE.BufferGeometry {
  const len = 4.3;
  const bodyH = 0.55;
  const bodyY = WHEEL_LINE + bodyH / 2;
  const cabinH = 0.5;
  const cabinY = WHEEL_LINE + bodyH + cabinH / 2;
  const parts: Part[] = [
    // lower body (hood → trunk), full length
    { w: BODY_W, h: bodyH, d: len, pos: [0, bodyY, 0], color: BODY },
    // cabin, centred, shorter than the body (leaves a hood in front, trunk behind)
    { w: BODY_W - 0.14, h: cabinH, d: 2.0, pos: [0, cabinY, -0.1], color: BODY },
    // window band: a slightly smaller, slightly lower dark box inside the cabin
    { w: BODY_W - 0.02, h: cabinH - 0.16, d: 1.7, pos: [0, cabinY - 0.02, -0.1], color: GLASS },
    // roof highlight cap
    { w: BODY_W - 0.2, h: 0.06, d: 1.7, pos: [0, cabinY + cabinH / 2 - 0.02, -0.1], color: ROOF },
    ...wheels(len),
  ];
  return mergeParts(parts, boxGeo, 'sedan');
}

function buildHatchback(boxGeo: BoxGeo): THREE.BufferGeometry {
  const len = 3.8;
  const bodyH = 0.55;
  const bodyY = WHEEL_LINE + bodyH / 2;
  const cabinH = 0.52;
  const cabinY = WHEEL_LINE + bodyH + cabinH / 2;
  const parts: Part[] = [
    { w: BODY_W, h: bodyH, d: len, pos: [0, bodyY, 0], color: BODY },
    // cabin pushed rearward (short hood, long glasshouse to the tail — hatch)
    { w: BODY_W - 0.14, h: cabinH, d: 2.2, pos: [0, cabinY, -0.4], color: BODY },
    { w: BODY_W - 0.02, h: cabinH - 0.16, d: 1.95, pos: [0, cabinY - 0.02, -0.4], color: GLASS },
    { w: BODY_W - 0.2, h: 0.06, d: 1.9, pos: [0, cabinY + cabinH / 2 - 0.02, -0.4], color: ROOF },
    ...wheels(len),
  ];
  return mergeParts(parts, boxGeo, 'hatchback');
}

function buildVan(boxGeo: BoxGeo): THREE.BufferGeometry {
  const len = 4.7;
  const bodyH = 0.6;
  const bodyY = WHEEL_LINE + bodyH / 2;
  const cabinH = 0.82; // taller box cabin
  const cabinY = WHEEL_LINE + bodyH + cabinH / 2;
  const parts: Part[] = [
    { w: BODY_W, h: bodyH, d: len, pos: [0, bodyY, 0], color: BODY },
    // tall long cabin (short bonnet, boxy body)
    { w: BODY_W - 0.08, h: cabinH, d: 3.3, pos: [0, cabinY, -0.55], color: BODY },
    // window band only over the front portion (a cargo van has a cab up front)
    { w: BODY_W - 0.0, h: cabinH - 0.4, d: 1.3, pos: [0, cabinY + 0.1, 0.9], color: GLASS },
    { w: BODY_W - 0.16, h: 0.06, d: 3.1, pos: [0, cabinY + cabinH / 2 - 0.02, -0.55], color: ROOF },
    ...wheels(len),
  ];
  return mergeParts(parts, boxGeo, 'van');
}

/** Per-axle geometry for a variant, in the same local space as the body
 * (origin at the wheel line, y=0, forward = +z). */
export interface WheelLayout {
  wheelbase: number; // m, distance between axle centres
  track: number;     // m, distance between left/right wheel centres
  radius: number;    // m, wheel radius
  width: number;     // m, wheel width across the car
}

/** Four wheel local offsets `[x, y, z]` for a wheel layout, y = radius.
 * FRONT pair (positive z) is listed first — carLayer applies steer to
 * indices 0 and 1. */
export function wheelOffsets(l: WheelLayout): [number, number, number][] {
  const x = l.track / 2;
  const z = l.wheelbase / 2;
  return [
    [x, l.radius, z], [-x, l.radius, z],
    [x, l.radius, -z], [-x, l.radius, -z],
  ];
}

/** A CS-style car variant: silhouette name, overall length, wheel layout, and
 * body/glass geometry builders. Order in CAR_VARIANTS is stable —
 * carVariantForId indexes into it. */
export interface CarVariant {
  name: string;
  length: number; // m, overall
  wheels: WheelLayout;
  buildBody: (boxGeo: BoxGeo) => THREE.BufferGeometry;
  buildGlass: () => THREE.BufferGeometry;
}

/** Placeholder glass geometry — replaced by the real cabin glass in Task 2. */
function stubGlass(): THREE.BufferGeometry {
  return new THREE.BoxGeometry(1, 0.3, 1.5);
}

// NOTE: wagon/suv/pickup temporarily alias the sedan/van box builders as
// stand-ins for this task; Task 2 replaces buildBody with real per-variant
// geometry (this table's shape — name/length/wheels — is the stable part).
export const CAR_VARIANTS: readonly CarVariant[] = [
  { name: 'sedan',     length: 4.5, wheels: { wheelbase: 2.7, track: 1.56, radius: 0.31, width: 0.24 }, buildBody: buildSedan,     buildGlass: stubGlass },
  { name: 'hatchback', length: 3.9, wheels: { wheelbase: 2.5, track: 1.52, radius: 0.30, width: 0.22 }, buildBody: buildHatchback, buildGlass: stubGlass },
  { name: 'wagon',     length: 4.6, wheels: { wheelbase: 2.8, track: 1.56, radius: 0.31, width: 0.24 }, buildBody: buildSedan,     buildGlass: stubGlass },
  { name: 'suv',       length: 4.6, wheels: { wheelbase: 2.8, track: 1.62, radius: 0.38, width: 0.28 }, buildBody: buildVan,       buildGlass: stubGlass },
  { name: 'van',       length: 5.2, wheels: { wheelbase: 3.3, track: 1.66, radius: 0.36, width: 0.26 }, buildBody: buildVan,       buildGlass: stubGlass },
  { name: 'pickup',    length: 5.0, wheels: { wheelbase: 3.1, track: 1.62, radius: 0.38, width: 0.28 }, buildBody: buildVan,       buildGlass: stubGlass },
] as const;
