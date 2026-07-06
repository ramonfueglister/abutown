// src/diorama/traffic/carModels.ts
//
// Cities-Skylines-style car models: pure variant/colour selection
// (unit-tested) + the loft-based vertex-coloured geometry builders.
//
// Six silhouette VARIANTS — sedan, hatchback, wagon, suv, van, pickup — each
// a loft of trapezoid cross-sections swept along z (flat-shaded low-poly
// panels, the CS look) plus merged detail boxes, with baked per-vertex colours:
//   * BODY faces are WHITE (1,1,1) so the per-instance body tint (setColorAt)
//     shows through unmodified;
//   * grille/lights/plates are baked in their own colours so they read at
//     distance under any tint;
//   * GLASS is a separate bright loft shell (own material, see carLayer);
//   * WHEELS are a separate instanced cylinder (buildWheelGeometry).
// No textures — geometry + vertex colours only, matching agentMeshes.ts.
//
// Selection is a stable per-id hash so a vehicle keeps its silhouette AND its
// colour for its whole life on the wire (see carLayer.ts).

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';

/** Body half-width baseline (m). SUV widens by +0.06, van by +0.10. */
const W = 1.82;

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

/** CS glass: bright reflective sky-blue (baked into the glass geometry AND
 * multiplied by the glass material colour — see carLayer). */
const GLASS = new THREE.Color(0xbfe0f2);
const RUBBER = new THREE.Color(0x17191c);
const RIM = new THREE.Color(0xb7bcc2);
const GRILLE = new THREE.Color(0x1d2126);
const HEADLIGHT = new THREE.Color(0xfff4d0);
const TAILLIGHT = new THREE.Color(0xb01a1a);
const PLATE = new THREE.Color(0xf5f5f0);
/** White = fully tintable body. */
const BODY = new THREE.Color(0xffffff);

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

// ── loft geometry kernel ────────────────────────────────────────────────────

interface Section { z: number; yBot: number; yTop: number; wBot: number; wTop: number }

/** Sweep trapezoid cross-sections along z into a flat-shaded, non-indexed,
 * uniformly vertex-coloured hull (left/right/top/bottom strips + end caps). */
function loft(sections: Section[], color: THREE.Color): THREE.BufferGeometry {
  const pos: number[] = [];
  const quad = (a: number[], b: number[], c: number[], d: number[]) =>
    pos.push(...a, ...b, ...c, ...a, ...c, ...d);
  const corners = (s: Section) => ({
    bl: [-s.wBot / 2, s.yBot, s.z], br: [s.wBot / 2, s.yBot, s.z],
    tl: [-s.wTop / 2, s.yTop, s.z], tr: [s.wTop / 2, s.yTop, s.z],
  });
  for (let i = 0; i < sections.length - 1; i++) {
    const a = corners(sections[i]);
    const b = corners(sections[i + 1]);
    quad(a.br, b.br, b.tr, a.tr); // right (+x)
    quad(b.bl, a.bl, a.tl, b.tl); // left (−x)
    quad(a.tr, b.tr, b.tl, a.tl); // top
    quad(a.bl, b.bl, b.br, a.br); // bottom
  }
  const first = corners(sections[0]);
  const last = corners(sections[sections.length - 1]);
  quad(first.bl, first.br, first.tr, first.tl); // front cap (−z end listed first)
  quad(last.br, last.bl, last.tl, last.tr);     // rear cap
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pos), 3));
  const n = g.attributes.position.count;
  const colors = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) colors.set([color.r, color.g, color.b], i * 3);
  g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  g.computeVertexNormals();
  g.computeBoundingSphere();
  return g;
}

/** Merge loft hulls and `mergeParts` outputs into one non-indexed geometry.
 * (`mergeGeometries` requires all-indexed or all-non-indexed inputs; the loft
 * is non-indexed, `mergeParts` output is indexed — normalise here.) */
function finishMerged(...parts: THREE.BufferGeometry[]): THREE.BufferGeometry {
  const prepared = parts.map((p) => {
    const g = p.index ? p.toNonIndexed() : p;
    // the loft has no uv channel (untextured); drop uv from box/cylinder parts
    // so all inputs carry the same attribute set — mergeGeometries requires it
    g.deleteAttribute('uv');
    return g;
  });
  const merged = mergeGeometries(prepared, false);
  if (!merged) throw new Error('carModels: hull merge failed');
  merged.computeVertexNormals();
  merged.computeBoundingSphere();
  return merged;
}

