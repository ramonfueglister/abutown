import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import {
  approach,
  CHILD_SCALE,
  createAgentInstances,
  lerpAngle,
  mergedPersonGeometry,
} from '../../src/diorama/ksw/agentMeshes';
import type { PersonRole } from '../../src/diorama/ksw/floorPlan';
import { kswPalette, palette } from '../../src/diorama/designTokens';

const roles: PersonRole[] = ['nurse', 'doctor', 'surgeon', 'patient', 'child', 'visitor', 'labtech', 'paramedic'];

describe('mergedPersonGeometry', () => {
  it('every role merges to a non-empty geometry with position/normal/color', () => {
    for (const role of roles) {
      const geo = mergedPersonGeometry(role);
      expect(geo.attributes.position.count, role).toBeGreaterThan(0);
      expect(geo.attributes.normal.count, role).toBe(geo.attributes.position.count);
      expect(geo.attributes.color.count, role).toBe(geo.attributes.position.count);
    }
  });

  it('bakes only design-token colors into the vertex colors', () => {
    const allTokenColors = new Set<number>([...Object.values(palette), ...Object.values(kswPalette)]);
    const c = new THREE.Color();
    for (const role of roles) {
      const colors = mergedPersonGeometry(role).attributes.color;
      for (let i = 0; i < colors.count; i++) {
        c.setRGB(colors.getX(i), colors.getY(i), colors.getZ(i));
        const hex = c.getHex();
        expect(allTokenColors.has(hex), `${role}: vertex color #${hex.toString(16)} is not a token`).toBe(true);
      }
    }
  });

  it('role accessories add geometry beyond the plain bean', () => {
    const bean = mergedPersonGeometry('visitor').attributes.position.count;
    for (const role of ['nurse', 'doctor', 'surgeon', 'labtech', 'paramedic'] as PersonRole[]) {
      expect(mergedPersonGeometry(role).attributes.position.count, role).toBeGreaterThan(bean);
    }
  });

  it('bakes the child scale into the geometry', () => {
    const adult = mergedPersonGeometry('visitor');
    const child = mergedPersonGeometry('child');
    adult.computeBoundingBox();
    child.computeBoundingBox();
    expect(child.boundingBox!.max.y).toBeCloseTo(adult.boundingBox!.max.y * CHILD_SCALE, 5);
  });

  it('does not mutate cached shared geometries', () => {
    const before = mergedPersonGeometry('doctor').attributes.position.array.slice();
    mergedPersonGeometry('labtech'); // shares cached eye/torus geometries
    const after = mergedPersonGeometry('doctor').attributes.position.array;
    expect(after).toEqual(before);
  });
});

describe('createAgentInstances', () => {
  it('builds one shadow-casting InstancedMesh per role with exact capacity', () => {
    const inst = createAgentInstances({ nurse: 3, child: 1 });
    expect(inst.meshes.length).toBe(2);
    for (const mesh of inst.meshes) {
      expect(mesh.castShadow).toBe(true);
      expect(mesh.receiveShadow).toBe(true);
      expect(mesh.frustumCulled).toBe(false);
    }
    const nurse = inst.meshes.find((m) => m.name === 'ksw-agents-nurse')!;
    expect(nurse.count).toBe(3);
  });

  it('slot writes are accepted and add() enforces capacity and known roles', () => {
    const inst = createAgentInstances({ nurse: 1 });
    const slot = inst.add('nurse', 7);
    slot.set(1.5, -2.5, 0.14, 0.7, true, 0.05);
    inst.update(3); // must not throw with dirty buffers + time advance
    expect(() => inst.add('nurse', 8)).toThrow(/over capacity/);
    expect(() => inst.add('doctor', 0)).toThrow(/no instance bucket/);
    expect(inst.meshes[0].count).toBe(1);
  });
});

describe('CPU mirror math', () => {
  it('approach eases toward the target and clamps at dt*rate >= 1', () => {
    expect(approach(0, 1, 0.05, 10)).toBeCloseTo(0.5);
    expect(approach(0, 1, 1, 10)).toBe(1); // min(1, dt*rate) clamps
    expect(approach(0.14, 0.14, 0.016, 10)).toBeCloseTo(0.14);
  });

  it('lerpAngle takes the shortest arc across the ±π seam', () => {
    // from just below +π to just above -π: must move forward, not spin back
    const a = Math.PI - 0.1;
    const b = -Math.PI + 0.1;
    const mid = lerpAngle(a, b, 0.5);
    expect(mid).toBeCloseTo(Math.PI, 5);
    expect(lerpAngle(0, 1, 0.25)).toBeCloseTo(0.25);
    expect(lerpAngle(1, 1, 0.9)).toBeCloseTo(1);
  });
});
