// Shared TSL clay sheen recipe: sheenColor = diffuse color lerped
// clay.sheenLerp toward palette.trueWhite, scaled by clay.sheen. This is the
// node-material twin of props.clayMat's CPU recipe — one source for both the
// static batches (per-instance batch color) and the agent instances
// (per-vertex color).

import * as THREE from 'three/webgpu';
import { float, mix, vec3 } from 'three/tsl';
import { clay, palette } from '../designTokens';

type Vec3Node = ReturnType<typeof vec3>;

const white = new THREE.Color(palette.trueWhite);

export function claySheenNode(colorNode: unknown): Vec3Node {
  return mix(colorNode as Vec3Node, vec3(white.r, white.g, white.b), float(clay.sheenLerp)).mul(
    float(clay.sheen),
  ) as unknown as Vec3Node;
}
