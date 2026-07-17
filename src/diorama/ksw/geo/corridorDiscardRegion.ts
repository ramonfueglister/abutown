// src/diorama/ksw/geo/corridorDiscardRegion.ts
//
// WHERE the corridor terrain-discard is active (#144) — one region, read by
// the two fragment shaders that must agree on it:
//
//   - terrain.ts (terrainDiscardMat) discards the ground inside a road/rail
//     corridor, opening the corridor slot;
//   - roads.ts (skirtMat) draws the vertical walls that close that slot.
//
// The skirt has no purpose of its own: it exists ONLY to close what the
// discard opened. Drawing it where nothing is discarded leaves a bare wall
// standing on ground the platform was never draped onto — beyond the fine
// ring the visible surface is the coarse L1/L0 backdrop, which deviates up to
// ~20 m from the fine heights the platform follows, so the wall stands clear
// of the terrain and reads as the black rectangle/line chains along the far
// ring. Keying both shaders to this single region makes "a skirt exists iff
// its hole exists" structural rather than a coincidence of two constants.
//
// The region is a disc around the camera because the discard itself is
// distance-limited: it must stay comfortably inside the streamer's guaranteed
// fine (L2) ring, where the corridor-snapped surface matches the platform
// within centimetres (see terrainDiscardMat for why the rim matters).
import * as THREE from 'three/webgpu';
import { positionWorld, uniform } from 'three/tsl';

const anchorU = uniform(new THREE.Vector2(0, 0));
// Until the render loop sets an anchor, the region is the whole world: that is
// the pre-#144 behaviour (discard everywhere), never a silent no-op.
const radiusU = uniform(1e9);

/**
 * Per-frame update of the discard region — call from the render loop with the
 * SAME position the tile streamer uses and a radius safely inside its fine
 * (L2) ring.
 */
export function updateCorridorDiscardAnchor(x: number, z: number, radiusM: number): void {
  (anchorU.value as THREE.Vector2).set(x, z);
  radiusU.value = radiusM;
}

/** TSL: true where the corridor discard is active (terrain.ts discards here). */
export function insideDiscardRegion() {
  return positionWorld.xz.sub(anchorU).length().lessThan(radiusU);
}

/**
 * TSL: true where the corridor discard is NOT active (skirtMat discards here).
 * Spelled as its own comparison rather than `.not()` of the above so the
 * boundary is explicit: `< radius` and `>= radius` partition every point, so
 * the terrain and the skirt can never both claim it, nor both disown it.
 */
export function outsideDiscardRegion() {
  return positionWorld.xz.sub(anchorU).length().greaterThanEqual(radiusU);
}

/**
 * Pure JS twin of the two nodes above. MIRROR (load-bearing): the shaders are
 * untestable in vitest, so this carries the region's contract under test —
 * change it and the nodes together.
 */
export function isInsideDiscardRegion(x: number, z: number): boolean {
  const a = anchorU.value as THREE.Vector2;
  return Math.hypot(x - a.x, z - a.y) < (radiusU.value as number);
}
