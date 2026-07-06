// tests/geo/terrain.test.ts
import { describe, expect, it } from 'vitest';
import * as THREE from 'three/webgpu';
import { buildTerrainTileMesh, buildL0Backdrop, buildTileSubCellTerrain } from '../../src/diorama/ksw/geo/terrain';
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

describe('buildTerrainTileMesh', () => {
  const mesh = buildTerrainTileMesh(makeTile());

  it('builds a gridN×gridN indexed grid mesh', () => {
    expect(mesh.geometry.attributes.position.count).toBe(9);
    expect(mesh.geometry.index!.count).toBe((3 - 1) * (3 - 1) * 6);
  });

  it('sets vertex y from the height array', () => {
    const pos = mesh.geometry.attributes.position;
    for (let n = 0; n < 9; n++) {
      expect(pos.getY(n)).toBeCloseTo(n);
    }
  });

  it('tints the forest vertex with the forest token color', () => {
    const color = mesh.geometry.attributes.color;
    const forest = new THREE.Color(terrainLook.forest);
    expect(color.getX(4)).toBeCloseTo(forest.r, 5);
    expect(color.getY(4)).toBeCloseTo(forest.g, 5);
    expect(color.getZ(4)).toBeCloseTo(forest.b, 5);
  });

  it('names the mesh and sets shadow flags', () => {
    expect(mesh.name).toBe('terrainL2/5_7');
    expect(mesh.receiveShadow).toBe(true);
    expect(mesh.castShadow).toBe(false);
  });
});

describe('buildL0Backdrop', () => {
  // The synthetic tile spans 2×2 m; split into 2×2 backdrop cells of 1 m.
  const { group, meshes } = buildL0Backdrop(makeTile(), 2);

  it('produces cellsPerSide² sub-meshes keyed "x_y" in cell-index space', () => {
    expect(group.children.length).toBe(4);
    expect([...meshes.keys()].sort()).toEqual(['0_0', '0_1', '1_0', '1_1']);
    expect(meshes.get('1_0')!.name).toBe('terrainL0Backdrop/1_0');
  });

  it('resamples heights ON the bilinear L0 surface (exact at source vertices)', () => {
    // Cell (0,0) covers x/z ∈ [0,1]; its corner vertices coincide with the
    // source grid vertices (0,0)=0, (1,0)=1, (0,1)=3, (1,1)=4.
    const pos = meshes.get('0_0')!.geometry.attributes.position;
    const at = new Map<string, number>();
    for (let n = 0; n < pos.count; n++) {
      at.set(`${pos.getX(n)}_${pos.getZ(n)}`, pos.getY(n));
    }
    expect(at.get('0_0')).toBeCloseTo(0);
    expect(at.get('1_0')).toBeCloseTo(1);
    expect(at.get('0_1')).toBeCloseTo(3);
    expect(at.get('1_1')).toBeCloseTo(4);
  });

  it('seams are watertight: adjacent cells share identical edge heights', () => {
    const east = meshes.get('0_0')!.geometry.attributes.position;
    const west = meshes.get('1_0')!.geometry.attributes.position;
    const edge = (pos: THREE.BufferAttribute | THREE.InterleavedBufferAttribute, x: number): Map<number, number> => {
      const m = new Map<number, number>();
      for (let n = 0; n < pos.count; n++) {
        if (Math.abs(pos.getX(n) - x) < 1e-9) m.set(pos.getZ(n), pos.getY(n));
      }
      return m;
    };
    const a = edge(east, 1);
    const b = edge(west, 1);
    expect(a.size).toBeGreaterThan(0);
    expect(a.size).toBe(b.size);
    for (const [z, y] of a) {
      expect(b.get(z)).toBeCloseTo(y, 9);
    }
  });

  it('sets shadow flags on every cell', () => {
    for (const mesh of meshes.values()) {
      expect(mesh.receiveShadow).toBe(true);
      expect(mesh.castShadow).toBe(false);
    }
  });
});

describe('buildTileSubCellTerrain (#141 per-sub-cell L1 hide)', () => {
  // L1-shaped synthetic tile: gridN=5, cellSize=1 → extent 4 m, 4×4 sub-cells
  // of 1 m each; sub-cell borders fall on source vertices here, and the
  // resample step is cellSize/2 (nSub = 3 per cell).
  function makeL1(): DecodedTile {
    const gridN = 5;
    const height = Array.from({ length: gridN * gridN }, (_, n) => n * 0.5);
    const landcover = new Array(gridN * gridN).fill(Landcover.MEADOW);
    return {
      level: 1,
      x: 2,
      y: 3,
      tile: { gridN, cellSize: 1, originX: 0, originZ: 0, height, landcover } as unknown as DecodedTile['tile'],
    };
  }
  const { group, meshes } = buildTileSubCellTerrain(makeL1(), 4);

  it('produces 16 sub-meshes keyed "i_j" and named with the tile key', () => {
    expect(group.children.length).toBe(16);
    expect(meshes.size).toBe(16);
    expect(meshes.get('1_2')!.name).toBe('tileTerrain/L1/2_3#1_2');
    expect(group.name).toBe('tileTerrainCells/L1/2_3');
  });

  it('resample is exact at source vertices (bilinear identity)', () => {
    // Sub-cell (0,0) covers x/z ∈ [0,1]; source heights: h(i,j) = (j*5+i)*0.5.
    const pos = meshes.get('0_0')!.geometry.attributes.position;
    const at = new Map<string, number>();
    for (let n = 0; n < pos.count; n++) at.set(`${pos.getX(n)}_${pos.getZ(n)}`, pos.getY(n));
    expect(at.get('0_0')).toBeCloseTo(0);
    expect(at.get('1_0')).toBeCloseTo(0.5);
    expect(at.get('0_1')).toBeCloseTo(2.5);
    expect(at.get('1_1')).toBeCloseTo(3);
    // Mid-cell point lies on the bilinear surface: (0.5, 0.5) → mean of corners.
    expect(at.get('0.5_0.5')).toBeCloseTo((0 + 0.5 + 2.5 + 3) / 4);
  });

  it('seams are watertight between adjacent sub-cells', () => {
    const edge = (m: THREE.Mesh, x: number): Map<number, number> => {
      const pos = m.geometry.attributes.position;
      const out = new Map<number, number>();
      for (let n = 0; n < pos.count; n++) {
        if (Math.abs(pos.getX(n) - x) < 1e-9) out.set(pos.getZ(n), pos.getY(n));
      }
      return out;
    };
    const a = edge(meshes.get('0_0')!, 1);
    const b = edge(meshes.get('1_0')!, 1);
    expect(a.size).toBeGreaterThan(0);
    expect(a.size).toBe(b.size);
    for (const [z, y] of a) expect(b.get(z)).toBeCloseTo(y, 9);
  });
});
