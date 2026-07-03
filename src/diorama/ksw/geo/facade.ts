// src/diorama/ksw/geo/facade.ts
// Door slot derivation for the instanced city doors (windows.ts). The window
// raster moved into the wall shader (Task 13, cityMassing.ts facadeMaterial),
// so the old facadeLayout window/column math is gone — a door is simply the
// baked door point (from scripts/geo/lib/style.mjs doorForBuilding), lifted to
// half the door height so the box sits on the ground.
import { kswCityStyle } from '../../designTokens';

export type WindowSlot = { x: number; y: number; z: number; yaw: number };

// Return the ground-standing door slot for a building, or null when the bake
// found no door (doorForBuilding requires a nearby road).
export function facadeDoor(b: { door?: { x: number; z: number; yaw: number } }): WindowSlot | null {
  if (!b.door) return null;
  return { x: b.door.x, y: kswCityStyle.doorH / 2, z: b.door.z, yaw: b.door.yaw };
}
