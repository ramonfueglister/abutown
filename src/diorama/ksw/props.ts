// Procedural equipment library for the KSW diorama. Chunky clay vocabulary
// (RoundedBox / capsule / cylinder / torus), every color a design token.
// Builders return ground-anchored groups (y = 0 at the floor).

import * as THREE from 'three/webgpu';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import { clay, kswPalette, palette, radii } from '../designTokens';
import type { PersonPlacement, PersonRole, PropPlacement } from './floorPlan';

const materialCache = new Map<number, THREE.MeshPhysicalMaterial>();
export function clayMat(color: number): THREE.MeshPhysicalMaterial {
  let m = materialCache.get(color);
  if (!m) {
    m = new THREE.MeshPhysicalMaterial({ color, roughness: clay.roughness, metalness: clay.metalness });
    m.sheen = clay.sheen;
    m.sheenRoughness = clay.sheenRoughness;
    m.sheenColor = new THREE.Color(color).lerp(new THREE.Color(0xffffff), 0.5);
    materialCache.set(color, m);
  }
  return m;
}

let glassMaterial: THREE.MeshStandardMaterial | null = null;
export function glassMat(): THREE.MeshStandardMaterial {
  if (!glassMaterial) {
    glassMaterial = new THREE.MeshStandardMaterial({
      color: palette.glass,
      roughness: 0.4,
      metalness: 0,
      transparent: true,
      opacity: 0.16,
    });
  }
  return glassMaterial;
}

const glowCache = new Map<number, THREE.MeshBasicMaterial>();
function glowMat(color: number): THREE.MeshBasicMaterial {
  let m = glowCache.get(color);
  if (!m) {
    m = new THREE.MeshBasicMaterial({ color });
    glowCache.set(color, m);
  }
  return m;
}

export function box(w: number, h: number, d: number, color: number, r: number = radii.s): THREE.Mesh {
  const radius = Math.max(0.01, Math.min(r, w / 2 - 1e-3, h / 2 - 1e-3, d / 2 - 1e-3));
  const mesh = new THREE.Mesh(new RoundedBoxGeometry(w, h, d, 4, radius), clayMat(color));
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  return mesh;
}

export function cylinder(rTop: number, rBot: number, h: number, color: number, seg = 20): THREE.Mesh {
  const mesh = new THREE.Mesh(new THREE.CylinderGeometry(rTop, rBot, h, seg), clayMat(color));
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  return mesh;
}

function sphere(r: number, color: number, seg = 14): THREE.Mesh {
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(r, seg, seg), clayMat(color));
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  return mesh;
}

function torus(r: number, tube: number, color: number, arc = Math.PI * 2): THREE.Mesh {
  const mesh = new THREE.Mesh(new THREE.TorusGeometry(r, tube, 12, 32, arc), clayMat(color));
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  return mesh;
}

function at<T extends THREE.Object3D>(obj: T, x: number, y: number, z: number, rotY = 0): T {
  obj.position.set(x, y, z);
  if (rotY) obj.rotation.y = rotY;
  return obj;
}

function screen(w: number, h: number): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(w, h, 0.07, palette.eye, radii.xs), 0, 0, 0));
  const face = new THREE.Mesh(new THREE.BoxGeometry(w - 0.06, h - 0.06, 0.02), glowMat(kswPalette.screenGlow));
  face.position.z = 0.04;
  g.add(face);
  return g;
}

function castering(g: THREE.Group, w: number, d: number, y = 0.055): void {
  for (const [sx, sz] of [
    [-1, 1],
    [1, 1],
    [-1, -1],
    [1, -1],
  ] as const) {
    g.add(at(sphere(0.055, palette.metalDark, 10), (sx * w) / 2, y, (sz * d) / 2));
  }
}

// ── ported prototype props ──────────────────────────────────────────────

function hospitalBed(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(2.0, 0.32, 0.95, palette.woodSoft, radii.m), 0, 0.3, 0));
  g.add(at(box(1.9, 0.2, 0.85, palette.white, radii.m), 0, 0.56, 0));
  g.add(at(box(1.0, 0.13, 0.87, palette.coralSoft, radii.m), -0.5, 0.67, 0));
  g.add(at(box(0.42, 0.15, 0.55, palette.creamLight, radii.m), 0.68, 0.68, 0));
  g.add(at(box(0.1, 0.75, 0.95, palette.woodSoft, radii.m), 0.99, 0.46, 0));
  return g;
}

function careCart(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.66, 0.74, 0.46, palette.white, radii.m), 0, 0.5, 0));
  for (const dy of [-0.2, 0.02, 0.24]) {
    g.add(at(box(0.56, 0.015, 0.02, palette.metalMatt, radii.xs), 0, 0.5 + dy, 0.235));
  }
  g.add(at(box(0.68, 0.05, 0.48, palette.mint, radii.xs), 0, 0.9, 0));
  g.add(at(cylinder(0.045, 0.045, 0.15, palette.mint, 12), -0.16, 1.0, 0.02));
  g.add(at(cylinder(0.04, 0.04, 0.12, palette.white, 12), 0.0, 0.99, -0.08));
  castering(g, 0.48, 0.3);
  return g;
}

function ivStand(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.05, 0.05, 1.65, palette.white, 14), 0, 0.85, 0));
  g.add(at(cylinder(0.2, 0.26, 0.09, palette.white), 0, 0.045, 0));
  g.add(at(box(0.2, 0.32, 0.09, palette.mint, radii.s), 0.14, 1.5, 0));
  const arm = cylinder(0.028, 0.028, 0.3, palette.white, 10);
  arm.rotation.z = Math.PI / 2;
  g.add(at(arm, 0.08, 1.68, 0));
  return g;
}

