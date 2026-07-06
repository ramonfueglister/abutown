// tests/geo/tileContent.test.ts
// Task 4 (M3): per-tile materialization — terrain mesh + merged building
// prisms + filtered TreeSpecs, plate exclusion, idempotent dispose.
import { describe, expect, it, vi } from 'vitest';
import * as THREE from 'three/webgpu';
import { create } from '@bufbuild/protobuf';
import { WorldTileSchema } from '../../src/proto/world_pb.js';
import { l1SubCellOfL2, materializeTile, subCellKey } from '../../src/diorama/ksw/geo/tileContent';
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

  it('treeExcludeRect grösser als plateRect: filtert Bäume im erweiterten Band, Gebäude bleiben (Platten-Rect)', () => {
    // Reproduces the M3 Task-6 boot-ring double-tree bug: nature.json trees
    // extend past the plate; the streamed tile trees must be excluded over
    // the LARGER samplerRect-equivalent band, while building massing must
    // still only exclude the (smaller) plateRect — otherwise the mid ring
    // loses its massing buildings.
    // Tree at (60,0) sits outside plateRect (w/d=100) but inside a wider
    // treeExcludeRect (w=140,d=100, still centered at 0,0) that now covers
    // it too, while (0,70) stays outside (|70| > d/2=50) and survives.
    const wideRect = { x: 0, z: 0, w: 140, d: 100 };
    const content = materializeTile(makeDec(), {
      plateRect,
      treeExcludeRect: wideRect,
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    // Building B's centroid (200,200) is still outside the wide rect too,
    // so it's unaffected here; the point is buildings only ever check
    // plateRect: building A (centroid 0,0) is dropped because plateRect
    // covers it, same as the baseline test.
    expect(content.group.userData.buildingCount).toBe(1);
    // Trees: (5,5) dropped by plateRect as before; (60,0) is now ALSO
    // dropped because it falls inside the wider treeExcludeRect; only
    // (0,70) survives (outside both rects).
    expect(content.trees).toHaveLength(1);
    expect(content.trees.map((t) => t.x)).toEqual([0]);
    expect(content.group.userData.treeCount).toBe(1);
  });

  it('treeExcludeRect fehlt: fällt auf plateRect für Bäume zurück (Rückwärtskompatibilität)', () => {
    const content = materializeTile(makeDec(), {
      plateRect,
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    // Same as the pre-existing baseline behaviour: only plateRect applies.
    expect(content.trees).toHaveLength(2);
    expect(content.trees.map((t) => t.x)).toEqual([60, 0]);
  });

  it('L2-Tiles bleiben un-subzelliert (subCells null, ein Massing-Mesh)', () => {
    const content = materializeTile(makeDec(), {
      plateRect: null,
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    expect(content.subCells).toBeNull();
    expect(meshNames(content.group).filter((n) => n.startsWith('tileBuildings'))).toEqual(['tileBuildings/L2/3_4']);
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

describe('#141 L2 ↔ L1-Subzellen-Mapping', () => {
  it('L2 (x,y) liegt in L1 (x>>2, y>>2), Subzelle (x&3, y&3)', () => {
    expect(l1SubCellOfL2(0, 0)).toEqual({ l1x: 0, l1y: 0, i: 0, j: 0 });
    expect(l1SubCellOfL2(9, 14)).toEqual({ l1x: 2, l1y: 3, i: 1, j: 2 });
    expect(l1SubCellOfL2(15, 15)).toEqual({ l1x: 3, l1y: 3, i: 3, j: 3 });
    // Rückrichtung: Subzelle (i,j) von L1 (l1x,l1y) deckt genau L2 (l1x*4+i, l1y*4+j).
    for (let x = 0; x < 16; x++) {
      for (let y = 0; y < 16; y++) {
        const { l1x, l1y, i, j } = l1SubCellOfL2(x, y);
        expect(l1x * 4 + i).toBe(x);
        expect(l1y * 4 + j).toBe(y);
      }
    }
    expect(subCellKey(1, 2)).toBe('1_2');
  });
});

describe('materializeTile L1-Subzellen (#141)', () => {
  // L1 fixture: gridN=5, cellSize=100 → extent 400 m, 4×4 Subzellen à 100 m.
  // Gebäude A Schwerpunkt (50,50) → Subzelle 0_0; B (350,150) → 3_1.
  // Bäume: (50,50) → 0_0; (250,350) → 2_3; (399,399) → 3_3.
  function makeL1(): DecodedTile {
    const gridN = 5;
    const tile = create(WorldTileSchema, {
      level: 1,
      x: 2,
      y: 2,
      gridN,
      cellSize: 100,
      originX: 0,
      originZ: 0,
      height: new Array(gridN * gridN).fill(400),
      landcover: new Array(gridN * gridN).fill(1),
      bId: ['a', 'b'],
      bUsage: [0, 0],
      bHeight: [10, 12],
      bBaseY: [400, 400],
      bFpOffset: [0, 4],
      bFpX: [45, 55, 55, 45, 345, 355, 355, 345],
      bFpZ: [45, 45, 55, 55, 145, 145, 155, 155],
      tX: [50, 250, 399],
      tZ: [50, 350, 399],
      tH: [10, 12, 8],
      tR: [3, 4, 2],
      tKind: [0, 1, 0],
      tFamily: [0, 3, 1],
    });
    return { level: 1, x: 2, y: 2, tile };
  }

  const content = materializeTile(makeL1(), {
    plateRect: null,
    groundShiftY: 0,
    buildings: true,
    trees: true,
  });

  it('liefert 16 Subzellen mit je einem Terrain-Sub-Mesh', () => {
    expect(content.subCells).not.toBeNull();
    expect(content.subCells!.size).toBe(16);
    const terrainNames = meshNames(content.group).filter((n) => n.startsWith('tileTerrain/'));
    expect(terrainNames).toHaveLength(16);
    expect(terrainNames).toContain('tileTerrain/L1/2_2#0_0');
    expect(terrainNames).toContain('tileTerrain/L1/2_2#3_3');
    for (const sc of content.subCells!.values()) {
      expect(sc.meshes.length).toBeGreaterThanOrEqual(1);
    }
  });

  it('partitioniert Gebäude per Footprint-Schwerpunkt in Subzellen-Meshes', () => {
    const bNames = meshNames(content.group).filter((n) => n.startsWith('tileBuildings'));
    expect(bNames.sort()).toEqual(['tileBuildings/L1/2_2#0_0', 'tileBuildings/L1/2_2#3_1']);
    expect(content.group.userData.buildingCount).toBe(2);
    // Die Massing-Meshes hängen in den Subzellen-Handles (für visible-Toggle).
    expect(content.subCells!.get('0_0')!.meshes.some((m) => m.name === 'tileBuildings/L1/2_2#0_0')).toBe(true);
    expect(content.subCells!.get('3_1')!.meshes.some((m) => m.name === 'tileBuildings/L1/2_2#3_1')).toBe(true);
    expect(content.subCells!.get('1_1')!.meshes).toHaveLength(1); // nur Terrain
  });

  it('partitioniert Bäume in Subzellen-Pools mit Keys L1/x_y#i_j; treeKey des Tiles ist null', () => {
    expect(content.treeKey).toBeNull(); // per-Subzelle registriert, nie ganzes Tile
    expect(content.trees).toHaveLength(3);
    const sc00 = content.subCells!.get('0_0')!;
    const sc23 = content.subCells!.get('2_3')!;
    const sc33 = content.subCells!.get('3_3')!;
    expect(sc00.treeKey).toBe('L1/2_2#0_0');
    expect(sc00.trees.map((t) => t.x)).toEqual([50]);
    expect(sc23.treeKey).toBe('L1/2_2#2_3');
    expect(sc33.treeKey).toBe('L1/2_2#3_3');
    // leere Subzelle: kein Pool-Key
    expect(content.subCells!.get('1_1')!.treeKey).toBeNull();
    // Summe der Subzellen-Bäume = alle Tile-Bäume (nichts doppelt/verloren)
    const total = [...content.subCells!.values()].reduce((s, sc) => s + sc.trees.length, 0);
    expect(total).toBe(3);
  });

  it('plateRect filtert auch im Subzellen-Pfad (Gebäude + Bäume)', () => {
    const filtered = materializeTile(makeL1(), {
      plateRect: { x: 50, z: 50, w: 100, d: 100 },
      groundShiftY: 0,
      buildings: true,
      trees: true,
    });
    // Gebäude A (50,50) + Baum (50,50) fallen weg.
    expect(filtered.group.userData.buildingCount).toBe(1);
    expect(filtered.subCells!.get('0_0')!.treeKey).toBeNull();
    expect(filtered.trees).toHaveLength(2);
  });
});
