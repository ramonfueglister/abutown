// src/diorama/environment/precipitation.ts — no-op until the precipitation task fills it in.
import { Group, type Object3D } from 'three/webgpu';
import type { PrecipType } from './environment';

export type PrecipitationSystem = {
  update(type: PrecipType, intensity: number, windSpeedMs: number, windDirRad: number, dtSeconds: number): void;
  object3d: Object3D;
};

export function createPrecipitation(): PrecipitationSystem {
  return { update: () => {}, object3d: new Group() };
}
