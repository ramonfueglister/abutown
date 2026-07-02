// tests/geo/roads.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildRoads } from '../../src/diorama/ksw/geo/roads';

describe('buildRoads', () => {
  const group = buildRoads(
    [{ class: 'residential', width: 6, pts: [[0, 0], [10, 0], [10, 10]] }],
    [{ class: 'rail', width: 3, pts: [[0, 5], [20, 5]] }],
  );
  const roads = group.getObjectByName('roadRibbons') as THREE.Mesh;
  const rails = group.getObjectByName('railRibbons') as THREE.Mesh;

  it('builds one ribbon quad per segment', () => {
    // road: 2 segments × 4 verts, rail: 1 segment × 4 verts
    expect(roads.geometry.getAttribute('position').count).toBe(8);
    expect(rails.geometry.getAttribute('position').count).toBe(4);
  });
  it('ribbon width matches the class width', () => {
    const pos = roads.geometry.getAttribute('position');
    // first segment runs +x, so its first two verts differ by `width` in z
    expect(Math.abs(pos.getZ(0) - pos.getZ(1))).toBeCloseTo(6);
  });
  it('roads receive but never cast shadows', () => {
    expect(roads.receiveShadow).toBe(true);
    expect(roads.castShadow).toBe(false);
  });
});
