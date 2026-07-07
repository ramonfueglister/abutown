// src/diorama/ksw/geo/lod.ts
// Semantic 3-ring LOD for the city (spec §2c): visibility + shadow policy by
// camera radius, with hysteresis so orbiting at a boundary never flickers.
import type * as THREE from 'three/webgpu';
import { kswCityStyle } from '../../designTokens';

export type CityLodRing = 'near' | 'mid' | 'far';

export function cityLodState(radius: number, prev: CityLodRing): CityLodRing {
  const { nearR, midR, hysteresis } = kswCityStyle.lod;
  const up = 1 + hysteresis;
  const dn = 1 - hysteresis;
  if (prev === 'near') return radius > nearR * up ? cityLodState(radius, 'mid') : 'near';
  if (prev === 'mid') {
    if (radius < nearR * dn) return 'near';
    return radius > midR * up ? 'far' : 'mid';
  }
  return radius < midR * dn ? cityLodState(radius, 'mid') : 'far';
}

// Refs are collected via getObjectByName, which can legitimately return
// undefined (a design-legal missing group/mesh) — applyCityLod must not
// assume any of these exist and must never throw on a partially-null refs
// object (e.g. in tests, or a bake that skips a mesh).
export type CityLodRefs = {
  // Windows are now a wall-shader raster (Task 13), not a separate object — the
  // far ring flips a uniform via setFacadeDetail instead of hiding a group.
  setFacadeDetail: (on: boolean) => void;
  // Lamps are NOT here — they have their own distance LOD (lampLodVisibility),
  // decoupled from the facade ring so the far-visible facade raster can't drag
  // 17.9k opaque lamp posts/bulbs into the establishing view (2026-07-07 fix).
  footways: THREE.Object3D | null;
  // Trees handle their own distance LOD (near-set compaction + the impostor's
  // vertex-stage near-collapse), so the ring never toggles tree visibility —
  // it only drives whether the near-set casts shadows.
  setTreeShadows: (on: boolean) => void;
};

export function applyCityLod(ring: CityLodRing, r: CityLodRefs): void {
  const far = ring === 'far';
  r.setFacadeDetail(!far);
  if (r.footways) r.footways.visible = !far;
  r.setTreeShadows(ring === 'near');
}

// ── Lamp LOD (2026-07-07 flicker/clutter fix) ───────────────────────────────
// Street lamps split into two roles with independent cull distances (tokens
// kswCityStyle.lampLod). The camera radius drives a hysteresis threshold per
// role so orbiting a boundary never flickers the toggle itself.
//  - hardware (posts/heads/bulbs): opaque + sub-pixel far away. These are the
//    day clutter and the scintillation; cull them past hardwareR (~300 m).
//  - glow (pools/halos): additive, invisible by day; the night atmosphere at
//    the establishing framing. Keep them out to glowR (~1500 m).
export type LampVis = { hardware: boolean; glow: boolean };

function bandVisible(radius: number, R: number, h: number, prev: boolean): boolean {
  if (radius < R * (1 - h)) return true;
  if (radius > R * (1 + h)) return false;
  return prev; // inside the hysteresis band → hold the current state
}

export function lampLodVisibility(radius: number, prev: LampVis): LampVis {
  const { hardwareR, glowR, hysteresis } = kswCityStyle.lampLod;
  return {
    hardware: bandVisible(radius, hardwareR, hysteresis, prev.hardware),
    glow: bandVisible(radius, glowR, hysteresis, prev.glow),
  };
}
