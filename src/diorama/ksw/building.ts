// Builds the KSW hospital shell from the declarative floor plan: banded
// walls with door/window openings, per-room floors, corridor floors, plaza
// slabs, department signage, and per-room roofs that fade with the camera
// zoom. Roof meshes cast and receive shadows while present, so the sun
// treats the building as a solid volume until you zoom inside.

import * as THREE from 'three/webgpu';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import { clay, kswPalette, kswScene, palette, radii } from '../designTokens';
import type { FloorPlan, Room, WallSide } from './floorPlan';
import { box, buildPerson, buildProp, glassMat } from './props';

export type WallOpening = { center: number; width: number; kind: 'door' | 'window' };

export type WallSegment = { c0: number; c1: number; y0: number; y1: number };

// Decompose one wall (local coordinates, origin at wall midpoint) into solid
// band segments around its openings. Doors reach the floor, windows start at
// the sill; both share one head height, so the top band is always continuous.
export function segmentWall(length: number, height: number, openings: WallOpening[]): WallSegment[] {
  const sill = kswScene.openingSill;
  const head = kswScene.openingHead;
  const lap = 0.06;
  const segments: WallSegment[] = [];

  if (height - head > 0.05) segments.push({ c0: -length / 2, c1: length / 2, y0: head - lap, y1: height });

  const bands: Array<{ y0: number; y1: number; cuts: WallOpening[] }> = [
    { y0: sill, y1: head, cuts: openings }, // mid band: doors + windows cut it
    { y0: 0, y1: sill + lap, cuts: openings.filter((o) => o.kind === 'door') }, // base band: doors only
  ];
  for (const band of bands) {
    const sorted = [...band.cuts].sort((a, b) => a.center - b.center);
    let cursor = -length / 2;
    for (const o of sorted) {
      const left = o.center - o.width / 2;
      if (left - cursor > 0.05) segments.push({ c0: cursor, c1: left, y0: band.y0, y1: band.y1 });
      cursor = Math.max(cursor, o.center + o.width / 2);
    }
    if (length / 2 - cursor > 0.05) segments.push({ c0: cursor, c1: length / 2, y0: band.y0, y1: band.y1 });
  }
  return segments;
}

export type RoofControl = { setFade(fade01: number): void; fade(): number };

const FLOOR_H = 0.14;

function wallGroup(length: number, thickness: number, color: number, openings: WallOpening[]): THREE.Group {
  const g = new THREE.Group();
  const height = kswScene.wallHeight;
  for (const s of segmentWall(length, height, openings)) {
    const seg = box(s.c1 - s.c0, s.y1 - s.y0, thickness, color, radii.xs);
    seg.position.set((s.c0 + s.c1) / 2, (s.y0 + s.y1) / 2, 0);
    g.add(seg);
  }
  const sill = kswScene.openingSill;
  const head = kswScene.openingHead;
  for (const o of openings) {
    const y0 = o.kind === 'door' ? 0 : sill;
    const fh = head - y0;
    const ft = 0.1;
    const depth = thickness + 0.08;
    if (o.kind === 'window') {
      const bottom = box(o.width + 0.16, ft, depth, palette.white, radii.xs);
      bottom.position.set(o.center, y0 + ft / 2 - 0.02, 0);
      g.add(bottom);
    }
    const top = box(o.width + 0.16, ft, depth, palette.white, radii.xs);
    top.position.set(o.center, head - ft / 2 + 0.02, 0);
    g.add(top);
    for (const side of [-1, 1]) {
      const jamb = box(ft, fh, depth, palette.white, radii.xs);
      jamb.position.set(o.center + side * (o.width / 2 + 0.03), y0 + fh / 2, 0);
      g.add(jamb);
    }
    if (o.kind === 'window') {
      const mullion = box(0.07, fh - 0.1, 0.09, palette.white, radii.xs);
      mullion.position.set(o.center, y0 + fh / 2, 0);
      g.add(mullion);
      const pane = new THREE.Mesh(new THREE.BoxGeometry(o.width - 0.04, fh - 0.06, 0.03), glassMat());
      pane.position.set(o.center, y0 + fh / 2, 0);
      g.add(pane);
    }
  }
  return g;
}

function isPerimeter(plan: FloorPlan, room: Room, side: WallSide): boolean {
  const b = plan.building;
  const r = room.rect;
  const EPS = 1e-6;
  switch (side) {
    case 'n':
      return Math.abs(r.z - r.d / 2 - (b.z - b.d / 2)) < EPS;
    case 's':
      return Math.abs(r.z + r.d / 2 - (b.z + b.d / 2)) < EPS;
    case 'w':
      return Math.abs(r.x - r.w / 2 - (b.x - b.w / 2)) < EPS;
    case 'e':
      return Math.abs(r.x + r.w / 2 - (b.x + b.w / 2)) < EPS;
  }
}

