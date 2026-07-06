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

/** CS-like car body palette: saturated-but-clay distinct colours. Whites and
 * silvers dominate a real car park; then reds/blues/greens/a yellow/black for
 * variety. Tuned to pop against the tarmac under soft GI without going neon
 * (mid-value, not fully-saturated primaries). setColorAt multiplies the white
 * body vertex colour by one of these. */
export const CAR_PALETTE: readonly number[] = [
  0xe8e6e0, // pearl white
  0xf2f0ea, // bright white
  0xb9c0c6, // silver
  0x8b939b, // gunmetal
  0x2f3338, // near-black
  0xc0402f, // clay red
  0xd97b34, // amber/orange
  0xe4be3f, // muted yellow
  0x3f6db0, // royal blue
  0x5f97c4, // sky blue
  0x2f7d5b, // racing green
  0x7ba05a, // olive/lime
  0x8a5a9c, // muted purple
  0xc25f86, // rose
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

/** The variant table: name (for mesh naming) + geometry builder. Order is
 * stable — carVariantForId indexes into it. */
export const CAR_VARIANTS: readonly { name: string; build: (boxGeo: BoxGeo) => THREE.BufferGeometry }[] = [
  { name: 'sedan', build: buildSedan },
  { name: 'hatchback', build: buildHatchback },
  { name: 'van', build: buildVan },
] as const;
