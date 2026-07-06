// src/diorama/ksw/geo/terrain.ts
// Terrain-tile renderer: turns the height+landcover grids baked into each
// world tile (Task 10's worldData.ts) into indexed grid meshes, one per
// tile of the chosen pyramid level. Vertex colors carry landcover via the
// terrainLook token table; material is the exact vertexTintMat pattern
// nature.ts already uses for the flat green/water areas, imported rather
// than duplicated.
import * as THREE from 'three/webgpu';
import { Fn, attribute, float, positionWorld, texture, vec2 } from 'three/tsl';
import { terrainLook } from '../../designTokens';
import { vertexTintMat } from './nature';
import type { DecodedTile } from './worldData';
import { Landcover } from '../../../proto/world_pb';
import { corridorMaskDataTexture, type CorridorMask } from './corridorMask';

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

/**
 * Terrain material with the corridor-discard (Task 5e, spec §5). Starts from
 * the shared vertexTintMat, then attaches a colorNode that DISCARDS the
 * fragment when the corridor mask reads "inside a road/rail corridor" at the
 * fragment's world (x,z). Ribbon side-skirts (roads.ts) close the resulting
 * hole, so terrain piercing a road surface is impossible by construction.
 *
 * The discard runs INSIDE an `Fn(() => …)()` so `.discard()` appends to the
 * fragment stack — a bare top-level `Discard()` attaches to no stack and is
 * SILENTLY DROPPED (project memory: the KSW dollhouse cutaway hit this exact
 * trap; cityMassing.ts uses the same Fn-wrapped guard). We threshold the
 * NEAREST-sampled mask at 0.5 — texels are 0 or 1 so the band edge is crisp.
 *
 * The mask is world-space and level-independent, so every pyramid level shares
 * this one material and discards the same footprint — no LOD popping.
 */
function terrainDiscardMat(mask: CorridorMask): THREE.MeshPhysicalMaterial {
  const m = vertexTintMat(terrainLook.meadow);
  const tex = corridorMaskDataTexture(mask);
  // uv from world (x,z): (worldX − originX) / (cols · cellSize), same for z.
  // The texture has flipY=false so texel (i,j) = cell (i,j); v uses z directly.
  const spanX = float(mask.cols * mask.cellSizeM);
  const spanZ = float(mask.rows * mask.cellSizeM);
  // vertexTintMat sets vertexColors=true + white base; the per-vertex landcover
  // tint lives in the 'color' attribute. Drive the fragment color from it so
  // the discard material keeps the exact terrain look outside the corridor.
  const tint = attribute<'vec3'>('color', 'vec3');
  m.colorNode = Fn(() => {
    const u = positionWorld.x.sub(float(mask.originX)).div(spanX);
    const v = positionWorld.z.sub(float(mask.originZ)).div(spanZ);
    const inside = texture(tex, vec2(u, v)).r.greaterThan(float(0.5));
    inside.discard();
    return tint;
  })() as unknown as THREE.MeshPhysicalMaterial['colorNode'];
  return m;
}

export function buildTerrainTiles(
  tiles: DecodedTile[],
  opts: { level: number; corridorMask?: CorridorMask },
): THREE.Group {
  const group = new THREE.Group();
  group.name = 'terrainTiles';

  // With a corridor mask, terrain fragments inside road/rail corridors are
  // discarded (spec §5 terrain-discard). Without one (tests, callers pre-Task
  // 5e), the plain shared tint material renders every fragment.
  const mat = opts.corridorMask ? terrainDiscardMat(opts.corridorMask) : vertexTintMat(terrainLook.meadow);

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
