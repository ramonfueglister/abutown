import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildPerson, buildProp, propBuilders } from '../../src/diorama/ksw/props';
import { kswPlan, type PersonRole } from '../../src/diorama/ksw/floorPlan';
import { kswPalette, palette } from '../../src/diorama/designTokens';

const allTokenColors = new Set<number>([...Object.values(palette), ...Object.values(kswPalette)]);

function meshes(g: THREE.Object3D): THREE.Mesh[] {
  const out: THREE.Mesh[] = [];
  g.traverse((o) => {
    if ((o as THREE.Mesh).isMesh) out.push(o as THREE.Mesh);
  });
  return out;
}

describe('prop registry', () => {
  it('covers every prop kind referenced by the floor plan', () => {
    const used = new Set<string>();
    for (const room of kswPlan.rooms) for (const p of room.props) used.add(p.kind);
    for (const p of kswPlan.outdoorProps) used.add(p.kind);
    const missing = [...used].filter((k) => !(k in propBuilders));
    expect(missing, `missing builders: ${missing.join(', ')}`).toEqual([]);
  });

  it('every builder yields a non-empty, shadow-casting group', () => {
    for (const [kind, build] of Object.entries(propBuilders)) {
      const g = build();
      const ms = meshes(g);
      expect(ms.length, kind).toBeGreaterThan(0);
      expect(
        ms.some((m) => m.castShadow),
        `${kind} casts no shadow at all`,
      ).toBe(true);
    }
  });

  it('uses only design-token colors (no hex values outside designTokens)', () => {
    for (const [kind, build] of Object.entries(propBuilders)) {
      for (const m of meshes(build())) {
        const mat = m.material as THREE.MeshStandardMaterial;
        const hex = mat.color.getHex();
        expect(allTokenColors.has(hex), `${kind}: color #${hex.toString(16)} is not a token`).toBe(true);
      }
    }
  });

  it('buildProp applies placement (position, rotation, scale)', () => {
    const g = buildProp({ kind: 'plant', x: 3, z: -4, rotY: 0.5, scale: 1.2 });
    expect(g.position.x).toBe(3);
    expect(g.position.z).toBe(-4);
    expect(g.rotation.y).toBeCloseTo(0.5);
    expect(g.scale.x).toBeCloseTo(1.2);
  });

  it('buildProp throws on unknown kinds', () => {
    expect(() => buildProp({ kind: 'nope', x: 0, z: 0 })).toThrow(/unknown prop kind/);
  });
});

describe('people', () => {
  const roles: PersonRole[] = ['nurse', 'doctor', 'surgeon', 'patient', 'child', 'visitor', 'labtech', 'paramedic'];

  it('every role builds with token colors and faces its yaw', () => {
    for (const role of roles) {
      const g = buildPerson({ role, x: 1, z: 2, yaw: 0.7 });
      expect(meshes(g).length, role).toBeGreaterThan(0);
      expect(g.rotation.y).toBeCloseTo(0.7);
      for (const m of meshes(g)) {
        const hex = (m.material as THREE.MeshStandardMaterial).color.getHex();
        expect(allTokenColors.has(hex), `${role}: color #${hex.toString(16)}`).toBe(true);
      }
    }
  });

  it('every person placed in the plan has a known role', () => {
    const known = new Set(roles);
    for (const room of kswPlan.rooms) for (const p of room.people) expect(known.has(p.role), p.role).toBe(true);
    for (const p of kswPlan.outdoorPeople) expect(known.has(p.role), p.role).toBe(true);
  });
});