function vitalsMonitor(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.045, 0.045, 1.15, palette.white, 12), 0, 0.6, 0));
  g.add(at(cylinder(0.18, 0.23, 0.08, palette.white), 0, 0.04, 0));
  g.add(at(screen(0.46, 0.34), 0, 1.32, 0));
  g.add(at(box(0.3, 0.03, 0.02, palette.mint, radii.xs), 0, 1.32, 0.07));
  return g;
}

function plant(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.19, 0.24, 0.3, palette.plantPot), 0, 0.15, 0));
  const puffs: Array<[number, number, number, number]> = [
    [0, 0.56, 0, 0.25],
    [0.15, 0.44, 0.07, 0.16],
    [-0.13, 0.47, -0.06, 0.17],
  ];
  for (const [x, y, z, r] of puffs) g.add(at(sphere(r, palette.plantGreen, 18), x, y, z));
  return g;
}

function sideTable(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.52, 0.52, 0.48, palette.woodSoft, radii.m), 0, 0.26, 0));
  g.add(at(cylinder(0.055, 0.05, 0.11, palette.white, 14), 0.08, 0.58, 0.05));
  return g;
}

// ── reception / waiting / signage ───────────────────────────────────────

function receptionDesk(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(2.6, 1.0, 0.7, palette.white, radii.l), 0, 0.5, 0));
  g.add(at(box(2.8, 0.12, 0.85, palette.mint, radii.m), 0, 1.06, 0));
  g.add(at(screen(0.4, 0.28), -0.7, 1.3, -0.1, Math.PI));
  g.add(at(cylinder(0.05, 0.05, 0.22, palette.white, 10), -0.7, 1.14, -0.1));
  g.add(at(plantSmall(0.55), 1.1, 1.12, -0.12));
  return g;
}

function triageDesk(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.8, 1.0, 0.65, palette.white, radii.l), 0, 0.5, 0));
  g.add(at(box(1.95, 0.12, 0.78, palette.coral, radii.m), 0, 1.06, 0));
  g.add(at(screen(0.36, 0.26), 0.45, 1.28, -0.08, Math.PI));
  g.add(at(cylinder(0.045, 0.045, 0.2, palette.white, 10), 0.45, 1.13, -0.08));
  return g;
}

function counterDesk(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(2.2, 1.0, 0.65, palette.white, radii.l), 0, 0.5, 0));
  g.add(at(box(2.35, 0.12, 0.78, palette.plantGreen, radii.m), 0, 1.06, 0));
  g.add(at(box(0.3, 0.22, 0.2, palette.mint, radii.s), -0.6, 1.23, 0));
  return g;
}

function waitingBench(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.52, 0.4, 1.5, palette.woodSoft, radii.m), 0, 0.2, 0));
  g.add(at(box(0.12, 0.5, 1.5, palette.woodSoft, radii.s), 0.22, 0.62, 0));
  return g;
}

function infoBoard(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.05, 0.06, 1.1, palette.metalMatt, 12), 0, 0.55, 0));
  g.add(at(box(0.95, 0.75, 0.08, palette.white, radii.s), 0, 1.5, 0));
  g.add(at(box(0.7, 0.09, 0.03, palette.mint, radii.xs), 0, 1.68, 0.05));
  g.add(at(box(0.7, 0.09, 0.03, palette.sage, radii.xs), 0, 1.5, 0.05));
  g.add(at(box(0.7, 0.09, 0.03, palette.coralSoft, radii.xs), 0, 1.32, 0.05));
  return g;
}

function handSanitizer(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.045, 0.055, 1.0, palette.white, 12), 0, 0.5, 0));
  g.add(at(cylinder(0.16, 0.2, 0.06, palette.white), 0, 0.03, 0));
  g.add(at(box(0.16, 0.26, 0.14, palette.mint, radii.s), 0, 1.1, 0.02));
  return g;
}

function wheelchair(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.5, 0.1, 0.48, palette.mint, radii.m), 0, 0.52, 0));
  g.add(at(box(0.5, 0.5, 0.1, palette.mint, radii.m), 0, 0.85, -0.26));
  for (const side of [-1, 1]) {
    const wheel = torus(0.3, 0.06, palette.metalDark);
    wheel.rotation.y = Math.PI / 2;
    g.add(at(wheel, side * 0.31, 0.31, -0.05));
    g.add(at(sphere(0.07, palette.metalDark, 10), side * 0.26, 0.075, 0.3));
    g.add(at(box(0.06, 0.06, 0.4, palette.metalMatt, radii.xs), side * 0.24, 0.6, 0.05));
  }
  return g;
}

function linenCart(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.9, 0.85, 0.55, palette.sage, radii.m), 0, 0.55, 0));
  g.add(at(box(0.7, 0.12, 0.42, palette.white, radii.s), 0, 1.05, 0));
  g.add(at(box(0.66, 0.12, 0.38, palette.mint, radii.s), 0.02, 1.17, 0));
  g.add(at(box(0.62, 0.12, 0.36, palette.white, radii.s), -0.02, 1.29, 0));
  castering(g, 0.72, 0.4);
  return g;
}

// ── emergency ───────────────────────────────────────────────────────────

