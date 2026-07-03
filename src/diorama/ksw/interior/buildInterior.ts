// Zone-ladder interior builder (T17, S3b-2). Turns the generated FloorPlan
// (interior/generatePlan.ts) into a THREE.Group of just the INTERIOR: per-room
// floors + accent inlays, banded inner/outer room walls with door openings,
// department signage, corridor floors, and the room/corridor props. It
// deliberately builds NO outer shell, NO roofs and NO ground plate — the real
// baked KSW shell (geo/kswCampus.ts) already provides those; this drops the
// authored room language inside it (rendered via ?interior=1 until the T18
// cutaway reveals it properly).
//
// Every mesh recipe is REUSED from building.ts (buildRoomWalls, buildSign,
// withFloorLift) and props.ts (buildProp) — no geometry is duplicated here.

import * as THREE from 'three/webgpu';
import { kswPalette, palette, radii } from '../../designTokens';
import type { FloorPlan } from '../floorPlan';
import { box, buildProp } from '../props';
import { FLOOR_H, buildRoomWalls, buildSign, withFloorLift } from '../building';

export function buildInterior(plan: FloorPlan): THREE.Group {
  const group = new THREE.Group();
  group.name = 'kswInterior';

  // room floors + accent inlays (building.ts recipe, sans shell/roof/plate)
  for (const room of plan.rooms) {
    const floor = box(room.rect.w, FLOOR_H, room.rect.d, palette.floorWarm, radii.s);
    floor.position.set(room.rect.x, FLOOR_H / 2, room.rect.z);
    group.add(floor);
    const inlay = box(room.rect.w * 0.3, 0.03, room.rect.d * 0.22, room.accent, radii.s);
    inlay.castShadow = false;
    inlay.position.set(room.rect.x, FLOOR_H + 0.015, room.rect.z);
    group.add(inlay);
  }

  // corridor floors
  for (const c of plan.corridors) {
    const floor = box(c.w, FLOOR_H, c.d, kswPalette.corridorFloor, radii.s);
    floor.position.set(c.x, FLOOR_H / 2, c.z);
    group.add(floor);
  }

  // walls + signage + room props
  for (const room of plan.rooms) {
    group.add(buildRoomWalls(plan, room));
    group.add(buildSign(room));
    for (const p of room.props) group.add(withFloorLift(buildProp(p)));
  }
  for (const p of plan.corridorProps) group.add(withFloorLift(buildProp(p)));

  return group;
}