function buildRoomWalls(plan: FloorPlan, room: Room): THREE.Group {
  const g = new THREE.Group();
  const r = room.rect;
  const EPSI = 0.012; // keep neighbouring rooms' wall faces off the shared plane
  for (const side of ['n', 's', 'e', 'w'] as WallSide[]) {
    const exterior = isPerimeter(plan, room, side);
    const t = exterior ? kswScene.wallThicknessOuter : kswScene.wallThicknessInner;
    const color = exterior ? palette.creamBase : palette.creamLight;
    const along = side === 'n' || side === 's' ? r.w : r.d;
    const openings: WallOpening[] = [
      ...room.doors.filter((d) => d.wall === side).map((d) => ({ center: d.center, width: d.width, kind: 'door' as const })),
      ...room.windows.filter((w) => w.wall === side).map((w) => ({ center: w.center, width: w.width, kind: 'window' as const })),
    ];
    const wall = wallGroup(along - 2 * EPSI, t, color, openings);
    wall.position.y = FLOOR_H;
    switch (side) {
      case 'n':
        wall.position.x = r.x;
        wall.position.z = r.z - r.d / 2 + t / 2 + EPSI;
        break;
      case 's':
        wall.position.x = r.x;
        wall.position.z = r.z + r.d / 2 - t / 2 - EPSI;
        wall.rotation.y = Math.PI;
        break;
      case 'w':
        wall.position.x = r.x - r.w / 2 + t / 2 + EPSI;
        wall.position.z = r.z;
        wall.rotation.y = Math.PI / 2;
        break;
      case 'e':
        wall.position.x = r.x + r.w / 2 - t / 2 - EPSI;
        wall.position.z = r.z;
        wall.rotation.y = -Math.PI / 2;
        break;
    }
    g.add(wall);
  }
  return g;
}

function doorWorld(room: Room, wall: WallSide, center: number): { x: number; z: number; out: [number, number] } {
  const r = room.rect;
  switch (wall) {
    case 'n':
      return { x: r.x + center, z: r.z - r.d / 2, out: [0, -1] };
    case 's':
      return { x: r.x + center, z: r.z + r.d / 2, out: [0, 1] };
    case 'w':
      return { x: r.x - r.w / 2, z: r.z + center, out: [-1, 0] };
    case 'e':
      return { x: r.x + r.w / 2, z: r.z + center, out: [1, 0] };
  }
}

// Department sign over the first door, hung on the outside: accent slab,
// plus a Swiss-cross block for the emergency ward and the main entrance.
function buildSign(room: Room): THREE.Group {
  const g = new THREE.Group();
  const d = room.doors[0];
  const p = doorWorld(room, d.wall, d.center);
  const alongX = d.wall === 'n' || d.wall === 's';
  const sign = box(alongX ? 1.5 : 0.14, 0.34, alongX ? 0.14 : 1.5, room.accent, radii.s);
  sign.position.set(p.x + p.out[0] * 0.36, FLOOR_H + kswScene.openingHead + 0.34, p.z + p.out[1] * 0.36);
  g.add(sign);
  if (room.id === 'notfall' || room.id === 'empfang') {
    const block = box(0.55, 0.55, 0.55, kswPalette.crossRed, radii.m);
    block.position.set(p.x + p.out[0] * 0.5, FLOOR_H + kswScene.openingHead + 0.95, p.z + p.out[1] * 0.5);
    g.add(block);
    const barV = box(alongX ? 0.14 : 0.6, 0.38, alongX ? 0.6 : 0.14, palette.white, radii.xs);
    barV.position.copy(block.position);
    g.add(barV);
    const barH = box(alongX ? 0.38 : 0.6, 0.14, alongX ? 0.6 : 0.38, palette.white, radii.xs);
    barH.position.copy(block.position);
    g.add(barH);
  }
  return g;
}