function stretcher(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.9, 0.14, 0.72, palette.white, radii.m), 0, 0.72, 0));
  g.add(at(box(0.9, 0.1, 0.66, palette.coralSoft, radii.m), -0.4, 0.83, 0));
  g.add(at(box(0.34, 0.12, 0.5, palette.creamLight, radii.m), 0.62, 0.83, 0));
  for (const sx of [-0.7, 0.7]) {
    g.add(at(cylinder(0.045, 0.045, 0.62, palette.metalMatt, 10), sx, 0.36, 0));
  }
  g.add(at(box(1.5, 0.07, 0.4, palette.metalMatt, radii.s), 0, 0.12, 0));
  castering(g, 1.5, 0.5);
  return g;
}

function defibrillator(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.5, 0.6, 0.4, palette.white, radii.m), 0, 0.72, 0));
  g.add(at(cylinder(0.05, 0.05, 0.5, palette.metalMatt, 10), 0, 0.24, 0));
  g.add(at(cylinder(0.16, 0.2, 0.06, palette.metalMatt), 0, 0.03, 0));
  g.add(at(screen(0.26, 0.18), 0, 0.86, 0.18));
  g.add(at(box(0.12, 0.08, 0.1, palette.coral, radii.xs), -0.14, 1.06, 0.06));
  g.add(at(box(0.12, 0.08, 0.1, palette.coral, radii.xs), 0.14, 1.06, 0.06));
  return g;
}

function shockroomLight(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.07, 0.09, 2.1, palette.white, 12), 0.85, 1.05, 0));
  const arm = cylinder(0.05, 0.05, 0.95, palette.white, 10);
  arm.rotation.z = Math.PI / 2;
  g.add(at(arm, 0.4, 2.05, 0));
  const disc = cylinder(0.3, 0.34, 0.1, palette.white, 20);
  g.add(at(disc, -0.1, 2.0, 0));
  const face = new THREE.Mesh(new THREE.CylinderGeometry(0.24, 0.24, 0.02, 20), glowMat(kswPalette.opLight));
  g.add(at(face, -0.1, 1.94, 0));
  return g;
}

function ambulance(): THREE.Group {
  const g = new THREE.Group();
  // box body + lower cab nose, Swiss-style white with coral stripe
  g.add(at(box(4.0, 1.9, 2.0, palette.white, radii.l), -0.35, 1.35, 0));
  g.add(at(box(1.4, 1.3, 1.9, palette.white, radii.l), 2.0, 1.05, 0));
  g.add(at(box(4.1, 0.3, 2.04, palette.coral, radii.s), -0.35, 1.1, 0));
  g.add(at(box(0.02, 0.55, 1.1, kswPalette.crossRed, radii.xs), -2.36, 1.7, 0));
  // windshield + cab side windows
  const shield = new THREE.Mesh(new THREE.BoxGeometry(0.06, 0.6, 1.6), glassMat());
  shield.rotation.z = -0.35;
  g.add(at(shield, 2.62, 1.35, 0));
  for (const side of [-1, 1]) {
    const win = new THREE.Mesh(new THREE.BoxGeometry(0.9, 0.5, 0.04), glassMat());
    win.rotation.y = Math.PI / 2;
    g.add(at(win, 1.95, 1.35, side * 0.96));
  }
  // wheels
  for (const [wx, wz] of [
    [1.7, 1.0],
    [1.7, -1.0],
    [-1.4, 1.0],
    [-1.4, -1.0],
  ] as const) {
    const wheel = torus(0.32, 0.14, palette.eye);
    wheel.rotation.y = Math.PI / 2;
    g.add(at(wheel, wx, 0.42, wz));
    g.add(at(sphere(0.14, palette.metalMatt, 10), wx, 0.42, wz));
  }
  // roof light bar: the coral cap pulses (userData.blink drives it in main.ts)
  g.add(at(box(0.7, 0.12, 1.2, palette.metalMatt, radii.s), 0.9, 2.36, 0));
  const blink = new THREE.Mesh(new THREE.BoxGeometry(0.4, 0.12, 0.5), glowMat(palette.coral).clone());
  blink.userData.blink = true;
  g.add(at(blink, 0.9, 2.46, 0));
  return g;
}

// ── surgery ─────────────────────────────────────────────────────────────

function opTable(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.7, 0.25, 0.55, palette.metalMatt, radii.m), 0, 0.13, 0));
  g.add(at(cylinder(0.22, 0.26, 0.55, palette.metalMatt, 16), 0, 0.5, 0));
  g.add(at(box(2.1, 0.16, 0.8, palette.white, radii.m), 0, 0.88, 0));
  g.add(at(box(1.0, 0.1, 0.74, palette.mint, radii.m), -0.4, 1.0, 0));
  g.add(at(box(0.4, 0.1, 0.6, palette.creamLight, radii.m), 0.7, 1.0, 0));
  return g;
}

function opLightDouble(): THREE.Group {
  const g = new THREE.Group();
  const poleX = 1.35;
  const poleZ = 1.0;
  g.add(at(cylinder(0.09, 0.12, 2.55, palette.white, 14), poleX, 1.28, poleZ));
  for (const [ax, az, dy] of [
    [-0.35, 0.15, 2.5],
    [0.3, -0.75, 2.3],
  ] as const) {
    // straight arm from the pole top to the lamp head, aimed and sized exactly
    const from = new THREE.Vector3(poleX, dy + 0.06, poleZ);
    const to = new THREE.Vector3(ax, dy + 0.06, az);
    const len = from.distanceTo(to);
    const arm = cylinder(0.055, 0.055, len, palette.white, 10);
    arm.position.copy(from.clone().add(to).multiplyScalar(0.5));
    arm.rotation.z = Math.PI / 2;
    arm.rotation.y = -Math.atan2(to.z - from.z, to.x - from.x);
    g.add(arm);
    const disc = cylinder(0.42, 0.46, 0.12, palette.white, 24);
    g.add(at(disc, ax, dy - 0.05, az));
    const face = new THREE.Mesh(new THREE.CylinderGeometry(0.34, 0.34, 0.02, 24), glowMat(kswPalette.opLight));
    g.add(at(face, ax, dy - 0.13, az));
  }
  return g;
}

