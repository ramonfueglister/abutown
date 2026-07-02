// Normative reference for the person-instance transform composition used by
// the agent TSL vertex stage (agentMeshes.ts, makeRoleMesh positionNode).
// The TSL cannot execute under vitest, so this pure function pins the
// intended convention; tests/diorama/personTransformRef.test.ts asserts it
// matches THREE.Object3D composition (the pre-Slice-C per-agent Object3Ds:
// scale.y = squash, rotation.z = roll, rotation.y = yaw with the default
// 'XYZ' Euler order — Z applied before Y — position = (x, yLift, z)).
//
// Composition order (right to left, i.e. applied to the vertex first-to-last):
//   M = T(x, yLift, z) * R_y(yaw) * R_z(roll) * S(1, squash, 1)
//
// Residual risk: this documents the intent; the TSL in agentMeshes.ts still
// has to transcribe the same math by hand (scalar-expanded). Any change there
// must be mirrored here and vice versa.

import * as THREE from 'three/webgpu';

export function personInstanceMatrix(
  x: number,
  yLift: number,
  z: number,
  yaw: number,
  roll: number,
  squash: number,
): THREE.Matrix4 {
  const m = new THREE.Matrix4().makeTranslation(x, yLift, z);
  m.multiply(new THREE.Matrix4().makeRotationY(yaw));
  m.multiply(new THREE.Matrix4().makeRotationZ(roll));
  m.multiply(new THREE.Matrix4().makeScale(1, squash, 1));
  return m;
}
