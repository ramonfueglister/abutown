// tests/geo/terrain.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildTerrainTiles } from '../../src/diorama/ksw/geo/terrain';
import type { DecodedTile } from '../../src/diorama/ksw/geo/worldData';
import { terrainLook } from '../../src/diorama/designTokens';
import { Landcover } from '../../src/proto/world_pb';

// synthetic 3×3 tile: gridN=3, cellSize=1, origin at (0,0). Landcover mostly
// meadow, one forest vertex at grid index (1,1) -> j*gridN+i = 4.
function makeTile(): DecodedTile {
  const gridN = 3;
  const height = [0, 1, 2, 3, 4, 5, 6, 7, 8]; // distinct so we can assert y directly
  const landcover = new Array(gridN * gridN).fill(Landcover.MEADOW);
  landcover[4] = Landcover.FOREST; // center vertex (i=1, j=1)
  return {
    level: 2,
    x: 5,
    y: 7,
    tile: {
      gridN,
      cellSize: 1,
      originX: 0,
      originZ: 0,
      height,
      landcover,
    } as unknown as DecodedTile['tile'],
  };
}

describe('buildTerrainTiles', () => {
  const tiles = [makeTile()];
  const group = buildTerrainTiles(tiles, { level: 2 });

  it('produces one mesh per tile of the chosen level', () => {
    expect(group.children.length).toBe(1);
  });

  it('builds a gridN×gridN indexed grid mesh', () => {
    const mesh = group.children[0] as THREE.Mesh;
    expect(mesh.geometry.attributes.position.count).toBe(9);
    expect(mesh.geometry.index!.count).toBe((3 - 1) * (3 - 1) * 6);
  });

  it('sets vertex y from the height array', () => {
    const mesh = group.children[0] as THREE.Mesh;
    const pos = mesh.geometry.attributes.position;
    for (let n = 0; n < 9; n++) {
      expect(pos.getY(n)).toBeCloseTo(n);
    }
  });

  it('tints the forest vertex with the forest token color', () => {
    const mesh = group.children[0] as THREE.Mesh;
    const color = mesh.geometry.attributes.color;
    const forest = new THREE.Color(terrainLook.forest);
    expect(color.getX(4)).toBeCloseTo(forest.r, 5);
    expect(color.getY(4)).toBeCloseTo(forest.g, 5);
    expect(color.getZ(4)).toBeCloseTo(forest.b, 5);
  });

  it('names the mesh, sets shadow flags, and skips tiles of other levels', () => {
    const mesh = group.children[0] as THREE.Mesh;
    expect(mesh.name).toBe('terrainL2/5_7');
    expect(mesh.receiveShadow).toBe(true);
    expect(mesh.castShadow).toBe(false);

    const otherLevelTiles = [{ ...makeTile(), level: 3 }];
    const emptyGroup = buildTerrainTiles(otherLevelTiles, { level: 2 });
    expect(emptyGroup.children.length).toBe(0);
  });
});
