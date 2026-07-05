// src/diorama/traffic/flowLayer.ts
//
// Task 12: the far-LOD "impostor" flow layer. Where the browser is NOT
// subscribed to per-vehicle CellFrames (i.e. outside the 3×3 AOI the camera
// is looking at, minus a one-CELL_SIZE_M fade ring), this layer renders a
// deterministic, aggregate stand-in for traffic density from the FlowFrame
// channel (Task 11): per-edge vehicle count + mean speed, no per-vehicle
// identity. The result is meant to read as "there is traffic on this street"
// from a distance, not to depict real cars — impostor placement is a
// deterministic hash of (edgeId, slot), advected along the edge at the
// FlowState's mean speed, NOT dead-reckoned per-vehicle (there is no
// per-vehicle state to reckon).
//
// Design invariant (binding, see the Task 12 brief + deviation log): an edge
// must never show BOTH real cars (via carLayer, inside the subscribed AOI)
// AND impostors in the same cell. `fadeFor` computes 0 opacity for any point
// inside a subscribed cell, ramping to full opacity (1) over one
// CELL_SIZE_M-wide ring outside the subscribed set, and `placeImpostors`'s
// callers (createFlowLayer.update) additionally test each impostor's actual
// world position (not just its edge) so a long edge that clips a subscribed
// cell only suppresses the impostors that fall inside it.
//
// The pure math (placement + fade distance) is factored out so it can be
// tested WITHOUT three.js/WebGL, following the same testable-core split as
// deadReckon.ts / trafficClient.ts (TrafficClientCore).

import * as THREE from 'three/webgpu';
import { palette } from '../designTokens';
import { buildCarGeometry } from './carLayer';
import type { GroundYAt } from './carLayer';
import { CELL_SIZE_M, type CellGrid } from './trafficClient';
import type { TrafficNetGeom } from './deadReckon';

/** Instance capacity for the impostor layer (per the Task 12 brief). Far
 * larger than CAR_CAPACITY (4096) since impostors summarise the WHOLE net
 * outside the AOI, not just a subscribed neighbourhood. */
export const FLOW_CAPACITY = 8192;

/** One decoded FlowFrame edge entry (already unit-decoded: v in m/s). Mirrors
 * `FlowState` (traffic.proto) after `vQ / 4` decode — see trafficClient.ts's
 * WireVehicle.vQ decode (`v_q / 4 -> m/s`) for the same quantisation. */
export interface FlowEdge {
  count: number;
  v: number;
}

/** Minimal edge geometry `placeImpostors` needs: the polyline + declared
 * length. A narrowed view of `TrafficNetGeom` for one edge id (edge id ==
 * lane id — `net.edges` in the Rust `TrafficNet` is per-lane, see
 * backend/crates/winterthur-traffic/src/flow.rs's `view.edge`). */
export interface EdgeGeom {
  pts: number[][];
  lengthM: number;
}

/** One placed impostor: world (x, z), yaw about +y (same atan2(tx,tz)
 * convention as deadReckon.poseAt), and `fade` in [0, 1] — 0 fully
 * transparent (suppressed inside the subscribed AOI), 1 fully opaque. Fade is
 * computed by the caller (createFlowLayer.update) via `fadeFor`, not by
 * `placeImpostors` itself, which only knows the edge — not the subscription
 * set. */
export interface ImpostorPlacement {
  x: number;
  z: number;
  yaw: number;
  fade: number;
}

/** Deterministic per-(edgeId, slot) hash in [0, 1). splitmix64-style
 * finalizer (project lesson: FNV-1a HIGH bits cluster, see the
 * stationary-age-seed memory — use a proper finalizer, not raw XOR-shift
 * high bits). Pure integer math, so the SAME (edgeId, slot) always yields the
 * SAME offset across calls/sessions — the placement is a pure function of
 * its inputs, never Math.random(). */
function hash01(edgeId: number, slot: number): number {
  let h = (edgeId >>> 0) * 0x9e3779b1 + (slot >>> 0);
  h = h >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x21f0aaad);
  h = Math.imul(h ^ (h >>> 15), 0x735a2d97);
  h = (h ^ (h >>> 15)) >>> 0;
  return h / 0x100000000;
}

