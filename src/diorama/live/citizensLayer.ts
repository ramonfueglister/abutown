// src/diorama/live/citizensLayer.ts
//
// Instanced citizen capsules for the live channel (MMORPG M1 Task 15).
// Pattern follows src/diorama/traffic/carLayer.ts (ONE InstancedMesh, clay
// material from designTokens via props.clayMat, per-instance color, capacity
// 4096, frustumCulled=false because instances roam far from the base
// geometry's bounding sphere — Task 10 finding) with the LOD1 capsule person
// silhouette from agentMeshes.ts (capsule body + head sphere, same overall
// ~1.2 m height so the crowd reads like the hero agents at city zoom).
//
// Dead reckoning between the 1 Hz CitizenCellFrames: a citizen with a moving
// activity (3=walking, 4=driving) advances linearly at WALK_SPEED_MPS along
// its last observed movement direction; a new frame SNAPS it to the wire
// position (and refreshes the direction from the observed displacement).
// Stationary citizens (0=home, 1=work, 2=market) render in place — standing
// people at buildings make the town read alive — but are capped per cell
// (STANDING_CAP_PER_CELL) so one dense block cannot exhaust the instance
// budget that moving citizens need.

import * as THREE from 'three/webgpu';
import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
import { palette } from '../designTokens';
import { clayMat } from '../ksw/props';
import type { GroundYAt } from '../traffic/carLayer';
import type { LiveCitizen } from './liveClient';

/** Instance capacity for VISIBLE citizens (the AOI-subscribed cells only). */
export const CITIZEN_CAPACITY = 4096;

/** Dead-reckoning walk speed between frames (m/s). */
export const WALK_SPEED_MPS = 1.4;

/** Max stationary citizens (activity 0/1/2) rendered per cell — movers are
 * never capped (they carry the "town is alive" signal). */
export const STANDING_CAP_PER_CELL = 64;

/** Tiny lift so the capsule feet never z-fight the draped ground. */
const FOOT_LIFT = 0.02;

/** Per-activity body colors (live.proto: 0=home 1=work 2=market 3=walking
 * 4=driving) — muted palette entries, one accent for market-goers. */
const ACTIVITY_COLORS: readonly number[] = [
  palette.honey, // home
  palette.sage, // work
  palette.coralSoft, // market
  palette.mint, // walking
  palette.skin, // driving (visible while the car channel is traffic's)
];

/** LOD1-person silhouette (agentMeshes.lodPersonGeometry proportions): a
 * low-poly capsule body + head sphere, feet at y=0, ~1.2 m tall. */
function buildCitizenGeometry(): THREE.BufferGeometry {
  const body = new THREE.CapsuleGeometry(0.3, 0.42, 1, 6);
  body.translate(0, 0.5, 0);
  const head = new THREE.SphereGeometry(0.235, 6, 4);
  head.translate(0, 1.0, 0);
  const merged = mergeGeometries([body, head], false);
  if (!merged) throw new Error('citizensLayer: person geometry merge failed');
  merged.computeBoundingSphere();
  return merged;
}

/** Per-citizen dead-reckoning state (positions in metres, world frame). */
interface CitState {
  /** Wire position at the last frame (snap anchor). */
  frameX: number;
  frameZ: number;
  /** Unit movement direction observed across the last two frames (0,0 until
   * a displacement was seen). */
  dirX: number;
  dirZ: number;
  activity: number;
  /** Layer-clock seconds when the last frame for this citizen arrived. */
  frameAt: number;
}

export interface CitizensLayer {
  /** Add this to the scene (cityRoot). */
  object3d: THREE.Object3D;
  /** Mirror one decoded CitizenCellFrame (wired to liveClient's onCitizens). */
  applyFrame(cell: number, citizens: LiveCitizen[], departed: number[], keyframe: boolean): void;
  /** Drop cells that left the AOI subscription (their departed frames stop). */
  dropCells(cells: number[]): void;
  /** Advance dead reckoning + write instance matrices. `nowSec` is the render
   * clock in seconds (same monotonic t the animate loop uses). */
  update(nowSec: number): void;
  /** Instances drawn after the last update() (smoke/debug surface). */
  count(): number;
  /** Total tracked citizens across all cells (may exceed what is drawn). */
  trackedCount(): number;
}

