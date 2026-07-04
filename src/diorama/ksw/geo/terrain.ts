// src/diorama/ksw/geo/terrain.ts
// Terrain-tile renderer: turns the height+landcover grids baked into each
// world tile (Task 10's worldData.ts) into indexed grid meshes, one per
// tile of the chosen pyramid level. Vertex colors carry landcover via the
// terrainLook token table; material is the exact vertexTintMat pattern
// nature.ts already uses for the flat green/water areas, imported rather
// than duplicated.
import * as THREE from 'three/webgpu';
import { terrainLook } from '../../designTokens';
import { vertexTintMat } from './nature';
import type { DecodedTile } from './worldData';
import { Landcover } from '../../../proto/world_pb';

const landcoverColor: Record<number, number> = {
  [Landcover.MEADOW]: terrainLook.meadow,
  [Landcover.FOREST]: terrainLook.forest,
  [Landcover.FARMLAND]: terrainLook.farmland,
  [Landcover.RESIDENTIAL]: terrainLook.residentialLu,
  [Landcover.INDUSTRIAL]: terrainLook.industrialLu,
  [Landcover.WATER]: terrainLook.water,
  [Landcover.ROCK]: terrainLook.rock,
};

function buildTileGeometry(tile: DecodedTile['tile']): THREE.BufferGeometry {
  const { gridN, cellSize, originX, originZ, height, landcover } = tile;

  const positions = new Float32Array(gridN * gridN * 3);
  const colors = new Float32Array(gridN * gridN * 3);
  const color = new THREE.Color();

  for (let j = 0; j < gridN; j++) {
    for (let i = 0; i < gridN; i++) {
      const n = j * gridN + i;
      positions[n * 3 + 0] = originX + i * cellSize;
      positions[n * 3 + 1] = height[n];
      positions[n * 3 + 2] = originZ + j * cellSize;

      const lc = landcover[n];
      color.set(landcoverColor[lc] ?? terrainLook.meadow);
      colors[n * 3 + 0] = color.r;
      colors[n * 3 + 1] = color.g;
      colors[n * 3 + 2] = color.b;
    }
  }

  const indices: number[] = [];
  for (let j = 0; j < gridN - 1; j++) {
    for (let i = 0; i < gridN - 1; i++) {
      const a = j * gridN + i;
      const b = j * gridN + (i + 1);
      const c = (j + 1) * gridN + i;
      const d = (j + 1) * gridN + (i + 1);
      // two triangles per cell, row-major
      indices.push(a, c, b);
      indices.push(b, c, d);
    }
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.setIndex(
    positions.length / 3 > 65535
      ? new THREE.BufferAttribute(new Uint32Array(indices), 1)
      : new THREE.BufferAttribute(new Uint16Array(indices), 1),
  );
  geo.computeVertexNormals();
  return geo;
}

export function buildTerrainTiles(tiles: DecodedTile[], opts: { level: number }): THREE.Group {
  const group = new THREE.Group();
  group.name = 'terrainTiles';

  const mat = vertexTintMat(terrainLook.meadow);

  for (const { level, x, y, tile } of tiles) {
    if (level !== opts.level) continue;

    const geo = buildTileGeometry(tile);
    const mesh = new THREE.Mesh(geo, mat);
    mesh.name = `terrainL${level}/${x}_${y}`;
    mesh.receiveShadow = true;
    mesh.castShadow = false;
    group.add(mesh);
  }

  return group;
}