/** Position + unit tangent at arc length `s` along `edge.pts` (0..lengthM
 * clamped, wrapping is the CALLER's job via mod). A minimal local port of
 * deadReckon.posAt's segment walk (linear scan — impostor edges are short
 * lists, no LUT/binary-search needed at this call volume). */
function posOnEdge(edge: EdgeGeom, s: number): { x: number; z: number; yaw: number } {
  const pts = edge.pts;
  if (pts.length < 2) return { x: pts[0]?.[0] ?? 0, z: pts[0]?.[1] ?? 0, yaw: 0 };
  const sc = Math.min(Math.max(s, 0), edge.lengthM);
  let acc = 0;
  for (let i = 1; i < pts.length; i++) {
    const a = pts[i - 1];
    const b = pts[i];
    const dx = b[0] - a[0];
    const dz = b[1] - a[1];
    const segLen = Math.sqrt(dx * dx + dz * dz);
    const segEnd = acc + segLen;
    if (sc <= segEnd || i === pts.length - 1) {
      const tanLen = segLen > 1e-9 ? segLen : 1;
      const tx = dx / tanLen;
      const tz = dz / tanLen;
      const t = segLen > 1e-9 ? Math.min(Math.max((sc - acc) / segLen, 0), 1) : 0;
      return { x: a[0] + dx * t, z: a[1] + dz * t, yaw: Math.atan2(tx, tz) };
    }
    acc = segEnd;
  }
  const last = pts[pts.length - 1];
  return { x: last[0], z: last[1], yaw: 0 };
}

/** Place `count` impostors along `edge` at wall-clock time `nowS`, advected
 * at `ADVECT_SPEED_MPS` from a deterministic per-slot base offset. Pure — no
 * three.js, no mutable module state. `edgeId` seeds the hash so different
 * edges with identical geometry (impossible in practice, but kept honest)
 * still place differently, and so the SAME edge's impostors are stable
 * across frames (no per-frame re-randomisation, which would read as
 * flickering/teleporting from a distance). */
const ADVECT_SPEED_MPS = 8; // a plausible "typical" flow speed for advection when v isn't supplied

export function placeImpostors(
  edge: EdgeGeom,
  count: number,
  nowS: number,
  edgeId: number,
  speedMps: number = ADVECT_SPEED_MPS,
): ImpostorPlacement[] {
  const out: ImpostorPlacement[] = [];
  const len = edge.lengthM > 1e-6 ? edge.lengthM : 1;
  for (let slot = 0; slot < count; slot++) {
    const baseOffset = hash01(edgeId, slot) * len;
    const advected = baseOffset + speedMps * nowS;
    const s = ((advected % len) + len) % len; // mod into [0, len)
    const { x, z, yaw } = posOnEdge(edge, s);
    out.push({ x, z, yaw, fade: 0 });
  }
  return out;
}

/** Distance-based fade in [0, 1] for a world point: 0 if the point's cell is
 * in `subscribedCells`, ramping linearly to 1 over one CELL_SIZE_M ring
 * beyond the subscribed set's nearest edge, 1 beyond that ring. Distance is
 * measured to the NEAREST cell boundary of the subscribed set — approximated
 * here via a simple grid search up to the ring radius (the subscribed set is
 * always a small 3×3-ish block, so this is O(1) in practice: a handful of
 * `cellOf` calls at most). */
export function fadeFor(
  grid: CellGrid,
  subscribedCells: ReadonlySet<number>,
  x: number,
  z: number,
): number {
  const cell = grid.cellOf(x, z);
  if (subscribedCells.has(cell)) return 0;
  const { col, row } = grid.colRowOf(x, z);
  // Find the minimum Chebyshev ring distance from (col,row) to ANY
  // subscribed cell's (col,row) — 1 ring == fully faded in (fade=1), 0 rings
  // is unreachable here (already returned 0 above via direct membership).
  let minRing = Infinity;
  for (const c of subscribedCells) {
    const srow = Math.floor(c / grid.cols);
    const scol = c - srow * grid.cols;
    const ring = Math.max(Math.abs(col - scol), Math.abs(row - srow));
    if (ring < minRing) minRing = ring;
  }
  if (!Number.isFinite(minRing)) return 1; // no subscription at all -> fully opaque
  // ring 1 => the immediately adjacent cell ring: partially faded in. ring 2
  // and beyond => fully opaque (one full CELL_SIZE_M ring of crossfade).
  const t = minRing / 2;
  return Math.min(Math.max(t, 0), 1);
}