export function buildHospital(plan: FloorPlan): { group: THREE.Group; roofs: RoofControl } {
  const group = new THREE.Group();

  // ground plate (soft lawn) + hard surfaces
  const plate = box(plan.plate.w, kswScene.plateThickness, plan.plate.d, palette.lawn, radii.l);
  plate.position.y = -kswScene.plateThickness / 2;
  group.add(plate);

  const plaza = box(22, 0.1, 9.5, kswPalette.plazaPath, radii.m);
  plaza.position.set(-2.5, 0.05, 21.5);
  group.add(plaza);
  const apron = box(13, 0.1, 9.5, kswPalette.plazaPath, radii.m);
  apron.position.set(-23.5, 0.05, 21.5);
  group.add(apron);

  // floors
  for (const room of plan.rooms) {
    const floor = box(room.rect.w, FLOOR_H, room.rect.d, palette.floorWarm, radii.s);
    floor.position.set(room.rect.x, FLOOR_H / 2, room.rect.z);
    group.add(floor);
    const inlay = box(room.rect.w * 0.45, 0.03, room.rect.d * 0.35, room.accent, radii.s);
    inlay.castShadow = false;
    inlay.position.set(room.rect.x, FLOOR_H + 0.015, room.rect.z);
    group.add(inlay);
  }
  for (const c of plan.corridors) {
    const floor = box(c.w, FLOOR_H, c.d, kswPalette.corridorFloor, radii.s);
    floor.position.set(c.x, FLOOR_H / 2, c.z);
    group.add(floor);
  }

  // walls + signage + interiors
  for (const room of plan.rooms) {
    group.add(buildRoomWalls(plan, room));
    group.add(buildSign(room));
    for (const p of room.props) group.add(withFloorLift(buildProp(p)));
    for (const p of room.people) group.add(withFloorLift(buildPerson(p)));
  }
  for (const p of plan.outdoorProps) group.add(buildProp(p));
  for (const p of plan.outdoorPeople) group.add(buildPerson(p));

  // roofs: one shared transparent clay material; slight per-room height
  // steps so overlapping lids never share a plane.
  const roofMat = new THREE.MeshPhysicalMaterial({
    color: kswPalette.roofClay,
    roughness: clay.roughness,
    metalness: clay.metalness,
    transparent: true,
    opacity: 1,
  });
  roofMat.sheen = clay.sheen;
  roofMat.sheenRoughness = clay.sheenRoughness;
  roofMat.sheenColor = new THREE.Color(kswPalette.roofClay).lerp(new THREE.Color(0xffffff), 0.5);
  const trimMat = new THREE.MeshPhysicalMaterial({
    color: kswPalette.roofTrim,
    roughness: clay.roughness,
    metalness: clay.metalness,
    transparent: true,
    opacity: 1,
  });

  const roofMeshes: THREE.Mesh[] = [];
  const roofGroup = new THREE.Group();
  const baseY = FLOOR_H + kswScene.wallHeight;
  const addRoof = (x: number, z: number, w: number, d: number, step: number, trim: boolean): void => {
    const over = kswScene.roofOverhang;
    const lid = new THREE.Mesh(
      new RoundedBoxGeometry(w + 2 * over, kswScene.roofThickness, d + 2 * over, 4, radii.m),
      roofMat,
    );
    lid.position.set(x, baseY + step + kswScene.roofThickness / 2, z);
    lid.castShadow = true;
    lid.receiveShadow = true;
    roofMeshes.push(lid);
    roofGroup.add(lid);
    if (trim) {
      const cap = new THREE.Mesh(
        new RoundedBoxGeometry(w * 0.6, 0.16, d * 0.6, 4, radii.m),
        trimMat,
      );
      cap.position.set(x, baseY + step + kswScene.roofThickness + 0.08, z);
      cap.castShadow = true;
      cap.receiveShadow = true;
      roofMeshes.push(cap);
      roofGroup.add(cap);
    }
  };
  plan.rooms.forEach((room, i) => {
    addRoof(room.rect.x, room.rect.z, room.rect.w, room.rect.d, 0.1 + (i % 3) * 0.07, true);
  });
  for (const c of plan.corridors) addRoof(c.x, c.z, c.w, c.d, 0, false);
  group.add(roofGroup);

  let currentFade = 1;
  const roofs: RoofControl = {
    setFade(fade01: number) {
      currentFade = Math.min(Math.max(fade01, 0), 1);
      roofMat.opacity = currentFade;
      trimMat.opacity = currentFade;
      const cast = currentFade > 0.5;
      const visible = currentFade > 0.02;
      for (const m of roofMeshes) {
        m.castShadow = cast;
        m.visible = visible;
      }
    },
    fade: () => currentFade,
  };
  return { group, roofs };
}

// Props/people inside rooms stand on the raised room floor.
function withFloorLift(g: THREE.Group): THREE.Group {
  g.position.y += FLOOR_H;
  return g;
}