function anesthesiaMachine(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.8, 1.3, 0.6, palette.white, radii.m), 0, 0.75, 0));
  g.add(at(screen(0.42, 0.3), 0, 1.55, 0.16));
  g.add(at(cylinder(0.09, 0.09, 0.55, palette.mint, 12), -0.24, 0.85, 0.34));
  g.add(at(cylinder(0.09, 0.09, 0.55, palette.sage, 12), 0.0, 0.85, 0.38));
  const hose = torus(0.28, 0.045, palette.metalMatt, Math.PI);
  hose.rotation.x = Math.PI / 2;
  g.add(at(hose, 0.3, 1.35, 0.3));
  castering(g, 0.6, 0.45);
  return g;
}

function instrumentTable(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.9, 0.06, 0.55, palette.metalMatt, radii.s), 0, 0.85, 0));
  g.add(at(cylinder(0.05, 0.05, 0.8, palette.metalMatt, 10), 0, 0.45, 0));
  g.add(at(cylinder(0.2, 0.26, 0.06, palette.metalMatt), 0, 0.03, 0));
  g.add(at(box(0.2, 0.04, 0.08, palette.white, radii.xs), -0.2, 0.9, 0.1));
  g.add(at(box(0.24, 0.04, 0.06, palette.mint, radii.xs), 0.1, 0.9, -0.08));
  g.add(at(box(0.14, 0.05, 0.14, palette.sage, radii.xs), 0.28, 0.9, 0.12));
  return g;
}

function scrubSink(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.2, 0.5, 0.5, palette.white, radii.m), 0, 0.75, 0));
  g.add(at(box(1.1, 0.1, 0.4, palette.glass, radii.s), 0, 1.02, 0));
  for (const sx of [-0.3, 0.3]) {
    const tap = torus(0.11, 0.035, palette.metalMatt, Math.PI);
    g.add(at(tap, sx, 1.14, -0.05));
  }
  g.add(at(box(1.2, 0.25, 0.06, palette.mint, radii.s), 0, 1.45, -0.22));
  return g;
}

// ── intensive care ──────────────────────────────────────────────────────

function icuBed(): THREE.Group {
  const g = hospitalBed();
  const bridge = cylinder(0.045, 0.045, 1.1, palette.white, 10);
  bridge.rotation.x = Math.PI / 2;
  g.add(at(bridge, 0.9, 1.75, 0));
  g.add(at(cylinder(0.05, 0.06, 1.75, palette.white, 10), 0.9, 0.88, -0.55));
  g.add(at(screen(0.34, 0.24), 0.9, 1.45, -0.52));
  g.add(at(box(0.2, 0.3, 0.09, palette.mint, radii.s), 0.9, 1.5, 0.4));
  return g;
}

function ventilator(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.6, 1.0, 0.5, palette.white, radii.m), 0, 0.62, 0));
  g.add(at(screen(0.36, 0.26), 0, 1.28, 0.1));
  g.add(at(cylinder(0.12, 0.12, 0.35, palette.glass, 14), -0.14, 0.62, 0.3));
  const hose = torus(0.22, 0.04, palette.mint, Math.PI * 0.8);
  hose.rotation.x = Math.PI / 3;
  g.add(at(hose, 0.26, 1.05, 0.24));
  castering(g, 0.44, 0.36);
  return g;
}

// ── imaging ─────────────────────────────────────────────────────────────

function xrayMachine(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(2.0, 0.5, 0.9, palette.white, radii.m), 0, 0.3, 0));
  g.add(at(box(1.9, 0.12, 0.8, palette.mint, radii.m), 0, 0.6, 0));
  g.add(at(cylinder(0.11, 0.13, 2.0, palette.white, 14), -1.25, 1.0, 0));
  const arm = cylinder(0.07, 0.07, 1.3, palette.white, 10);
  arm.rotation.z = Math.PI / 2;
  g.add(at(arm, -0.6, 1.9, 0));
  g.add(at(box(0.5, 0.3, 0.5, palette.sage, radii.m), 0.05, 1.75, 0));
  return g;
}

function ctScanner(): THREE.Group {
  const g = new THREE.Group();
  const gantry = torus(1.0, 0.42, palette.white);
  gantry.rotation.y = Math.PI / 2;
  g.add(at(gantry, -0.9, 1.25, 0));
  g.add(at(box(0.9, 0.5, 2.3, palette.white, radii.l), -0.9, 0.25, 0));
  g.add(at(box(0.32, 0.1, 0.32, palette.mint, radii.s), -0.9, 2.0, 0));
  // patient couch sliding into the bore
  g.add(at(box(0.7, 0.35, 0.5, palette.white, radii.m), 1.1, 0.18, 0));
  g.add(at(box(2.6, 0.14, 0.6, palette.white, radii.m), 0.7, 0.62, 0));
  g.add(at(box(1.2, 0.08, 0.54, palette.mint, radii.s), 1.2, 0.73, 0));
  return g;
}

