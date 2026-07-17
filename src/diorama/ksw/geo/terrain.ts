// src/diorama/ksw/geo/terrain.ts
// Terrain-tile renderer: turns the height+landcover grids baked into each
// world tile (Task 10's worldData.ts) into indexed grid meshes, one per
// tile of the chosen pyramid level. Vertex colors carry landcover via the
// terrainLook token table; material is the exact vertexTintMat pattern
// nature.ts already uses for the flat green/water areas, imported rather
// than duplicated.
import * as THREE from 'three/webgpu';
import { Fn, attribute, float, positionWorld, texture, uniform, vec2 } from 'three/tsl';
import { terrainLook } from '../../designTokens';
import { groundTintMat, terrainDetailTint, vertexTintMat } from './nature';
import type { DecodedTile } from './worldData';
import { Landcover } from '../../../proto/world_pb';
import { corridorMaskDataTexture, type CorridorMask } from './corridorMask';
import { insideDiscardRegion } from './corridorDiscardRegion';

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
 * The distance-limited corridor discard (#144) lives in corridorDiscardRegion:
 * Level-gating (only L2 tiles get the discard material) removes the coarse-LOD
 * divergence, but at the fine ring's OUTER rim the discard holes of edge L2
 * tiles look through to the offset L1 surfaces behind them — a dashed bright
 * seam along the ring boundary. Keeping the discard radius comfortably INSIDE
 * the guaranteed-fine region (main.ts passes 0.8 × the streamer's r2) means
 * the discard→no-discard transition happens over pure L2 terrain, where the
 * corridor-snapped surface matches the platform within centimetres — no gap
 * for a ray to slip through. Beyond it, terrain simply covers the corridors.
 *
 * The skirts that close these holes read the SAME region (roads.ts skirtMat),
 * so a hole and its wall always begin and end together.
 */

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
 * The mask is world-space, but the discard applies ONLY to the finest (L2)
 * terrain (#144): the road platform is built against fine heights; coarser
 * levels deviate metres from them, and discarding those fragments opens
 * corridor slots the platform cannot close (far-field white-dash storm).
 * Callers gate the mask by level (tileContent.ts, main.ts L0 backdrop).
 */
function terrainDiscardMat(mask: CorridorMask): THREE.MeshPhysicalMaterial {
  const m = vertexTintMat(terrainLook.meadow);
  // colorNode reads the tint attribute itself — vertexColors on would
  // multiply it in twice (tint², subtly darkened terrain). 2026-07-07.
  m.vertexColors = false;
  const tex = corridorMaskDataTexture(mask);
  // uv from world (x,z): (worldX − originX + cell/2) / (cols · cellSize), same
  // for z. The +cell/2 makes the NEAREST texel floor(u·cols) equal the bake's
  // round-to-nearest cell — without it the discard footprint shifts +half a
  // cell in +x/+z beyond the platform coverage (#144 see-through strips).
  // MIRROR: corridorMask.ts `maskShaderUv` is the unit-tested JS twin of this
  // formula; change both together. flipY=false so texel (i,j) = cell (i,j).
  const halfCell = float(mask.cellSizeM / 2);
  const spanX = float(mask.cols * mask.cellSizeM);
  const spanZ = float(mask.rows * mask.cellSizeM);
  // vertexTintMat sets vertexColors=true + white base; the per-vertex landcover
  // tint lives in the 'color' attribute. Drive the fragment color from it so
  // the discard material keeps the exact terrain look outside the corridor.
  const tint = attribute<'vec3'>('color', 'vec3');
  m.colorNode = Fn(() => {
    const u = positionWorld.x.sub(float(mask.originX)).add(halfCell).div(spanX);
    const v = positionWorld.z.sub(float(mask.originZ)).add(halfCell).div(spanZ);
    const inside = texture(tex, vec2(u, v)).r.greaterThan(float(0.5));
    // Distance limit (#144): only discard well inside the fine ring — see
    // corridorDiscardRegion.
    inside.and(insideDiscardRegion()).discard();
    // tint² on purpose — see groundTintMat (nature.ts): the approved ground
    // look was curated under the accidental double-multiply.
    return terrainDetailTint(tint.mul(tint));
  })() as unknown as THREE.MeshPhysicalMaterial['colorNode'];
  return m;
}

// Shared terrain material across all LOD levels/tiles — vertex colours carry
// per-vertex landcover tint (plus the world-space detail mottling), so one
// material suffices.
const terrainMat = groundTintMat(terrainLook.meadow);

// Memoized discard material per corridor mask: streamed tiles materialize one
// at a time over many frames, so the material must be shared/cached rather
// than rebuilt per tile (one DataTexture + one shader program per mask). The
// WeakMap keys on the decoded mask object the boot path loads exactly once.
const discardMatCache = new WeakMap<CorridorMask, THREE.MeshPhysicalMaterial>();
function terrainMaterialFor(mask?: CorridorMask): THREE.MeshPhysicalMaterial {
  if (!mask) return terrainMat;
  let m = discardMatCache.get(mask);
  if (!m) {
    m = terrainDiscardMat(mask);
    discardMatCache.set(mask, m);
  }
  return m;
}

/**
 * Builds the terrain grid mesh for a single decoded tile. Extracted from
 * the pre-M3 all-tiles boot loop (Task 4) so `tileContent.ts`'s per-tile
 * materialization can reuse the exact same geometry-building logic.
 *
 * With a corridor mask (spec §5 terrain-discard), fragments inside road/rail
 * corridors are discarded. Streamed FINE (L2) tiles MUST pass the same mask
 * the boot path uses, or roads would sink into un-discarded streamed terrain;
 * coarser levels must NOT pass one (#144, see terrainDiscardMat).
 *
 * Mesh naming is load-bearing: LOD code elsewhere resolves terrain meshes by
 * `terrainL${level}/${x}_${y}` via `getObjectByName`. Do not change this
 * convention without updating those call sites.
 */
export function buildTerrainTileMesh(dec: DecodedTile, opts?: { corridorMask?: CorridorMask }): THREE.Mesh {
  const { level, x, y, tile } = dec;
  const geo = buildTileGeometry(tile);
  const mesh = new THREE.Mesh(geo, terrainMaterialFor(opts?.corridorMask));
  mesh.name = `terrainL${level}/${x}_${y}`;
  mesh.receiveShadow = true;
  mesh.castShadow = false;
  return mesh;
}

/**
 * Shared resample core for the per-cell terrain splits (L0 backdrop and the
 * #141 L1 sub-cells): one tile's height field cut into a `cellsPerSide` grid
 * of independent sub-meshes, each an `nSub`×`nSub` vertex grid bilinearly
 * resampled from the source height field. Every resampled vertex lies exactly
 * ON the source's bilinear surface, so seams between adjacent cells — and
 * between adjacent tiles that are resampled with the same scheme — are
 * watertight; landcover tint is nearest-vertex.
 */
function buildResampledCellMeshes(
  dec: DecodedTile,
  cellsPerSide: number,
  nSub: number,
  mat: THREE.Material,
  nameFor: (cx: number, cy: number) => string,
): { group: THREE.Group; meshes: Map<string, THREE.Mesh> } {
  const { gridN, cellSize, originX, originZ, height, landcover } = dec.tile;
  const extent = (gridN - 1) * cellSize;
  const cellExtent = extent / cellsPerSide;

  const heightAt = (x: number, z: number): number => {
    const fx = Math.min(gridN - 1, Math.max(0, (x - originX) / cellSize));
    const fz = Math.min(gridN - 1, Math.max(0, (z - originZ) / cellSize));
    const i0 = Math.min(Math.floor(fx), gridN - 2);
    const j0 = Math.min(Math.floor(fz), gridN - 2);
    const tx = fx - i0;
    const tz = fz - j0;
    const h00 = height[j0 * gridN + i0];
    const h10 = height[j0 * gridN + i0 + 1];
    const h01 = height[(j0 + 1) * gridN + i0];
    const h11 = height[(j0 + 1) * gridN + i0 + 1];
    return (h00 + (h10 - h00) * tx) * (1 - tz) + (h01 + (h11 - h01) * tx) * tz;
  };
  const landcoverAt = (x: number, z: number): number => {
    const i = Math.min(gridN - 1, Math.max(0, Math.round((x - originX) / cellSize)));
    const j = Math.min(gridN - 1, Math.max(0, Math.round((z - originZ) / cellSize)));
    return landcover[j * gridN + i];
  };

  const group = new THREE.Group();
  const meshes = new Map<string, THREE.Mesh>();
  const color = new THREE.Color();

  for (let cy = 0; cy < cellsPerSide; cy++) {
    for (let cx = 0; cx < cellsPerSide; cx++) {
      const x0 = originX + cx * cellExtent;
      const z0 = originZ + cy * cellExtent;
      const step = cellExtent / (nSub - 1);
      const positions = new Float32Array(nSub * nSub * 3);
      const colors = new Float32Array(nSub * nSub * 3);
      for (let j = 0; j < nSub; j++) {
        for (let i = 0; i < nSub; i++) {
          const n = j * nSub + i;
          const x = x0 + i * step;
          const z = z0 + j * step;
          positions[n * 3 + 0] = x;
          positions[n * 3 + 1] = heightAt(x, z);
          positions[n * 3 + 2] = z;
          color.set(landcoverColor[landcoverAt(x, z)] ?? terrainLook.meadow);
          colors[n * 3 + 0] = color.r;
          colors[n * 3 + 1] = color.g;
          colors[n * 3 + 2] = color.b;
        }
      }
      const indices: number[] = [];
      for (let j = 0; j < nSub - 1; j++) {
        for (let i = 0; i < nSub - 1; i++) {
          const a = j * nSub + i;
          const b = a + 1;
          const c = a + nSub;
          const d = c + 1;
          indices.push(a, c, b, b, c, d);
        }
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
      geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
      geo.setIndex(
        nSub * nSub > 65535
          ? new THREE.BufferAttribute(new Uint32Array(indices), 1)
          : new THREE.BufferAttribute(new Uint16Array(indices), 1),
      );
      geo.computeVertexNormals();
      const mesh = new THREE.Mesh(geo, mat);
      mesh.name = nameFor(cx, cy);
      mesh.receiveShadow = true;
      mesh.castShadow = false;
      group.add(mesh);
      meshes.set(`${cx}_${cy}`, mesh);
    }
  }

  return { group, meshes };
}

/**
 * M3 backdrop (Task 6): the L0 overview tile split into a `cellsPerSide`
 * grid of independent sub-meshes aligned to the L2 tile regions (the bake
 * subdivides 4x4 per level → 16 L2 tiles per side of the world square), so
 * the streamer can hide exactly the backdrop cells whose region is covered
 * by a live fine tile. The coarse L0 surface deviates from the fine terrain
 * by up to ~±20 m — where it sits ABOVE it, an always-on L0 would veil whole
 * districts, so covered cells must disappear per-region rather than z-fight.
 *
 * Geometry per cell: see buildResampledCellMeshes (bilinear, watertight).
 *
 * Returns the group plus a `"x_y"`-keyed mesh map in L2-tile index space —
 * main.ts flips `mesh.visible` from the streamer's onReady/onUnload.
 */
export function buildL0Backdrop(
  dec: DecodedTile,
  cellsPerSide: number,
  opts?: { corridorMask?: CorridorMask },
): { group: THREE.Group; meshes: Map<string, THREE.Mesh> } {
  const { gridN, cellSize } = dec.tile;
  const cellExtent = ((gridN - 1) * cellSize) / cellsPerSide;
  // ~4 segments per 1000 m source cell keeps the resample visually identical
  // to the source surface; +1 for the closing vertex row.
  const nSub = Math.max(2, Math.round(cellExtent / 250) + 1);
  const out = buildResampledCellMeshes(
    dec,
    cellsPerSide,
    nSub,
    terrainMaterialFor(opts?.corridorMask),
    (cx, cy) => `terrainL0Backdrop/${cx}_${cy}`,
  );
  out.group.name = 'terrainL0Backdrop';
  return out;
}

/**
 * #141 per-sub-cell L1 hide: one streamed tile's terrain split into a 4×4
 * grid of sub-meshes exactly congruent with its L2 children, so main.ts can
 * hide precisely the sub-cell a live L2 tile replaces instead of the whole
 * 5-km tile. An index-split is NOT possible here (L1 gridN=51 → 50 source
 * cells per side, not divisible by 4: the sub-cell border falls mid-cell), so
 * the cells are bilinearly resampled at HALF the source cell size — every
 * resampled vertex (borders included) lies exactly on the source's bilinear
 * surface, keeping seams watertight between sub-cells AND between adjacent
 * tiles resampled with the same scheme (no new seams vs. the pre-split mesh).
 *
 * Returned map is keyed `"i_j"` (i, j ∈ 0..cellsPerSide-1) in sub-cell index
 * space; L2 child (x, y) maps to sub-cell (x & 3, y & 3) of L1 (x>>2, y>>2).
 */
export function buildTileSubCellTerrain(
  dec: DecodedTile,
  cellsPerSide: number,
  opts?: { corridorMask?: CorridorMask },
): { group: THREE.Group; meshes: Map<string, THREE.Mesh> } {
  const { gridN, cellSize } = dec.tile;
  const cellExtent = ((gridN - 1) * cellSize) / cellsPerSide;
  // Half the source cell size: the resample lands on every source vertex AND
  // every mid-cell point, so the surface tracks the source bilinear patches
  // closely while the sub-cell borders (multiples of cellSize/2 when
  // (gridN-1) % (cellsPerSide/2) aligns, mid-patch otherwise) stay exactly on
  // the bilinear surface either way.
  const nSub = Math.max(2, Math.round(cellExtent / (cellSize / 2)) + 1);
  const out = buildResampledCellMeshes(
    dec,
    cellsPerSide,
    nSub,
    terrainMaterialFor(opts?.corridorMask),
    (cx, cy) => `tileTerrain/L${dec.level}/${dec.x}_${dec.y}#${cx}_${cy}`,
  );
  out.group.name = `tileTerrainCells/L${dec.level}/${dec.x}_${dec.y}`;
  return out;
}
