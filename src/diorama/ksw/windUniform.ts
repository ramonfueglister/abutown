// Shared wind uniforms driving tree sway (pattern: glowUniform.ts — one tiny
// module so builders and main.ts import without cycles). Set per env update
// in applyCityEnvironment from the live Open-Meteo wind.
import * as THREE from 'three/webgpu';
import { uniform } from 'three/tsl';

export const windAmpU = uniform(0);
export const windDirU = uniform(new THREE.Vector2(1, 0));

// 0 at calm; ~0.5 at a 5 m/s breeze; saturates at 1.2 for storms.
export function windAmplitude(windSpeedMs: number): number {
  return Math.min(1.2, windSpeedMs / 10);
}