function mriScanner(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.6, 2.2, 2.4, palette.white, radii.l), -1.0, 1.12, 0));
  const bore = torus(0.62, 0.3, palette.creamLight);
  bore.rotation.y = Math.PI / 2;
  g.add(at(bore, -0.15, 1.15, 0));
  g.add(at(box(0.5, 0.14, 1.6, palette.mint, radii.s), -1.0, 2.28, 0));
  g.add(at(box(0.7, 0.35, 0.55, palette.white, radii.m), 1.5, 0.18, 0));
  g.add(at(box(3.0, 0.14, 0.62, palette.white, radii.m), 0.9, 0.62, 0));
  g.add(at(box(0.4, 0.12, 0.5, palette.creamLight, radii.s), 2.1, 0.73, 0));
  return g;
}

function cathLabArm(): THREE.Group {
  const g = new THREE.Group();
  const c = torus(1.05, 0.13, palette.white, Math.PI);
  c.rotation.z = Math.PI / 2;
  c.rotation.y = Math.PI / 2;
  g.add(at(c, 0, 1.15, 0));
  g.add(at(box(0.5, 0.4, 0.5, palette.sage, radii.m), 0, 2.35, 0));
  g.add(at(box(0.5, 0.3, 0.5, palette.sage, radii.m), 0, 0.18, 0));
  g.add(at(cylinder(0.14, 0.18, 0.9, palette.white, 14), 1.3, 0.45, 0));
  return g;
}

function leadShieldWindow(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.6, 0.9, 0.16, palette.creamBase, radii.s), 0, 0.45, 0));
  g.add(at(box(1.5, 0.9, 0.12, palette.white, radii.s), 0, 1.32, 0));
  const pane = new THREE.Mesh(new THREE.BoxGeometry(1.3, 0.7, 0.04), glassMat());
  g.add(at(pane, 0, 1.32, 0));
  return g;
}

function radiologyConsole(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.4, 0.72, 0.55, palette.woodSoft, radii.m), 0, 0.38, 0));
  g.add(at(screen(0.44, 0.32), -0.32, 1.0, -0.05));
  g.add(at(screen(0.44, 0.32), 0.32, 1.0, -0.05));
  g.add(at(box(0.5, 0.04, 0.2, palette.white, radii.xs), 0, 0.78, 0.14));
  return g;
}

// ── lab / pharmacy ──────────────────────────────────────────────────────

function labBench(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(2.4, 0.85, 0.7, palette.sage, radii.m), 0, 0.45, 0));
  g.add(at(box(2.5, 0.08, 0.8, palette.white, radii.s), 0, 0.92, 0));
  g.add(at(box(2.3, 0.35, 0.14, palette.white, radii.s), 0, 1.55, -0.28));
  g.add(at(cylinder(0.045, 0.045, 0.16, palette.mint, 10), -0.7, 1.04, 0.1));
  g.add(at(cylinder(0.045, 0.045, 0.14, palette.coralSoft, 10), -0.5, 1.03, -0.05));
  g.add(at(cylinder(0.05, 0.05, 0.18, palette.plantGreen, 10), 0.9, 1.05, 0.05));
  return g;
}

function microscope(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.24, 0.06, 0.3, palette.metalMatt, radii.xs), 0, 0.99, 0));
  const arm = cylinder(0.035, 0.035, 0.3, palette.metalMatt, 8);
  arm.rotation.x = 0.5;
  g.add(at(arm, 0, 1.14, -0.06));
  const tube = cylinder(0.045, 0.045, 0.22, palette.white, 10);
  tube.rotation.x = 0.5;
  g.add(at(tube, 0, 1.3, 0.02));
  return g;
}

function centrifuge(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.2, 0.24, 0.28, palette.white, 18), 0, 1.1, 0));
  g.add(at(cylinder(0.16, 0.16, 0.05, palette.mint, 18), 0, 1.26, 0));
  g.add(at(box(0.1, 0.05, 0.06, palette.eye, radii.xs), 0.16, 1.1, 0.12));
  return g;
}

function sampleRack(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.4, 0.08, 0.2, palette.woodSoft, radii.xs), 0, 1.0, 0));
  let i = 0;
  for (const color of [palette.mint, palette.coralSoft, palette.honey, palette.plantGreen, palette.glass]) {
    g.add(at(cylinder(0.022, 0.022, 0.14, color, 8), -0.15 + i * 0.075, 1.1, 0));
    i++;
  }
  return g;
}

function pharmacyShelf(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(2.2, 2.0, 0.5, palette.woodSoft, radii.m), 0, 1.0, 0));
  for (const y of [0.6, 1.1, 1.6]) {
    g.add(at(box(2.0, 0.06, 0.44, palette.creamLight, radii.xs), 0, y, 0.02));
    let i = 0;
    for (const color of [palette.mint, palette.white, palette.coralSoft, palette.sage, palette.honey, palette.white]) {
      g.add(at(cylinder(0.06, 0.06, 0.22, color, 10), -0.8 + i * 0.32, y + 0.16, 0.08));
      i++;
    }
  }
  return g;
}

// ── birth / neonatology ─────────────────────────────────────────────────

