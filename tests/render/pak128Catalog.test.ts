import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { pak128AssetPack, PAK128_REQUIRED_ROLES, PAK128_REVISION } from '../../src/assets/pak128Catalog';

describe('pak128 catalog', () => {
  const retiredAssetPattern = new RegExp(`${['open', 'gfx'].join('')}|${['open', 'ttd'].join('')}`, 'i');
  const root = process.cwd();
  const pak128RoadPath = '/simutrans-assets/pak128/infrastructure/roads/road_090.png';
  const pak128RoadProvenance = {
    sourcePath: 'infrastructure/roads/road_090.png',
    datPath: 'infrastructure/roads/road_090.dat',
  };
  const yellowIntakeDir = join(root, 'asset-intake', 'yellow-crossing-hk');
  const publicRoadDir = join(root, 'public', 'simutrans-assets', 'pak128', 'infrastructure', 'roads');
  const pak128RoadPng = join(publicRoadDir, 'road_090.png');
  const pak128RoadDat = join(publicRoadDir, 'road_090.dat');

  it('pins the audited source revision', () => {
    expect(PAK128_REVISION).toBe('acdf2f0793a6beee5ea34ea85d308fbbeccf50c5');
  });

  it('defines every runtime role with pak128 provenance', () => {
    for (const role of PAK128_REQUIRED_ROLES) {
      const asset = pak128AssetPack.require(role);
      expect(asset.role).toBe(role);
      expect(asset.path).toMatch(/^\/simutrans-assets\/pak128\//u);
      expect(asset.cleanup).toBe('pak128');
      expect(asset.source.width).toBeGreaterThan(0);
      expect(asset.source.height).toBeGreaterThan(0);
      expect(asset.provenance).toEqual(expect.objectContaining({
        license: 'Artistic-2.0',
        revision: PAK128_REVISION,
      }));
    }
  });

  it('does not contain retired asset paths', () => {
    for (const asset of pak128AssetPack.all()) {
      expect(asset.path).not.toMatch(retiredAssetPattern);
      expect(asset.provenance.sourcePath).not.toMatch(retiredAssetPattern);
    }
  });

  it('uses DAT-backed frame coordinates for known directional assets', () => {
    expect(pak128AssetPack.require('terrain.grass').source).toEqual({ x: 512, y: 0, width: 128, height: 64 });
    expect(pak128AssetPack.require('terrain.water').source).toEqual({ x: 0, y: 128, width: 128, height: 128 });
    expect(pak128AssetPack.require('building.residential.low').source).toEqual({ x: 0, y: 128, width: 128, height: 256 });
    expect(pak128AssetPack.require('agent.pedestrian').source).toEqual({ x: 0, y: 128, width: 128, height: 128 });
    expect(pak128AssetPack.require('vehicle.bus').source.y).toBe(128);
  });

  it('uses real pak128 road sheet for surface road roles until a complete original Yellow Crossing sheet exists', () => {
    expect(pak128RoadPng).toSatisfy(existsSync);
    expect(pak128RoadDat).toSatisfy(existsSync);

    for (const role of ['road.straight', 'road.curve', 'road.intersection'] as const) {
      const asset = pak128AssetPack.require(role);

      expect(asset.path).toBe(pak128RoadPath);
      expect(asset.provenance).toEqual(expect.objectContaining(pak128RoadProvenance));
    }

    expect(pak128AssetPack.require('road.bridge').path).toBe('/simutrans-assets/pak128/infrastructure/road_bridges/road_040_bridge.png');
  });

  it('keeps partial Yellow Crossing material out of the runtime pack', () => {
    for (const file of ['Source1.jpg', 'Source2.gif', 'Demo.png', 'tku_road.zip']) {
      expect(join(yellowIntakeDir, file), `Missing Yellow Crossing intake file ${file}`).toSatisfy(existsSync);
    }

    for (const asset of pak128AssetPack.all()) {
      expect(asset.path).not.toContain('yellow_crossing_hk_surface_road');
      expect(asset.provenance.sourcePath).not.toContain('yellow_crossing_hk_surface_road');
    }

    expect(existsSync(join(publicRoadDir, 'yellow_crossing_hk_surface_road.png'))).toBe(false);
    expect(existsSync(join(publicRoadDir, 'yellow_crossing_hk_surface_road.dat'))).toBe(false);
    expect(existsSync(join(publicRoadDir, 'yellow-crossing-hk'))).toBe(false);
  });
});