export function createCitizensLayer(groundYAt?: GroundYAt): CitizensLayer {
  const geometry = buildCitizenGeometry();
  const material = clayMat(palette.trueWhite);
  const mesh = new THREE.InstancedMesh(geometry, material, CITIZEN_CAPACITY);
  mesh.name = 'liveCitizens';
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  mesh.castShadow = false; // a street crowd is too many casters for the cascade
  mesh.receiveShadow = true;
  mesh.frustumCulled = false;
  mesh.count = 0;
  mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(CITIZEN_CAPACITY * 3), 3);

  /** cell id -> (citizen id -> dead-reckoning state). */
  const cells = new Map<number, Map<number, CitState>>();
  let lastNow = 0;

  // Reused scratch — no per-frame allocation on the hot path.
  const mat = new THREE.Matrix4();
  const pos = new THREE.Vector3();
  const quat = new THREE.Quaternion();
  const scl = new THREE.Vector3(1, 1, 1);
  const up = new THREE.Vector3(0, 1, 0);
  const col = new THREE.Color();

  function applyFrame(cell: number, citizens: LiveCitizen[], departed: number[], keyframe: boolean): void {
    const prev = cells.get(cell);
    let table = prev;
    if (keyframe || !table) {
      table = new Map<number, CitState>();
      cells.set(cell, table);
    }
    for (const c of citizens) {
      // Direction from the observed displacement since the previous frame —
      // look the citizen up in the pre-keyframe table too, so a keyframe
      // doesn't erase its heading.
      const old = table.get(c.id) ?? prev?.get(c.id);
      let dirX = old?.dirX ?? 0;
      let dirZ = old?.dirZ ?? 0;
      if (old) {
        const dx = c.x - old.frameX;
        const dz = c.z - old.frameZ;
        const d = Math.hypot(dx, dz);
        if (d > 1e-3) {
          dirX = dx / d;
          dirZ = dz / d;
        }
      }
      table.set(c.id, { frameX: c.x, frameZ: c.z, dirX, dirZ, activity: c.activity, frameAt: lastNow });
    }
    if (!keyframe) {
      for (const id of departed) table.delete(id);
    }
  }

  function update(nowSec: number): void {
    lastNow = nowSec;
    let i = 0;
    outer: for (const table of cells.values()) {
      let standing = 0;
      for (const [, c] of table) {
        if (i >= CITIZEN_CAPACITY) break outer;
        const moving = c.activity === 3 || c.activity === 4;
        if (!moving) {
          if (standing >= STANDING_CAP_PER_CELL) continue;
          standing++;
        }
        let x = c.frameX;
        let z = c.frameZ;
        let yaw = 0;
        if (moving) {
          // Linear dead reckoning at walk speed along the last heading; the
          // next frame snaps (frameX/frameZ are wire-authoritative).
          const dt = Math.max(0, nowSec - c.frameAt);
          x += c.dirX * WALK_SPEED_MPS * dt;
          z += c.dirZ * WALK_SPEED_MPS * dt;
          if (c.dirX !== 0 || c.dirZ !== 0) yaw = Math.atan2(c.dirX, c.dirZ);
        }
        const groundY = groundYAt ? groundYAt(x, z) : 0;
        pos.set(x, groundY + FOOT_LIFT, z);
        quat.setFromAxisAngle(up, yaw);
        mat.compose(pos, quat, scl);
        mesh.setMatrixAt(i, mat);
        col.set(ACTIVITY_COLORS[c.activity] ?? palette.skin);
        mesh.setColorAt(i, col);
        i++;
      }
    }
    mesh.count = i;
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
  }

  return {
    object3d: mesh,
    applyFrame,
    dropCells(cellIds: number[]): void {
      for (const c of cellIds) cells.delete(c);
    },
    update,
    count: () => mesh.count,
    trackedCount: () => {
      let n = 0;
      for (const t of cells.values()) n += t.size;
      return n;
    },
  };
}