function birthingBed(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.9, 0.4, 1.1, palette.woodSoft, radii.m), 0, 0.34, 0));
  g.add(at(box(1.2, 0.2, 1.0, palette.white, radii.m), -0.3, 0.64, 0));
  const back = box(0.7, 0.2, 1.0, palette.white, radii.m);
  back.rotation.z = -0.55;
  g.add(at(back, 0.6, 0.82, 0));
  g.add(at(box(0.5, 0.12, 0.9, palette.coralSoft, radii.m), -0.5, 0.76, 0));
  return g;
}

function incubator(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.0, 0.85, 0.65, palette.white, radii.m), 0, 0.45, 0));
  g.add(at(box(0.94, 0.1, 0.6, palette.mint, radii.s), 0, 0.93, 0));
  const dome = new THREE.Mesh(new THREE.SphereGeometry(0.34, 18, 12, 0, Math.PI * 2, 0, Math.PI / 2), glassMat());
  dome.scale.set(1.25, 0.85, 0.8);
  g.add(at(dome, 0, 0.98, 0));
  // tiny sleeping bean baby under the dome
  const baby = new THREE.Mesh(new THREE.CapsuleGeometry(0.09, 0.14, 6, 12), clayMat(palette.honey));
  baby.rotation.z = Math.PI / 2;
  baby.castShadow = true;
  g.add(at(baby, 0, 1.05, 0));
  castering(g, 0.8, 0.5);
  return g;
}

function babyCrib(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.85, 0.35, 0.55, palette.glass, radii.m), 0, 0.75, 0));
  g.add(at(box(0.75, 0.08, 0.45, palette.white, radii.s), 0, 0.78, 0));
  g.add(at(box(0.4, 0.1, 0.4, palette.mint, radii.s), -0.1, 0.86, 0));
  for (const [sx, sz] of [
    [-0.3, 0.18],
    [0.3, 0.18],
    [-0.3, -0.18],
    [0.3, -0.18],
  ] as const) {
    g.add(at(cylinder(0.04, 0.04, 0.6, palette.metalMatt, 8), sx, 0.3, sz));
  }
  return g;
}

// ── day clinics ─────────────────────────────────────────────────────────

function infusionChair(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.7, 0.35, 0.75, palette.sage, radii.l), 0, 0.35, 0));
  const back = box(0.7, 0.85, 0.2, palette.sage, radii.l);
  back.rotation.x = 0.25;
  g.add(at(back, 0, 0.85, -0.38));
  g.add(at(box(0.14, 0.12, 0.6, palette.woodSoft, radii.s), -0.4, 0.58, 0));
  g.add(at(box(0.14, 0.12, 0.6, palette.woodSoft, radii.s), 0.4, 0.58, 0));
  g.add(at(box(0.55, 0.16, 0.5, palette.creamLight, radii.m), 0, 0.5, 0.1));
  return g;
}

function dialysisMachine(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.62, 1.35, 0.5, palette.white, radii.m), 0, 0.8, 0));
  g.add(at(screen(0.34, 0.24), 0, 1.62, 0.1));
  g.add(at(cylinder(0.1, 0.1, 0.4, palette.glass, 14), -0.16, 0.9, 0.3));
  g.add(at(cylinder(0.07, 0.07, 0.3, palette.coralSoft, 12), 0.14, 0.85, 0.3));
  const loop = torus(0.18, 0.035, palette.mint, Math.PI * 1.3);
  g.add(at(loop, 0.1, 1.25, 0.28));
  castering(g, 0.46, 0.36);
  return g;
}

// ── physio ──────────────────────────────────────────────────────────────

function physioTable(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.9, 0.16, 0.75, palette.mint, radii.m), 0, 0.62, 0));
  g.add(at(box(0.4, 0.1, 0.65, palette.white, radii.m), 0.7, 0.74, 0));
  for (const [sx, sz] of [
    [-0.8, 0.28],
    [0.8, 0.28],
    [-0.8, -0.28],
    [0.8, -0.28],
  ] as const) {
    g.add(at(box(0.12, 0.55, 0.12, palette.woodSoft, radii.s), sx, 0.28, sz));
  }
  return g;
}

function exerciseBike(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.1, 0.14, 0.4, palette.metalMatt, radii.m), 0, 0.08, 0));
  const wheel = cylinder(0.32, 0.32, 0.12, palette.sage, 20);
  wheel.rotation.x = Math.PI / 2;
  g.add(at(wheel, -0.3, 0.45, 0));
  g.add(at(cylinder(0.06, 0.07, 0.55, palette.metalMatt, 10), 0.32, 0.42, 0));
  g.add(at(box(0.34, 0.1, 0.22, palette.coralSoft, radii.m), 0.32, 0.74, 0));
  g.add(at(cylinder(0.05, 0.06, 0.7, palette.metalMatt, 10), -0.42, 0.55, 0));
  const bars = cylinder(0.04, 0.04, 0.5, palette.woodSoft, 8);
  bars.rotation.x = Math.PI / 2;
  g.add(at(bars, -0.42, 0.95, 0));
  return g;
}

function parallelBars(): THREE.Group {
  const g = new THREE.Group();
  for (const side of [-1, 1]) {
    const rail = cylinder(0.05, 0.05, 2.2, palette.woodSoft, 10);
    rail.rotation.x = Math.PI / 2;
    g.add(at(rail, side * 0.35, 0.95, 0));
    for (const zc of [-0.9, 0.9]) {
      g.add(at(cylinder(0.05, 0.06, 0.95, palette.metalMatt, 10), side * 0.35, 0.48, zc));
    }
  }
  g.add(at(box(1.1, 0.06, 2.4, palette.mint, radii.s), 0, 0.03, 0));
  return g;
}