/** The impostor flow layer object + its per-frame update entry point. */
export interface FlowLayer {
  /** Add this to the scene. */
  object3d: THREE.Object3D;
  /** Recompute impostor placement + fade for every edge with `count >= 1` in
   * `flow`, skipping impostors whose world position falls inside a
   * subscribed cell (so a subscribed edge never double-renders real cars +
   * impostors in the same cell — see the module banner). */
  update(flow: Map<number, FlowEdge>, subscribedCells: ReadonlySet<number>, nowS: number): void;
  /** Number of impostor instances actually drawn as of the last `update()`
   * call (i.e. `mesh.count`) — exposed for the `?traffic` debug hook
   * (`window.__traffic.flowCount()`, Task 13 smoke assertion (g)). */
  count(): number;
}

/** Per-edge impostor count, scaled down from the raw (saturating-255) count
 * so a busy arterial doesn't blow the 8192 cap on its own — this is a
 * DENSITY IMPRESSION, not a literal per-vehicle render. Clamped to at least 1
 * for any edge with count >= 1. */
function impostorCountFor(count: number): number {
  return Math.max(1, Math.min(6, Math.round(count / 4)));
}

export function createFlowLayer(net: TrafficNetGeom, grid: CellGrid, groundYAt?: GroundYAt): FlowLayer {
  const geometry = buildCarGeometry();
  const material = new THREE.MeshStandardMaterial({
    color: palette.metalDark,
    transparent: true,
  });
  const mesh = new THREE.InstancedMesh(geometry, material, FLOW_CAPACITY);
  mesh.name = 'trafficFlowImpostors';
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  mesh.castShadow = false;
  mesh.receiveShadow = false;
  // Instances are scattered across the ENTIRE net (far from the base
  // geometry's origin-centred bounding sphere), same reasoning as carLayer:
  // disable per-object frustum culling or the whole mesh vanishes as soon as
  // the camera looks away from the world origin.
  mesh.frustumCulled = false;
  mesh.count = 0;
  mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(FLOW_CAPACITY * 3), 3);

  const surfaceOffset = 0.06; // small lift above the draped ground, matching carLayer's CAR_LIFT

  const mat = new THREE.Matrix4();
  const pos = new THREE.Vector3();
  const quat = new THREE.Quaternion();
  const scl = new THREE.Vector3(1, 1, 1);
  const up = new THREE.Vector3(0, 1, 0);
  const col = new THREE.Color();

  function update(flow: Map<number, FlowEdge>, subscribedCells: ReadonlySet<number>, nowS: number): void {
    let i = 0;
    for (const [edgeId, edge] of flow) {
      if (i >= FLOW_CAPACITY) break;
      if (edge.count < 1) continue;
      const pts = net.pts.get(edgeId);
      const arcLut = net.arcLut.get(edgeId);
      if (!pts || !arcLut || pts.length < 2) continue;
      const lengthM = arcLut[arcLut.length - 1];
      const edgeGeom: EdgeGeom = { pts, lengthM };
      const n = impostorCountFor(edge.count);
      const placements = placeImpostors(edgeGeom, n, nowS, edgeId, edge.v > 0 ? edge.v : ADVECT_SPEED_MPS);
      for (const p of placements) {
        if (i >= FLOW_CAPACITY) break;
        const fade = fadeFor(grid, subscribedCells, p.x, p.z);
        if (fade <= 0) continue; // fully suppressed inside the subscribed AOI
        const groundY = groundYAt ? groundYAt(p.x, p.z) : 0;
        pos.set(p.x, groundY + surfaceOffset, p.z);
        quat.setFromAxisAngle(up, p.yaw);
        mat.compose(pos, quat, scl);
        mesh.setMatrixAt(i, mat);
        // Per-instance opacity via instanceColor: three.js's instancing path
        // multiplies vertex color into the material's base color/alpha when
        // `material.transparent` is set and the geometry carries an
        // instanceColor attribute — a dim, fixed hue scaled by `fade` reads
        // as a faint stand-in near the fade ring and a fully dim car farther
        // out. NO custom shader (per the brief).
        col.setRGB(fade, fade, fade);
        mesh.setColorAt(i, col);
        i++;
      }
    }
    mesh.count = i;
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
  }

  return { object3d: mesh, update, count: () => mesh.count };
}