// ── the six silhouette builders ─────────────────────────────────────────────
// Each body is a loft of trapezoid sections (+z = FORWARD) plus merged detail
// boxes (grille, lights, plates). The body underside floats at UNDERBODY —
// instanced wheels (carLayer) fill the gap below.

/** Body underside height (m). SUV/pickup ride higher, van slightly higher —
 * those builders use a local override. */
const UNDERBODY = 0.30;

/** Grille + head/taillights + number plates as merged boxes at the ±length/2
 * cap faces, protruding 0.03 so they read at distance. */
function detailBoxes(
  boxGeo: BoxGeo,
  length: number,
  width = W,
  underbody = UNDERBODY,
  belt = 0.95,
): THREE.BufferGeometry {
  const half = length / 2;
  const lightX = width / 2 - 0.34; // lights near the body corners
  const lightY = belt - 0.18;
  const lowY = underbody + 0.28; // plates + grille height
  const parts: Part[] = [
    // grille across the nose (box depth 0.06 centred ON the cap → 0.03 proud)
    { w: width * 0.55, h: 0.16, d: 0.06, pos: [0, lowY, half], color: GRILLE },
    // headlights / taillights at the four corners
    { w: 0.28, h: 0.12, d: 0.06, pos: [lightX, lightY, half], color: HEADLIGHT },
    { w: 0.28, h: 0.12, d: 0.06, pos: [-lightX, lightY, half], color: HEADLIGHT },
    { w: 0.28, h: 0.12, d: 0.06, pos: [lightX, lightY, -half], color: TAILLIGHT },
    { w: 0.28, h: 0.12, d: 0.06, pos: [-lightX, lightY, -half], color: TAILLIGHT },
    // number plates, 0.005 in front of the grille face so they never z-fight
    { w: 0.5, h: 0.13, d: 0.04, pos: [0, lowY, half + 0.015], color: PLATE },
    { w: 0.5, h: 0.13, d: 0.04, pos: [0, lowY, -half - 0.015], color: PLATE },
  ];
  return mergeParts(parts, boxGeo, 'details');
}