function gymBall(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(sphere(0.32, palette.coralSoft, 20), 0, 0.32, 0));
  return g;
}

// ── endoscopy ───────────────────────────────────────────────────────────

function endoscopyTower(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.7, 1.7, 0.6, palette.white, radii.m), 0, 0.95, 0));
  g.add(at(screen(0.5, 0.36), 0, 1.95, 0.05));
  g.add(at(box(0.6, 0.16, 0.5, palette.mint, radii.s), 0, 1.4, 0.08));
  g.add(at(box(0.6, 0.16, 0.5, palette.sage, radii.s), 0, 1.1, 0.08));
  const scope = torus(0.2, 0.035, palette.eye, Math.PI * 1.5);
  g.add(at(scope, 0.28, 0.7, 0.3));
  castering(g, 0.5, 0.44);
  return g;
}

// ── office / cafeteria ──────────────────────────────────────────────────

function officeDesk(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(1.5, 0.08, 0.75, palette.woodSoft, radii.s), 0, 0.72, 0));
  for (const [sx, sz] of [
    [-0.65, 0.3],
    [0.65, 0.3],
    [-0.65, -0.3],
    [0.65, -0.3],
  ] as const) {
    g.add(at(box(0.1, 0.7, 0.1, palette.woodSoft, radii.s), sx, 0.36, sz));
  }
  g.add(at(screen(0.42, 0.3), 0.2, 1.02, -0.15));
  g.add(at(cylinder(0.045, 0.045, 0.18, palette.white, 8), 0.2, 0.84, -0.15));
  g.add(at(box(0.4, 0.03, 0.16, palette.white, radii.xs), -0.15, 0.78, 0.12));
  return g;
}

function officeChair(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.5, 0.12, 0.5, palette.sage, radii.m), 0, 0.5, 0));
  g.add(at(box(0.5, 0.55, 0.12, palette.sage, radii.m), 0, 0.85, -0.22));
  g.add(at(cylinder(0.05, 0.05, 0.4, palette.metalMatt, 10), 0, 0.25, 0));
  g.add(at(cylinder(0.2, 0.24, 0.06, palette.metalMatt), 0, 0.03, 0));
  return g;
}

function filingCabinet(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.55, 1.4, 0.5, palette.sage, radii.m), 0, 0.7, 0));
  for (const y of [0.35, 0.7, 1.05]) {
    g.add(at(box(0.42, 0.02, 0.03, palette.metalMatt, radii.xs), 0, y, 0.26));
  }
  return g;
}

function deskPlant(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.08, 0.1, 0.14, palette.plantPot, 12), 0, 0.79, 0));
  g.add(at(sphere(0.12, palette.plantGreen, 12), 0, 0.94, 0));
  return g;
}

function cafeTable(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.45, 0.45, 0.06, palette.woodSoft, 24), 0, 0.72, 0));
  g.add(at(cylinder(0.06, 0.07, 0.7, palette.metalMatt, 12), 0, 0.36, 0));
  g.add(at(cylinder(0.24, 0.28, 0.05, palette.metalMatt), 0, 0.025, 0));
  g.add(at(cylinder(0.05, 0.045, 0.1, palette.white, 12), 0.14, 0.8, 0.05));
  g.add(at(cylinder(0.05, 0.045, 0.1, palette.mint, 12), -0.14, 0.8, -0.08));
  return g;
}

function cafeChair(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.42, 0.1, 0.42, palette.mint, radii.m), 0, 0.44, 0));
  g.add(at(box(0.42, 0.45, 0.1, palette.mint, radii.m), 0, 0.72, -0.18));
  for (const [sx, sz] of [
    [-0.16, 0.16],
    [0.16, 0.16],
    [-0.16, -0.16],
    [0.16, -0.16],
  ] as const) {
    g.add(at(box(0.08, 0.4, 0.08, palette.woodSoft, radii.xs), sx, 0.2, sz));
  }
  return g;
}

function counterBar(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(3.0, 1.0, 0.7, palette.woodSoft, radii.l), 0, 0.5, 0));
  g.add(at(box(3.15, 0.1, 0.85, palette.creamLight, radii.m), 0, 1.05, 0));
  g.add(at(box(0.5, 0.28, 0.35, palette.glass, radii.s), -1.0, 1.24, 0));
  g.add(at(cylinder(0.14, 0.16, 0.24, palette.coralSoft, 14), 0.2, 1.22, 0));
  g.add(at(box(0.4, 0.16, 0.28, palette.honey, radii.s), 1.0, 1.18, 0));
  return g;
}

function espressoMachine(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(box(0.5, 0.4, 0.4, palette.metalMatt, radii.m), 0, 1.32, 0));
  g.add(at(box(0.44, 0.08, 0.34, palette.coral, radii.s), 0, 1.56, 0));
  g.add(at(cylinder(0.035, 0.035, 0.12, palette.metalDark, 8), -0.1, 1.1, 0.12));
  g.add(at(cylinder(0.045, 0.04, 0.09, palette.white, 10), -0.1, 1.02, 0.12));
  return g;
}

// ── outdoor ─────────────────────────────────────────────────────────────

function tree(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(0.14, 0.2, 1.1, palette.woodSoft, 10), 0, 0.55, 0));
  const puffs: Array<[number, number, number, number]> = [
    [0, 1.6, 0, 0.75],
    [0.45, 1.25, 0.2, 0.48],
    [-0.4, 1.35, -0.18, 0.52],
    [0.1, 1.2, -0.42, 0.42],
  ];
  for (const [x, y, z, r] of puffs) g.add(at(sphere(r, palette.plantGreen, 16), x, y, z));
  return g;
}

