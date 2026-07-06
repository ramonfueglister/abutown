// tests/geo/tileContent.test.ts
// Task 4 (M3): per-tile materialization — terrain mesh + merged building
// prisms + filtered TreeSpecs, plate exclusion, idempotent dispose.
import { describe, expect, it, vi } from 'vitest';
import * as THREE from 'three/webgpu';
import { create } from '@bufbuild/protobuf';
import { WorldTileSchema } from '../../src/proto/world_pb.js';
import { materializeTile } from '../../src/diorama/ksw/geo/tileContent';
import type { DecodedTile } from '../../src/diorama/ksw/geo/worldData';

// Fixture: 2×2 heightfield tile with 2 buildings and 3 trees.
// Building A: 10×10 m square centred at (0,0)   → centroid inside plateRect.
// Building B: 10×10 m square centred at (200,200) → outside.
// Trees: (5,5) inside plateRect; (60,0) and (0,70) outside.
function makeDec(): DecodedTile {
  const tile = create(WorldTileSchema, {
    level: 2,
    x: 3,
    y: 4,
    gridN: 2,
    cellSize: 10,
    originX: 0,
    originZ: 0,
    height: [400, 401, 402, 403],
    landcover: [1, 1, 1, 1],
    bId: ['a', 'b'],
    bUsage: [0, 0],
    bHeight: [10, 12],
    bBaseY: [400, 402],
    bFpOffset: [0, 4],
    bFpX: [-5, 5, 5, -5, 195, 205, 205, 195],
    bFpZ: [-5, -5, 5, 5, 195, 195, 205, 205],
    tX: [5, 60, 0],
    tZ: [5, 0, 70],
    tH: [10, 12, 8],
    tR: [3, 4, 2],
    tKind: [0, 1, 0],
    tFamily: [0, 3, 1],
  });
  return { level: 2, x: 3, y: 4, tile };
}

const plateRect = { x: 0, z: 0, w: 100, d: 100 };

function meshNames(group: THREE.Group): string[] {
  const names: string[] = [];
  group.traverse((o) => {
    if ((o as { isMesh?: boolean }).isMesh) names.push(o.name);
  });
  return names;
}

describe('materializeTile', () => {
  it('überspringt Platten-Inhalt und baut den Rest', () => {
    const content = materializeTile(makeDec(), {
      plateRect,
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    const meshes = meshNames(content.group);
    expect(meshes.some((n) => n.startsWith('tileTerrain'))).toBe(true);
    expect(meshes.some((n) => n.startsWith('tileBuildings'))).toBe(true);
    // 1 of 2 buildings kept (A's centroid sits in the plate), 2 of 3 trees.
    expect(content.group.userData.buildingCount).toBe(1);
    expect(content.group.userData.treeCount).toBe(2);
    expect(content.group.userData.plateSkipped).toBe(2); // 1 building + 1 tree
    expect(content.trees).toHaveLength(2);
    expect(content.trees.map((t) => t.x)).toEqual([60, 0]);
    expect(content.key).toBe('L2/3_4');
    expect(content.treeKey).toBe('L2/3_4');
  });

  it('plateRect null: nichts wird gefiltert', () => {
    const content = materializeTile(makeDec(), {
      plateRect: null,
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    expect(content.group.userData.buildingCount).toBe(2);
    expect(content.group.userData.treeCount).toBe(3);
    expect(content.group.userData.plateSkipped).toBe(0);
    expect(content.trees).toHaveLength(3);
  });

  it('dispose gibt Geometrien frei und lässt geteilte Materialien leben; idempotent', () => {
    const content = materializeTile(makeDec(), {
      plateRect: null,
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    const geoSpies: ReturnType<typeof vi.spyOn>[] = [];
    const matSpies: ReturnType<typeof vi.spyOn>[] = [];
    content.group.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      geoSpies.push(vi.spyOn(mesh.geometry, 'dispose'));
      const mat = mesh.material as THREE.Material;
      matSpies.push(vi.spyOn(mat, 'dispose'));
    });
    expect(geoSpies.length).toBeGreaterThan(0);

    content.dispose();
    for (const s of geoSpies) expect(s).toHaveBeenCalledTimes(1);
    for (const s of matSpies) expect(s).not.toHaveBeenCalled();

    content.dispose(); // idempotent: no double-dispose
    for (const s of geoSpies) expect(s).toHaveBeenCalledTimes(1);
  });

  it('buildings:false lässt Gebäude weg (L1-Modus), Bäume bleiben', () => {
    const content = materializeTile(makeDec(), {
      plateRect,
      groundShiftY: 0,
      buildings: false,
      trees: true,
    });
    const meshes = meshNames(content.group);
    expect(meshes.some((n) => n.startsWith('tileBuildings'))).toBe(false);
    expect(meshes.some((n) => n.startsWith('tileTerrain'))).toBe(true);
    expect(content.group.userData.buildingCount).toBe(0);
    expect(content.trees).toHaveLength(2); // Task 5 baut daraus Impostors
  });

  it('trees:false liefert keine Specs und treeKey null (L0-Modus)', () => {
    const content = materializeTile(makeDec(), {
      plateRect,
      groundShiftY: 0,
      buildings: false,
      trees: false,
    });
    expect(content.trees).toEqual([]);
    expect(content.treeKey).toBeNull();
    expect(content.group.userData.treeCount).toBe(0);
  });

  it('groundShiftY verschiebt die Group, nicht die Geometrie', () => {
    const dec = makeDec();
    const shifted = materializeTile(dec, {
      plateRect: null,
      groundShiftY: -400,
      buildings: true,
      trees: false,
    });
    expect(shifted.group.position.y).toBe(-400);
    // Geometrie bleibt in absoluten Bake-Metern: Terrain-Vertex-y = DEM-Höhe.
    const terrain = shifted.group.children.find((c) =>
      c.name.startsWith('tileTerrain'),
    ) as THREE.Mesh;
    expect(terrain.geometry.attributes.position.getY(0)).toBeCloseTo(400);
  });

  it('Gebäude-Prisma steht auf bBaseY und reicht bis bBaseY+bHeight', () => {
    const content = materializeTile(makeDec(), {
      plateRect,
      groundShiftY: 0,
      buildings: true,
      trees: false,
    });
    const buildings = content.group.children.find((c) =>
      c.name.startsWith('tileBuildings'),
    ) as THREE.Mesh;
    buildings.geometry.computeBoundingBox();
    const bb = buildings.geometry.boundingBox!;
    // only building B survives: base 402, top 414, footprint x/z 195..205
    expect(bb.min.y).toBeCloseTo(402);
    expect(bb.max.y).toBeCloseTo(414);
    expect(bb.min.x).toBeCloseTo(195);
    expect(bb.max.x).toBeCloseTo(205);
  });
});
