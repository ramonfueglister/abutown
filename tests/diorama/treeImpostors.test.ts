import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { OCT_GRID, atlasLayout, hemiOctUv, viewDirFor } from '../../src/diorama/ksw/geo/treeImpostors';

describe('hemi-octahedral mapping', () => {
  it('round-trips every grid cell', () => {
    for (let iy = 0; iy < OCT_GRID; iy++) {
      for (let ix = 0; ix < OCT_GRID; ix++) {
        const dir = viewDirFor(ix, iy);
        expect(dir.y).toBeGreaterThanOrEqual(-1e-6); // upper hemisphere
        expect(dir.length()).toBeCloseTo(1, 5);
        const { u, v } = hemiOctUv(dir);
        expect(Math.round(u)).toBe(ix);
        expect(Math.round(v)).toBe(iy);
      }
    }
  });
  it('maps the horizon ring to the grid border and zenith to center', () => {
    const zen = hemiOctUv(new THREE.Vector3(0, 1, 0));
    expect(zen.u).toBeCloseTo((OCT_GRID - 1) / 2, 1);
    expect(zen.v).toBeCloseTo((OCT_GRID - 1) / 2, 1);
  });
});

describe('atlasLayout', () => {
  it('fits archCount × OCT_GRID² cells in a near-square power-of-two atlas', () => {
    const l = atlasLayout(20);
    expect(l.cols * l.rows).toBeGreaterThanOrEqual(20 * OCT_GRID * OCT_GRID);
    expect(Math.log2(l.width) % 1).toBe(0);
    expect(Math.log2(l.height) % 1).toBe(0);
  });
});