function helipad(): THREE.Group {
  const g = new THREE.Group();
  g.add(at(cylinder(3.0, 3.1, 0.12, kswPalette.plazaPath, 36), 0, 0.06, 0));
  const rim = torus(2.7, 0.09, palette.coral);
  rim.rotation.x = Math.PI / 2;
  g.add(at(rim, 0, 0.13, 0));
  g.add(at(box(0.4, 0.05, 2.0, palette.white, radii.xs), -0.7, 0.15, 0));
  g.add(at(box(0.4, 0.05, 2.0, palette.white, radii.xs), 0.7, 0.15, 0));
  g.add(at(box(1.0, 0.05, 0.4, palette.white, radii.xs), 0, 0.15, 0));
  return g;
}

function plantSmall(scale: number): THREE.Group {
  const g = plant();
  g.scale.setScalar(scale);
  return g;
}

// ── people ──────────────────────────────────────────────────────────────

export function beanPerson(bodyColor: number): THREE.Group {
  const g = new THREE.Group();
  const body = new THREE.Mesh(new THREE.CapsuleGeometry(0.34, 0.55, 8, 24), clayMat(bodyColor));
  body.position.y = 0.62;
  body.castShadow = true;
  body.receiveShadow = true;
  g.add(body);
  const eyeGeo = new THREE.SphereGeometry(0.052, 12, 12);
  for (const side of [-1, 1]) {
    const eye = new THREE.Mesh(eyeGeo, clayMat(palette.eye));
    eye.position.set(side * 0.105, 0.92, 0.305);
    g.add(eye);
  }
  const mouth = new THREE.Mesh(new THREE.CapsuleGeometry(0.02, 0.06, 4, 8), clayMat(palette.eye));
  mouth.rotation.z = Math.PI / 2;
  mouth.position.set(0, 0.8, 0.33);
  g.add(mouth);
  return g;
}

function badge(g: THREE.Group): void {
  g.add(at(box(0.11, 0.14, 0.03, palette.white, radii.xs), 0.14, 0.72, 0.31));
}

export function buildPerson(p: PersonPlacement): THREE.Group {
  let g: THREE.Group;
  switch (p.role) {
    case 'nurse': {
      g = beanPerson(palette.mint);
      badge(g);
      break;
    }
    case 'doctor': {
      g = beanPerson(palette.white);
      const scope = torus(0.2, 0.028, palette.eye, Math.PI);
      scope.rotation.x = Math.PI;
      g.add(at(scope, 0, 0.86, 0.26));
      break;
    }
    case 'surgeon': {
      g = beanPerson(palette.sage);
      g.add(at(cylinder(0.24, 0.28, 0.14, palette.mint, 18), 0, 1.06, 0));
      g.add(at(box(0.26, 0.14, 0.05, palette.white, radii.xs), 0, 0.8, 0.31));
      break;
    }
    case 'patient':
      g = beanPerson(palette.coral);
      break;
    case 'child': {
      g = beanPerson(palette.honey);
      g.scale.setScalar(0.68);
      break;
    }
    case 'visitor':
      g = beanPerson(palette.skin);
      break;
    case 'labtech': {
      g = beanPerson(palette.white);
      badge(g);
      for (const side of [-1, 1]) {
        const rim = torus(0.07, 0.016, palette.eye);
        g.add(at(rim, side * 0.105, 0.92, 0.3));
      }
      break;
    }
    case 'paramedic': {
      g = beanPerson(palette.coralSoft);
      g.add(at(box(0.4, 0.12, 0.06, palette.white, radii.xs), 0, 0.66, 0.29));
      break;
    }
  }
  g.rotation.y = p.yaw;
  g.position.set(p.x, 0, p.z);
  return g;
}

// ── registry ────────────────────────────────────────────────────────────

export const propBuilders: Record<string, () => THREE.Group> = {
  hospitalBed,
  careCart,
  ivStand,
  vitalsMonitor,
  plant,
  sideTable,
  receptionDesk,
  triageDesk,
  counterDesk,
  waitingBench,
  infoBoard,
  handSanitizer,
  wheelchair,
  linenCart,
  stretcher,
  defibrillator,
  shockroomLight,
  ambulance,
  opTable,
  opLightDouble,
  anesthesiaMachine,
  instrumentTable,
  scrubSink,
  icuBed,
  ventilator,
  xrayMachine,
  ctScanner,
  mriScanner,
  cathLabArm,
  leadShieldWindow,
  radiologyConsole,
  labBench,
  microscope,
  centrifuge,
  sampleRack,
  pharmacyShelf,
  birthingBed,
  incubator,
  babyCrib,
  infusionChair,
  dialysisMachine,
  physioTable,
  exerciseBike,
  parallelBars,
  gymBall,
  endoscopyTower,
  officeDesk,
  officeChair,
  filingCabinet,
  deskPlant,
  cafeTable,
  cafeChair,
  counterBar,
  espressoMachine,
  tree,
  helipad,
};

export function buildProp(p: PropPlacement): THREE.Group {
  const builder = propBuilders[p.kind];
  if (!builder) throw new Error(`unknown prop kind: ${p.kind}`);
  const g = builder();
  if (p.scale) g.scale.setScalar(p.scale);
  if (p.rotY) g.rotation.y = p.rotY;
  g.position.set(p.x, 0, p.z);
  return g;
}