function buildSedan(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 4.5, half = L / 2, belt = 0.95, roof = 1.38;
  const hull = loft([
    { z:  half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.42, wBot: W - 0.30, wTop: W - 0.34 }, // bumper lip
    { z:  half - 0.28, yBot: UNDERBODY,        yTop: belt - 0.12,      wBot: W - 0.06, wTop: W - 0.10 }, // nose
    { z:  half - 0.80, yBot: UNDERBODY,        yTop: belt - 0.06,      wBot: W - 0.03, wTop: W - 0.08 }, // hood mid
    { z:  half - 1.30, yBot: UNDERBODY,        yTop: belt,             wBot: W,        wTop: W - 0.06 }, // hood end / windshield base
    { z:  half - 2.05, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // windshield top / roof front
    { z:  0.0,         yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // roof mid (front doors)
    { z: -0.35,        yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // roof mid (rear doors)
    { z: -half + 1.55, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // roof rear
    { z: -half + 1.25, yBot: UNDERBODY,        yTop: (roof + belt) / 2 + 0.05, wBot: W, wTop: W - 0.22 }, // rear glass mid
    { z: -half + 0.95, yBot: UNDERBODY,        yTop: belt + 0.06,      wBot: W,        wTop: W - 0.10 }, // trunk lid
    { z: -half + 0.60, yBot: UNDERBODY,        yTop: belt - 0.02,      wBot: W - 0.03, wTop: W - 0.11 }, // trunk mid
    { z: -half + 0.26, yBot: UNDERBODY,        yTop: belt - 0.10,      wBot: W - 0.06, wTop: W - 0.12 }, // tail
    { z: -half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.40, wBot: W - 0.30, wTop: W - 0.36 }, // rear bumper lip
  ], BODY);
  return finishMerged(hull, detailBoxes(boxGeo, L));
}

function sedanGlass(): THREE.BufferGeometry {
  const half = 4.5 / 2, belt = 0.95, roof = 1.38;
  return loft([
    { z:  half - 1.32, yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.36 }, // windshield base
    { z:  half - 2.03, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // windshield top
    { z: -half + 1.57, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // rear roof
    { z: -half + 0.97, yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.38 }, // rear glass base
  ], GLASS);
}

function buildHatchback(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 3.9, half = L / 2, belt = 0.95, roof = 1.42;
  const hull = loft([
    { z:  half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.42, wBot: W - 0.30, wTop: W - 0.34 }, // bumper lip
    { z:  half - 0.28, yBot: UNDERBODY,        yTop: belt - 0.12,      wBot: W - 0.06, wTop: W - 0.10 }, // nose
    { z:  half - 1.10, yBot: UNDERBODY,        yTop: belt,             wBot: W,        wTop: W - 0.06 }, // hood end / windshield base
    { z:  half - 1.75, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // windshield top / roof front
    { z: -half + 1.30, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // roof rear
    { z: -half + 0.26, yBot: UNDERBODY,        yTop: 1.05,             wBot: W - 0.06, wTop: W - 0.12 }, // tail (rear glass runs to it)
    { z: -half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.40, wBot: W - 0.30, wTop: W - 0.36 }, // rear bumper lip
  ], BODY);
  return finishMerged(hull, detailBoxes(boxGeo, L));
}

function hatchbackGlass(): THREE.BufferGeometry {
  const half = 3.9 / 2, belt = 0.95, roof = 1.42;
  return loft([
    { z:  half - 1.12, yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.36 }, // windshield base
    { z:  half - 1.77, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // windshield top
    { z: -half + 1.32, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // roof rear
    { z: -half + 0.40, yBot: belt, yTop: belt + 0.03,  wBot: W - 0.10, wTop: W - 0.38 }, // hatch glass base
  ], GLASS);
}

function buildWagon(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 4.6, half = L / 2, belt = 0.95, roof = 1.42;
  const hull = loft([
    { z:  half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.42, wBot: W - 0.30, wTop: W - 0.34 }, // bumper lip
    { z:  half - 0.28, yBot: UNDERBODY,        yTop: belt - 0.12,      wBot: W - 0.06, wTop: W - 0.10 }, // nose
    { z:  half - 1.30, yBot: UNDERBODY,        yTop: belt,             wBot: W,        wTop: W - 0.06 }, // hood end / windshield base
    { z:  half - 1.95, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // windshield top / roof front
    { z: -half + 0.55, yBot: UNDERBODY,        yTop: roof,             wBot: W,        wTop: W - 0.34 }, // roof rear (long roof)
    { z: -half + 0.26, yBot: UNDERBODY,        yTop: 1.00,             wBot: W - 0.06, wTop: W - 0.12 }, // steep tailgate
    { z: -half,        yBot: UNDERBODY + 0.10, yTop: UNDERBODY + 0.40, wBot: W - 0.30, wTop: W - 0.36 }, // rear bumper lip
  ], BODY);
  return finishMerged(hull, detailBoxes(boxGeo, L));
}

function wagonGlass(): THREE.BufferGeometry {
  const half = 4.6 / 2, belt = 0.95, roof = 1.42;
  return loft([
    { z:  half - 1.32, yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.36 }, // windshield base
    { z:  half - 1.97, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // windshield top
    { z: -half + 0.57, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // roof rear
    { z: -half + 0.33, yBot: belt, yTop: belt + 0.03,  wBot: W - 0.10, wTop: W - 0.38 }, // tailgate glass base
  ], GLASS);
}

function buildSuv(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 4.6, half = L / 2, belt = 1.10, roof = 1.68;
  const Wv = W + 0.06, u = 0.42;
  const hull = loft([
    { z:  half,        yBot: u + 0.10, yTop: u + 0.42,    wBot: Wv - 0.30, wTop: Wv - 0.34 }, // bumper lip
    { z:  half - 0.28, yBot: u,        yTop: belt - 0.12, wBot: Wv - 0.06, wTop: Wv - 0.10 }, // upright nose
    { z:  half - 1.05, yBot: u,        yTop: belt,        wBot: Wv,        wTop: Wv - 0.06 }, // hood end / windshield base
    { z:  half - 1.55, yBot: u,        yTop: roof,        wBot: Wv,        wTop: Wv - 0.34 }, // windshield top / roof front
    { z: -half + 0.45, yBot: u,        yTop: roof,        wBot: Wv,        wTop: Wv - 0.34 }, // roof rear
    { z: -half + 0.26, yBot: u,        yTop: belt + 0.10, wBot: Wv - 0.04, wTop: Wv - 0.12 }, // near-vertical tailgate
    { z: -half,        yBot: u + 0.10, yTop: u + 0.42,    wBot: Wv - 0.30, wTop: Wv - 0.36 }, // rear bumper lip
  ], BODY);
  return finishMerged(hull, detailBoxes(boxGeo, L, Wv, u, belt));
}

function suvGlass(): THREE.BufferGeometry {
  const half = 4.6 / 2, belt = 1.10, roof = 1.68;
  const Wv = W + 0.06;
  return loft([
    { z:  half - 1.07, yBot: belt, yTop: belt + 0.02,  wBot: Wv - 0.10, wTop: Wv - 0.36 }, // windshield base
    { z:  half - 1.57, yBot: belt, yTop: roof + 0.015, wBot: Wv - 0.08, wTop: Wv - 0.36 }, // windshield top
    { z: -half + 0.47, yBot: belt, yTop: roof + 0.015, wBot: Wv - 0.08, wTop: Wv - 0.36 }, // roof rear
    { z: -half + 0.33, yBot: belt, yTop: belt + 0.03,  wBot: Wv - 0.10, wTop: Wv - 0.38 }, // tailgate glass base
  ], GLASS);
}

function buildVan(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 5.2, half = L / 2, belt = 1.15, roof = 2.05;
  const Wv = W + 0.10, u = 0.34;
  const hull = loft([
    { z:  half,        yBot: u + 0.10, yTop: u + 0.42,    wBot: Wv - 0.30, wTop: Wv - 0.34 }, // bumper lip
    { z:  half - 0.25, yBot: u,        yTop: belt - 0.12, wBot: Wv - 0.06, wTop: Wv - 0.10 }, // stubby nose
    { z:  half - 0.85, yBot: u,        yTop: belt,        wBot: Wv,        wTop: Wv - 0.06 }, // short hood / windshield base
    { z:  half - 1.45, yBot: u,        yTop: roof,        wBot: Wv,        wTop: Wv - 0.30 }, // steep windshield top
    { z: -half + 0.30, yBot: u,        yTop: roof,        wBot: Wv,        wTop: Wv - 0.30 }, // long box roof
    { z: -half + 0.15, yBot: u,        yTop: roof - 0.10, wBot: Wv - 0.04, wTop: Wv - 0.30 }, // near-vertical tail
    { z: -half,        yBot: u + 0.10, yTop: u + 0.45,    wBot: Wv - 0.30, wTop: Wv - 0.36 }, // rear bumper lip
  ], BODY);
  return finishMerged(hull, detailBoxes(boxGeo, L, Wv, u, belt));
}

function vanGlass(): THREE.BufferGeometry {
  // Windshield + front-door band only — a cargo van has a cab up front.
  const half = 5.2 / 2, belt = 1.15;
  const Wv = W + 0.10;
  return loft([
    { z:  half - 0.87, yBot: belt, yTop: belt + 0.02, wBot: Wv - 0.10, wTop: Wv - 0.32 }, // windshield base
    { z:  half - 1.40, yBot: belt, yTop: 1.92,        wBot: Wv - 0.08, wTop: Wv - 0.32 }, // windshield top (below roofline)
    { z:  half - 2.60, yBot: belt, yTop: 1.92,        wBot: Wv - 0.08, wTop: Wv - 0.32 }, // front-door band end
  ], GLASS);
}

function buildPickup(boxGeo: BoxGeo): THREE.BufferGeometry {
  const L = 5.0, half = L / 2, belt = 1.05, roof = 1.62;
  const u = 0.42;
  const hull = loft([
    { z:  half,        yBot: u + 0.10, yTop: u + 0.42,    wBot: W - 0.30, wTop: W - 0.34 }, // bumper lip
    { z:  half - 0.28, yBot: u,        yTop: belt - 0.12, wBot: W - 0.06, wTop: W - 0.10 }, // nose
    { z:  half - 1.05, yBot: u,        yTop: belt,        wBot: W,        wTop: W - 0.06 }, // hood end / windshield base
    { z:  half - 1.55, yBot: u,        yTop: roof,        wBot: W,        wTop: W - 0.34 }, // windshield top / cab roof
    { z:  0.20,        yBot: u,        yTop: roof,        wBot: W,        wTop: W - 0.34 }, // cab roof rear
    { z:  0.05,        yBot: u,        yTop: belt + 0.02, wBot: W,        wTop: W - 0.06 }, // cab back → bed rail
    { z: -half + 0.20, yBot: u,        yTop: belt + 0.02, wBot: W,        wTop: W - 0.06 }, // bed side end
    { z: -half,        yBot: u + 0.10, yTop: belt - 0.02, wBot: W - 0.20, wTop: W - 0.24 }, // tailgate
  ], BODY);
  // Open-bed cavity look: a dark inner loft recessed below the bed rails.
  const bed = loft([
    { z:  0.0,         yBot: u + 0.10, yTop: belt - 0.06, wBot: W - 0.30, wTop: W - 0.30 },
    { z: -half + 0.18, yBot: u + 0.10, yTop: belt - 0.06, wBot: W - 0.30, wTop: W - 0.30 },
  ], RUBBER);
  return finishMerged(hull, bed, detailBoxes(boxGeo, L, W, u, belt));
}

function pickupGlass(): THREE.BufferGeometry {
  const half = 5.0 / 2, belt = 1.05, roof = 1.62;
  return loft([
    { z:  half - 1.07, yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.36 }, // windshield base
    { z:  half - 1.57, yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // windshield top
    { z:  0.24,        yBot: belt, yTop: roof + 0.015, wBot: W - 0.08, wTop: W - 0.36 }, // cab roof rear
    { z:  0.10,        yBot: belt, yTop: belt + 0.02,  wBot: W - 0.10, wTop: W - 0.38 }, // rear cab glass base
  ], GLASS);
}

function buildTruck(boxGeo: BoxGeo): THREE.BufferGeometry {
  // Rigid HGV (Motorwagen): stubby high cab + separate tall cargo box. The sim
  // drives a 12 m kinematic length (idm::class_len_m); the visual body stays a
  // touch shorter so the box never pokes into the follower's bumper gap.
  const L = 10.4, half = L / 2, cabBelt = 1.45, cabRoof = 2.55;
  const Wv = W + 0.22, u = 0.55;
  const cab = loft([
    { z:  half,        yBot: u + 0.10, yTop: u + 0.55,       wBot: Wv - 0.30, wTop: Wv - 0.34 }, // bumper
    { z:  half - 0.20, yBot: u,        yTop: cabBelt - 0.15, wBot: Wv - 0.06, wTop: Wv - 0.10 }, // flat nose
    { z:  half - 0.55, yBot: u,        yTop: cabBelt,        wBot: Wv,        wTop: Wv - 0.06 }, // windshield base
    { z:  half - 1.10, yBot: u,        yTop: cabRoof,        wBot: Wv,        wTop: Wv - 0.26 }, // steep windshield top
    { z:  half - 2.30, yBot: u,        yTop: cabRoof,        wBot: Wv,        wTop: Wv - 0.26 }, // cab rear
  ], BODY);
  // Cargo box: slightly wider/taller than the cab, vertical walls, in the
  // neutral box-body white (NOT the per-instance tint — set via its own colour
  // so fleet tints only touch the cab).
  const box = loft([
    { z:  half - 2.45, yBot: u + 0.05, yTop: 3.15, wBot: Wv + 0.06, wTop: Wv + 0.06 }, // box front
    { z: -half + 0.30, yBot: u + 0.05, yTop: 3.15, wBot: Wv + 0.06, wTop: Wv + 0.06 }, // box rear
    { z: -half + 0.15, yBot: u + 0.10, yTop: 3.05, wBot: Wv - 0.10, wTop: Wv - 0.14 }, // rear doors inset
  ], BODY);
  return finishMerged(cab, box, detailBoxes(boxGeo, L, Wv, u, cabBelt));
}

function truckGlass(): THREE.BufferGeometry {
  // Cab-only glazing: windshield + door band, nothing on the box.
  const half = 10.4 / 2, belt = 1.45;
  const Wv = W + 0.22;
  return loft([
    { z:  half - 0.57, yBot: belt, yTop: belt + 0.02, wBot: Wv - 0.10, wTop: Wv - 0.28 }, // windshield base
    { z:  half - 1.08, yBot: belt, yTop: 2.42,        wBot: Wv - 0.08, wTop: Wv - 0.28 }, // windshield top
    { z:  half - 2.10, yBot: belt, yTop: 2.42,        wBot: Wv - 0.08, wTop: Wv - 0.28 }, // door band end
  ], GLASS);
}

// ── wheel geometry (shared by all variants; carLayer scales instances) ──────

export const WHEEL_GEO_RADIUS = 0.3;

/** Unit-ish wheel: dark tire + lighter rim, cylinder axis along x (the axle).
 * Instances are scaled by `layout.radius / WHEEL_GEO_RADIUS`. */
export function buildWheelGeometry(): THREE.BufferGeometry {
  const r = WHEEL_GEO_RADIUS;
  const tire = new THREE.CylinderGeometry(r, r, 0.22, 12);
  tire.rotateZ(Math.PI / 2); // cylinder axis y → x (axle across the car)
  const rim = new THREE.CylinderGeometry(r * 0.55, r * 0.55, 0.235, 8);
  rim.rotateZ(Math.PI / 2);
  const paint = (g: THREE.BufferGeometry, c: THREE.Color) => {
    const n = g.attributes.position.count;
    const colors = new Float32Array(n * 3);
    for (let i = 0; i < n; i++) colors.set([c.r, c.g, c.b], i * 3);
    g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    return g;
  };
  return finishMerged(paint(tire, RUBBER), paint(rim, RIM));
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

export const CAR_VARIANTS: readonly CarVariant[] = [
  { name: 'sedan',     length: 4.5,  wheels: { wheelbase: 2.7, track: 1.56, radius: 0.31, width: 0.24 }, buildBody: buildSedan,     buildGlass: sedanGlass },
  { name: 'hatchback', length: 3.9,  wheels: { wheelbase: 2.5, track: 1.52, radius: 0.30, width: 0.22 }, buildBody: buildHatchback, buildGlass: hatchbackGlass },
  { name: 'wagon',     length: 4.6,  wheels: { wheelbase: 2.8, track: 1.56, radius: 0.31, width: 0.24 }, buildBody: buildWagon,     buildGlass: wagonGlass },
  { name: 'suv',       length: 4.6,  wheels: { wheelbase: 2.8, track: 1.62, radius: 0.38, width: 0.28 }, buildBody: buildSuv,       buildGlass: suvGlass },
  { name: 'van',       length: 5.2,  wheels: { wheelbase: 3.3, track: 1.66, radius: 0.36, width: 0.26 }, buildBody: buildVan,       buildGlass: vanGlass },
  { name: 'pickup',    length: 5.0,  wheels: { wheelbase: 3.1, track: 1.62, radius: 0.38, width: 0.28 }, buildBody: buildPickup,    buildGlass: pickupGlass },
  { name: 'truck',     length: 10.4, wheels: { wheelbase: 5.6, track: 1.90, radius: 0.50, width: 0.34 }, buildBody: buildTruck,     buildGlass: truckGlass },
] as const;

/** Index of the HGV silhouette in [`CAR_VARIANTS`]. */
export const TRUCK_VARIANT = CAR_VARIANTS.length - 1;

/** Number of PRIVATE-car silhouettes (the leading entries private traffic may
 * draw from — everything before van; vans/pickups/trucks are reserved for the
 * commercial classes so a family sedan never renders as a box van). */
const PRIVATE_VARIANTS = 4;

/** Silhouette for a (wire vehicle class, id) pair — the class decides the
 * family, the id hash the member:
 *  * class 0 (car):       sedan / hatchback / wagon / suv
 *  * class 1 (delivery):  van (¾) or pickup (¼)
 *  * class 2 (HGV):       truck
 * Unknown classes fall back to the car family (wire is additive-only). */
export function carVariantForClass(cls: number, id: number): number {
  const h = hashId(id ^ 0x9e3779b9);
  if (cls === 2) return TRUCK_VARIANT;
  if (cls === 1) return h % 4 === 0 ? 5 : 4; // pickup : van
  return h % PRIVATE_VARIANTS;
}

/** Commercial body colours: white-dominated with silver/grey and a few company
 * accents — delivery fleets are overwhelmingly white. */
export const COMMERCIAL_PALETTE: readonly number[] = [
  0xf2f2ee, 0xf2f2ee, 0xf2f2ee, 0xe8e8e4, // white ×4
  0xc9ccd1, 0x9aa0a6, // silver / grey
  0xc9452c, 0x2c5fa8, 0xdba63a, // company accents: red / blue / amber
];

/** Stable body colour for a (class, id) pair: private cars keep the full
 * palette, commercial classes draw from the white-heavy commercial palette. */
export function carColorForClass(cls: number, id: number): number {
  if (cls === 1 || cls === 2) {
    return COMMERCIAL_PALETTE[hashId(id) % COMMERCIAL_PALETTE.length];
  }
  return carColorForId(id);
}
