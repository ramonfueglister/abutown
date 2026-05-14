import { describe, expect, test } from 'vitest';
import { normalizeOpenTtdMap } from '../../scripts/openttdMapImportLib.mjs';

describe('normalizeOpenTtdMap', () => {
  test('samples OpenTTD tile types into a 512 square with terrain and object layers', () => {
    const width = 8;
    const height = 8;
    const source = new Uint8Array(width * height).fill(0);

    const set = (x: number, y: number, type: number) => {
      source[y * width + x] = type;
    };

    set(0, 0, 7);
    set(1, 0, 7);
    set(0, 1, 7);
    set(1, 1, 7);

    set(2, 0, 6);
    set(3, 0, 6);
    set(2, 1, 6);
    set(3, 1, 6);

    set(4, 0, 2);
    set(5, 0, 2);
    set(4, 1, 2);
    set(5, 1, 2);

    set(6, 0, 9);
    set(7, 0, 9);
    set(6, 1, 9);
    set(7, 1, 9);

    set(0, 2, 3);
    set(1, 2, 3);
    set(0, 3, 3);
    set(1, 3, 3);

    set(2, 2, 4);
    set(3, 2, 4);
    set(2, 3, 4);
    set(3, 3, 4);

    set(4, 2, 8);
    set(5, 2, 8);
    set(4, 3, 8);
    set(5, 3, 8);

    set(6, 2, 5);
    set(7, 2, 5);
    set(6, 3, 5);
    set(7, 3, 5);

    const result = normalizeOpenTtdMap({
      id: 'fixture',
      sourceWidth: width,
      sourceHeight: height,
      targetSize: 4,
      tileTypes: source,
    });

    expect(result.width).toBe(4);
    expect(result.height).toBe(4);
    expect(result.terrainKinds).toEqual(['grass', 'water', 'riverbank', 'forest']);
    expect(result.terrainRle).toEqual([
      [2, 1],
      [1, 1],
      [2, 1],
      [0, 2],
      [3, 1],
      [0, 10],
    ]);
    expect(result.roads).toEqual([
      [2, 0, 2, 0],
      [3, 0, 8, 1],
    ]);
    expect(result.buildings).toHaveLength(4);
    expect(new Set(result.buildings.map(([x, y]) => `${x}:${y}`)).size).toBe(4);
    expect(result.trees).toEqual([[1, 1]]);
    expect(result.details).toHaveLength(8);
    expect(result.details.filter(([, , category, asset]) => category === 3 && asset === 9)).toHaveLength(4);
    expect(result.details.filter(([, , category, asset]) => category === 5 && asset === 2)).toHaveLength(4);
  });

  test('preserves multiple source house tiles instead of collapsing them into one target building', () => {
    const width = 8;
    const height = 8;
    const source = new Uint8Array(width * height).fill(0);
    for (const [x, y] of [[0, 0], [1, 0], [0, 1], [1, 1]]) {
      source[y * width + x] = 3;
    }

    const result = normalizeOpenTtdMap({
      id: 'dense-house-fixture',
      sourceWidth: width,
      sourceHeight: height,
      targetSize: 4,
      tileTypes: source,
    });

    expect(result.buildings).toHaveLength(4);
    expect(new Set(result.buildings.map(([x, y]) => `${x}:${y}`)).size).toBe(4);
  });

  test('keeps imported house tiles visually clear of water after normalization', () => {
    const width = 8;
    const height = 8;
    const source = new Uint8Array(width * height).fill(0);
    for (let y = 0; y <= 3; y += 1) {
      for (let x = 0; x <= 3; x += 1) {
        source[y * width + x] = 6;
      }
    }
    source[4 * width + 4] = 3;

    const result = normalizeOpenTtdMap({
      id: 'water-clearance-fixture',
      sourceWidth: width,
      sourceHeight: height,
      targetSize: 8,
      tileTypes: source,
    });

    expect(result.buildings).toHaveLength(1);
    const [buildingX, buildingY] = result.buildings[0];
    const terrain = decodeTerrain(result.terrainRle);
    const waterIndex = result.terrainKinds.indexOf('water');
    const riverbankIndex = result.terrainKinds.indexOf('riverbank');

    expect(terrain[buildingY * result.width + buildingX]).not.toBe(riverbankIndex);
    for (let y = buildingY - 2; y <= buildingY + 2; y += 1) {
      for (let x = buildingX - 2; x <= buildingX + 2; x += 1) {
        if (x < 0 || y < 0 || x >= result.width || y >= result.height) continue;
        expect(terrain[y * result.width + x]).not.toBe(waterIndex);
      }
    }
  });

  test('maps source object tiles to waterfront and roadside city assets', () => {
    const width = 8;
    const height = 8;
    const source = new Uint8Array(width * height).fill(0);
    const set = (x: number, y: number, type: number) => {
      source[y * width + x] = type;
    };

    set(0, 0, 6);
    set(1, 0, 6);
    set(0, 1, 6);
    set(1, 1, 10);

    set(5, 6, 2);
    set(6, 6, 10);

    const result = normalizeOpenTtdMap({
      id: 'object-variety-fixture',
      sourceWidth: width,
      sourceHeight: height,
      targetSize: 8,
      tileTypes: source,
    });

    expect(result.details.some(([, , category, asset]) => category === 6 && [6, 7, 8].includes(asset))).toBe(true);
    expect(result.details.some(([, , category, asset]) => [5, 9].includes(category) && [3, 5].includes(asset))).toBe(true);
  });

  test('fills free land with deterministic forests when a source has no tree layer', () => {
    const width = 32;
    const height = 32;
    const source = new Uint8Array(width * height).fill(0);

    const result = normalizeOpenTtdMap({
      id: 'no-tree-source',
      sourceWidth: width,
      sourceHeight: height,
      targetSize: 32,
      tileTypes: source,
    });

    expect(result.trees.length).toBeGreaterThan(0);
  });
});

function decodeTerrain(rle: Array<[number, number]>): number[] {
  return rle.flatMap(([kind, length]) => Array.from({ length }, () => kind));
}
