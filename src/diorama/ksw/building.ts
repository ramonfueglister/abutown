// Builds the KSW hospital shell from the declarative floor plan: banded
// walls with door/window openings, per-room floors, corridor floors, plaza
// slabs, department signage, and per-room roofs that fade with the camera
// zoom. Roof meshes cast and receive shadows while present, so the sun
// treats the building as a solid volume until you zoom inside.
//
// The builders emit individual Meshes; buildHospital's final step hoists
// them into a handful of BatchedMesh buckets (see staticBatch.ts), so the
// returned group renders in a few draw calls.

import * as THREE from 'three/webgpu';
import { kswPalette, kswScene, palette, radii } from '../designTokens';
import type { FloorPlan, Room, WallSide } from './floorPlan';
import { boxGeo, cyl, roundedBox } from './geometryCache';
import { box, buildProp, clayMat, glassMat } from './props';
import { batchHospital } from './staticBatch';

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

export const FLOOR_H = 0.14;

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
      const pane = new THREE.Mesh(boxGeo(o.width - 0.04, fh - 0.06, 0.03), glassMat());
      pane.position.set(o.center, y0 + fh / 2, 0);
      pane.userData.windowPane = true;
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

// Exported for the zone-ladder interior builder (T17, interior/buildInterior.ts)
// so it reuses the exact banded-wall recipe instead of duplicating it.
export function buildRoomWalls(plan: FloorPlan, room: Room): THREE.Group {
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
export function buildSign(room: Room): THREE.Group {
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

export function buildHospital(plan: FloorPlan, opts: { lampGlow: boolean }): { group: THREE.Group; roofs: RoofControl } {
  const group = new THREE.Group();

  // ground plate (soft lawn) + hard surfaces
  const plate = box(plan.plate.w, kswScene.plateThickness, plan.plate.d, palette.lawn, radii.l);
  plate.position.y = -kswScene.plateThickness / 2;
  group.add(plate);

  for (const s of plan.outdoorSlabs) {
    const slab = box(s.w, 0.1, s.d, kswPalette.plazaPath, radii.m);
    slab.position.set(s.x, 0.05, s.z);
    group.add(slab);
  }

  // floors
  for (const room of plan.rooms) {
    const floor = box(room.rect.w, FLOOR_H, room.rect.d, palette.floorWarm, radii.s);
    floor.position.set(room.rect.x, FLOOR_H / 2, room.rect.z);
    group.add(floor);
    const inlay = box(room.rect.w * 0.3, 0.03, room.rect.d * 0.22, room.accent, radii.s);
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
    // every street-side door gets a canopy on two posts
    for (const d of room.doors) {
      if (!isPerimeter(plan, room, d.wall)) continue;
      const p = doorWorld(room, d.wall, d.center);
      const alongX = d.wall === 'n' || d.wall === 's';
      const cw = d.width + 1.6;
      const cd = 1.7;
      const slab = box(alongX ? cw : cd, 0.16, alongX ? cd : cw, palette.white, radii.s);
      slab.position.set(p.x + p.out[0] * (cd / 2 + 0.2), FLOOR_H + kswScene.openingHead + 0.12, p.z + p.out[1] * (cd / 2 + 0.2));
      group.add(slab);
      for (const side of [-1, 1]) {
        const post = box(0.14, kswScene.openingHead, 0.14, palette.white, radii.xs);
        const offAlong = side * (cw / 2 - 0.2);
        post.position.set(
          p.x + p.out[0] * (cd - 0.1) + (alongX ? offAlong : 0),
          FLOOR_H + kswScene.openingHead / 2,
          p.z + p.out[1] * (cd - 0.1) + (alongX ? 0 : offAlong),
        );
        group.add(post);
      }
    }
    for (const p of room.props) group.add(withFloorLift(buildProp(p)));
  }
  for (const p of plan.corridorProps) group.add(withFloorLift(buildProp(p)));
  for (const p of plan.outdoorProps) group.add(buildProp(p));
  // people are NOT built here: main.ts spawns them as wandering agents

  // roofs: everything up here is tagged userData.roofFade so the batching
  // step collects it into the one transparent roofFade bucket whose opacity
  // the RoofControl drives. Slight per-room height steps so overlapping lids
  // never share a plane.
  const roofGroup = new THREE.Group();
  const baseY = FLOOR_H + kswScene.wallHeight;
  const EPS = 1e-6;
  const b = plan.building;
  // Overhang only where the rect meets the building perimeter; interior
  // edges stay flush so neighbouring lids read as distinct stepped tiers.
  const tagRoof = (m: THREE.Mesh): THREE.Mesh => {
    m.userData.roofFade = true;
    m.castShadow = true;
    m.receiveShadow = true;
    roofGroup.add(m);
    return m;
  };
  const addRoof = (rect: { x: number; z: number; w: number; d: number }, step: number, color: number, attika: boolean): void => {
    const over = kswScene.roofOverhang;
    const eW = Math.abs(rect.x - rect.w / 2 - (b.x - b.w / 2)) < EPS ? over : 0;
    const eE = Math.abs(rect.x + rect.w / 2 - (b.x + b.w / 2)) < EPS ? over : 0;
    const eN = Math.abs(rect.z - rect.d / 2 - (b.z - b.d / 2)) < EPS ? over : 0;
    const eS = Math.abs(rect.z + rect.d / 2 - (b.z + b.d / 2)) < EPS ? over : 0;
    const w = rect.w + eW + eE;
    const d = rect.d + eN + eS;
    const lid = tagRoof(new THREE.Mesh(roundedBox(w, kswScene.roofThickness, d, 4, radii.s), clayMat(color)));
    lid.position.set(rect.x + (eE - eW) / 2, baseY + step + kswScene.roofThickness / 2, rect.z + (eS - eN) / 2);
    if (attika && rect.w > 2.2 && rect.d > 2.2) {
      const cap = tagRoof(new THREE.Mesh(roundedBox(rect.w - 1.0, 0.12, rect.d - 1.0, 4, radii.s), clayMat(kswPalette.roofTrim)));
      cap.position.set(rect.x, baseY + step + kswScene.roofThickness + 0.06, rect.z);
    }
  };
  plan.rooms.forEach((room, i) => {
    addRoof(room.rect, 0.12 + (i % 3) * 0.09, kswPalette.roofClay, true);
  });
  for (const c of plan.corridors) addRoof(c, 0, kswPalette.roofTrim, false);

  // rooftop dressing: HVAC boxes, vents and solar rows — a Swiss flat roof
  // is never empty. All of it fades with the lids.
  const addRoofMesh = (geo: THREE.BufferGeometry, color: number, x: number, y: number, z: number, rotY = 0): void => {
    const m = tagRoof(new THREE.Mesh(geo, clayMat(color)));
    m.position.set(x, y, z);
    m.rotation.y = rotY;
  };
  plan.rooms.forEach((room, i) => {
    const step = 0.12 + (i % 3) * 0.09;
    const topY = baseY + step + kswScene.roofThickness + 0.12;
    const r = room.rect;
    if (i % 3 === 0) {
      addRoofMesh(roundedBox(1.2, 0.7, 0.9, 4, radii.s), palette.metalMatt, r.x - r.w * 0.22, topY + 0.28, r.z - r.d * 0.2, 0.2);
      addRoofMesh(cyl(0.16, 0.2, 0.5, 12), palette.white, r.x + r.w * 0.24, topY + 0.2, r.z + r.d * 0.18);
    } else if (i % 3 === 1) {
      // south-tilted solar row — w varies with room size, so this box stays uncached
      const panel = tagRoof(new THREE.Mesh(new THREE.BoxGeometry(Math.min(r.w * 0.5, 3.2), 0.06, 1.1), clayMat(palette.eye)));
      panel.position.set(r.x, topY + 0.34, r.z + r.d * 0.16);
      panel.rotation.x = -0.32;
      addRoofMesh(roundedBox(0.5, 0.36, 0.5, 4, radii.xs), palette.metalMatt, r.x - r.w * 0.28, topY + 0.14, r.z - r.d * 0.22);
    } else {
      addRoofMesh(cyl(0.2, 0.26, 0.62, 12), palette.white, r.x - r.w * 0.2, topY + 0.26, r.z + r.d * 0.2);
      addRoofMesh(roundedBox(0.8, 0.5, 0.7, 4, radii.s), palette.metalMatt, r.x + r.w * 0.22, topY + 0.2, r.z - r.d * 0.16, -0.15);
    }
  });
  group.add(roofGroup);

  const { roofs } = batchHospital(group, opts);
  return { group, roofs };
}

// Props/people inside rooms stand on the raised room floor.
export function withFloorLift(g: THREE.Group): THREE.Group {
  g.position.y += FLOOR_H;
  return g;
}
