// F9: the person-instance transform composition (agentMeshes.ts TSL) pinned
// against THREE.Object3D — the pre-Slice-C per-agent Object3Ds composed
// local = T * R(yaw) * R(roll) * S(squash) via scale.y / rotation.z /
// rotation.y (default 'XYZ' Euler order applies Z before Y) / position.
// personInstanceMatrix is the executable reference the TSL comment points at.

import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { personInstanceMatrix } from '../../src/diorama/ksw/personTransformRef';

const samples: Array<{ x: number; yLift: number; z: number; yaw: number; roll: number; squash: number }> = [
  { x: 0, yLift: 0, z: 0, yaw: 0, roll: 0, squash: 1 },
  { x: 3.2, yLift: 0.14, z: -7.5, yaw: 0.8, roll: 0.05, squash: 1.025 },
  { x: -12.4, yLift: 0, z: 22.1, yaw: -2.4, roll: -0.05, squash: 0.988 },
  { x: 100, yLift: 0.14, z: -50, yaw: Math.PI - 0.1, roll: 0.03, squash: 1.012 },
  { x: -0.5, yLift: 0.07, z: 0.5, yaw: -Math.PI / 2, roll: -0.02, squash: 0.975 },
];

describe('personInstanceMatrix', () => {
  it('matches THREE.Object3D composition (scale.y, rotation.z, rotation.y, position)', () => {
    for (const s of samples) {
      const obj = new THREE.Object3D();
      expect(obj.rotation.order).toBe('XYZ'); // the convention the reference assumes
      obj.scale.y = s.squash;
      obj.rotation.z = s.roll;
      obj.rotation.y = s.yaw;
      obj.position.set(s.x, s.yLift, s.z);
      obj.updateMatrix();

      const ref = personInstanceMatrix(s.x, s.yLift, s.z, s.yaw, s.roll, s.squash);
      for (let i = 0; i < 16; i++) {
        expect(ref.elements[i], `sample ${JSON.stringify(s)} element ${i}`).toBeCloseTo(obj.matrix.elements[i], 10);
      }
    }
  });

  it('transforms a probe vertex identically to Object3D.localToWorld', () => {
    for (const s of samples) {
      const obj = new THREE.Object3D();
      obj.scale.y = s.squash;
      obj.rotation.z = s.roll;
      obj.rotation.y = s.yaw;
      obj.position.set(s.x, s.yLift, s.z);
      obj.updateMatrixWorld(true);

      const v = new THREE.Vector3(0.34, 0.92, 0.305); // an eye-ish LOD0 vertex
      const viaObj = obj.localToWorld(v.clone());
      const viaRef = v.clone().applyMatrix4(personInstanceMatrix(s.x, s.yLift, s.z, s.yaw, s.roll, s.squash));
      expect(viaRef.x).toBeCloseTo(viaObj.x, 10);
      expect(viaRef.y).toBeCloseTo(viaObj.y, 10);
      expect(viaRef.z).toBeCloseTo(viaObj.z, 10);
    }
  });
});
