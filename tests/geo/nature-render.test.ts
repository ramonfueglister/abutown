// tests/geo/nature-render.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildNature } from '../../src/diorama/ksw/geo/nature';
import type { CityNature } from '../../src/diorama/ksw/geo/geoData';

const nature: CityNature = {
  greens: [
    { kind: 'park', ring: [[0, 0], [40, 0], [40, 30], [0, 30]] },
    { kind: 'wood', ring: [[100, 0], [160, 0], [160, 50], [100, 50]] },
  ],
  waterAreas: [{ ring: [[-50, 0], [-20, 0], [-20, 20], [-50, 20]] }],
  rivers: [{ width: 6, pts: [[200, 0], [260, 0]] }],
  // Trees are handled by the archetype tree layer (buildTreeLayer), not by
  // buildNature — this input is ignored here.
  trees: [
    { x: 10, z: 10, h: 9, r: 3, kind: 'broad' },
    { x: 20, z: 15, h: 9, r: 3, kind: 'broad' },
    { x: 500, z: 500, h: 9, r: 3, kind: 'broad' },
  ],
};

describe('buildNature', () => {
  const group = buildNature(nature, {});
  const greens = group.getObjectByName('natureGreens') as THREE.Mesh;
  const water = group.getObjectByName('natureWater') as THREE.Mesh;

  it('triangulates green and water areas into single meshes', () => {
    expect(greens.geometry.index!.count).toBe(12); // 2 rects × 2 tris
    // water: 1 area rect (2 tris) + 1 river segment quad (2 tris)
    expect(water.geometry.index!.count).toBe(12);
  });

  it('produces no tree meshes (trees live in the archetype tree layer)', () => {
    for (const name of ['treeCanopies', 'treeConifers', 'treeTrunks', 'treeImpostors', 'treeImpostorsConifer']) {
      expect(group.getObjectByName(name)).toBeUndefined();
    }
  });

  it('greens sit below road level and receive shadows', () => {
    const pos = greens.geometry.getAttribute('position');
    expect(pos.getY(0)).toBeLessThan(0.04);
    expect(greens.receiveShadow).toBe(true);
    expect(greens.castShadow).toBe(false);
  });
});
