// Task 5 (M3): dynamic per-tile tree pools on the TreeLayer.
// Full-detail InstancedMeshes stay GLOBAL (compactNear pulls the near set from
// ALL pools — boot + tiles — via the spatial grid); impostor quads are built
// PER TILE as their own small InstancedMesh so removeTileTrees can dispose
// exactly that tile's far-field mesh.
import { describe, expect, it, vi } from 'vitest';
import * as THREE from 'three/webgpu';
import { allArchetypes } from '../../src/diorama/ksw/geo/treeArchetypes';
import { buildTreeLayer, type TreeLayer } from '../../src/diorama/ksw/geo/treeLayer';
import type { TreeSpec } from '../../src/diorama/ksw/geo/geoData';

// Boot stand clustered around the origin (well inside NEAR_TREE_DIST of 0,0).
const bootSpecs: TreeSpec[] = Array.from({ length: 40 }, (_, i) => ({
  x: (i % 8) * 9.1,
  z: Math.floor(i / 8) * 7.3,
  h: 6 + (i % 7),
  r: 2 + (i % 4) * 0.6,
  kind: i % 3 === 0 ? 'conifer' : 'broad',
}));

// Tile stand clustered around (2000, 2000) — far beyond NEAR_TREE_DIST (165 m)
// from the boot stand, so compaction at either point isolates one pool.
const TILE_X = 2000;
const TILE_Z = 2000;
const tileSpecs: TreeSpec[] = Array.from({ length: 25 }, (_, i) => ({
  x: TILE_X + (i % 5) * 8.3,
  z: TILE_Z + Math.floor(i / 5) * 6.7,
  h: 5 + (i % 6),
  r: 1.8 + (i % 3) * 0.7,
  kind: i % 4 === 0 ? 'conifer' : 'broad',
}));

const KEY = 'L2/12_7';

function makeLayer(): TreeLayer {
  const layer = buildTreeLayer(bootSpecs);
  // Per-tile impostor meshes share the boot-baked atlas texture; under vitest
  // (no GPU) a bare Texture object suffices — mesh construction is pure JS.
  layer.setImpostorContext(new THREE.Texture(), allArchetypes().length);
  return layer;
}

function fullCount(layer: TreeLayer): number {
  return layer.fullMeshes.reduce((s, m) => s + m.count, 0);
}

function tileImpostor(layer: TreeLayer, key: string): THREE.InstancedMesh | undefined {
  return layer.group.children.find((c) => c.name === `treeImpostors:${key}`) as
    | THREE.InstancedMesh
    | undefined;
}

describe('TreeLayer tile pools', () => {
  it('addTileTrees grows instances deterministically and tileKeys lists the key', () => {
    const a = makeLayer();
    const b = makeLayer();
    expect(a.tileKeys()).toEqual([]);

    const bootN = a.instances.length;
    a.addTileTrees(KEY, tileSpecs);
    b.addTileTrees(KEY, tileSpecs);

    expect(a.instances.length).toBe(bootN + tileSpecs.length);
    expect(a.tileKeys()).toEqual([KEY]);
    // Deterministic assignment: same specs → same archetypes/tints across layers.
    const sliceA = a.instances.slice(bootN);
    const sliceB = b.instances.slice(bootN);
    expect(sliceA.map((t) => t.archetype)).toEqual(sliceB.map((t) => t.archetype));
    expect(sliceA.map((t) => t.tint.getHex())).toEqual(sliceB.map((t) => t.tint.getHex()));
    // The tile got its own impostor mesh, sized to the tile.
    const imp = tileImpostor(a, KEY);
    expect(imp).toBeDefined();
    expect(imp!.count).toBe(tileSpecs.length);
  });

  it('compactNear pulls tile instances into the GLOBAL full meshes', () => {
    const layer = makeLayer();
    layer.compactNear(0, 0);
    const bootNear = fullCount(layer);
    expect(bootNear).toBe(bootSpecs.length);

    layer.addTileTrees(KEY, tileSpecs);
    layer.compactNear(TILE_X, TILE_Z);
    expect(fullCount(layer)).toBe(tileSpecs.length);
    // Boot stand untouched by the tile pool.
    layer.compactNear(0, 0);
    expect(fullCount(layer)).toBe(bootNear);
  });

  it('removeTileTrees removes exactly the tile instances and disposes the tile impostor mesh', () => {
    const layer = makeLayer();
    const bootN = layer.instances.length;
    layer.addTileTrees(KEY, tileSpecs);

    const imp = tileImpostor(layer, KEY)!;
    const geoDispose = vi.spyOn(imp.geometry, 'dispose');
    const meshDispose = vi.spyOn(imp, 'dispose');

    layer.removeTileTrees(KEY);

    expect(layer.instances.length).toBe(bootN);
    expect(layer.tileKeys()).toEqual([]);
    expect(geoDispose).toHaveBeenCalledTimes(1);
    expect(meshDispose).toHaveBeenCalledTimes(1);
    expect(tileImpostor(layer, KEY)).toBeUndefined();

    // Grid + compaction no longer see the tile's trees…
    layer.compactNear(TILE_X, TILE_Z);
    expect(fullCount(layer)).toBe(0);
    // …while the boot stand still compacts as before.
    layer.compactNear(0, 0);
    expect(fullCount(layer)).toBe(bootSpecs.length);
  });

  it('throws on duplicate add for the same key', () => {
    const layer = makeLayer();
    layer.addTileTrees(KEY, tileSpecs);
    expect(() => layer.addTileTrees(KEY, tileSpecs)).toThrow(/L2\/12_7/);
  });

  it('throws when removing an unknown key', () => {
    const layer = makeLayer();
    expect(() => layer.removeTileTrees('L2/9_9')).toThrow(/L2\/9_9/);
  });

  it('add → remove → add on the same key works', () => {
    const layer = makeLayer();
    const bootN = layer.instances.length;
    layer.addTileTrees(KEY, tileSpecs);
    layer.removeTileTrees(KEY);
    layer.addTileTrees(KEY, tileSpecs);
    expect(layer.instances.length).toBe(bootN + tileSpecs.length);
    expect(layer.tileKeys()).toEqual([KEY]);
    expect(tileImpostor(layer, KEY)?.count).toBe(tileSpecs.length);
    layer.compactNear(TILE_X, TILE_Z);
    expect(fullCount(layer)).toBe(tileSpecs.length);
  });

  it('addTileTrees without impostor context throws (no silent impostor-less pool)', () => {
    const layer = buildTreeLayer(bootSpecs);
    expect(() => layer.addTileTrees(KEY, tileSpecs)).toThrow(/setImpostorContext/);
  });

  it('full meshes carry the documented tile headroom (capacity >= 4096 per archetype)', () => {
    const layer = makeLayer();
    for (const m of layer.fullMeshes) {
      expect(m.instanceMatrix.count).toBeGreaterThanOrEqual(4096);
    }
  });
});
