import { describe, expect, it } from 'vitest';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pak128AssetPack, PAK128_REQUIRED_ROLES, PAK128_REVISION } from '../../src/assets/pak128Catalog';

describe('pak128 catalog', () => {
  const retiredAssetPattern = new RegExp(`${['open', 'gfx'].join('')}|${['open', 'ttd'].join('')}`, 'i');
  const root = process.cwd();
  const yellowCrossingRoadPath = '/simutrans-assets/pak128/infrastructure/roads/yellow_crossing_hk_surface_road.png';
  const yellowCrossingRoadProvenance = {
    sourcePath: 'infrastructure/roads/yellow_crossing_hk_surface_road.png',
    datPath: 'infrastructure/roads/yellow_crossing_hk_surface_road.dat',
  };
  const yellowSourceDir = join(root, 'public', 'simutrans-assets', 'pak128', 'infrastructure', 'roads', 'yellow-crossing-hk', 'source');
  const yellowRuntimePng = join(root, 'public', 'simutrans-assets', 'pak128', 'infrastructure', 'roads', 'yellow_crossing_hk_surface_road.png');
  const yellowRuntimeDat = join(root, 'public', 'simutrans-assets', 'pak128', 'infrastructure', 'roads', 'yellow_crossing_hk_surface_road.dat');

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

  it('uses the Yellow Crossing runtime sheet for surface road roles only', () => {
    for (const role of ['road.straight', 'road.curve', 'road.intersection'] as const) {
      const asset = pak128AssetPack.require(role);

      expect(asset.path).toBe(yellowCrossingRoadPath);
      expect(asset.provenance).toEqual(expect.objectContaining(yellowCrossingRoadProvenance));
    }

    expect(pak128AssetPack.require('road.bridge').path).toBe('/simutrans-assets/pak128/infrastructure/road_bridges/road_040_bridge.png');
  });

  it('fails clearly when Yellow Crossing source files are missing', () => {
    const emptyRoot = mkdtempSync(join(tmpdir(), 'yellow-crossing-missing-'));
    expect(() => execFileSync(process.execPath, [join(root, 'scripts', 'import-pak128-assets.mjs'), '--yellow-crossing-hk'], {
      cwd: emptyRoot,
      stdio: 'pipe',
    })).toThrow(/Missing Yellow Crossing HK source files/u);
  });

  it('has generated Yellow Crossing runtime and source provenance', () => {
    for (const file of ['Source1.jpg', 'Source2.gif', 'tku_road.zip']) {
      expect(join(yellowSourceDir, file), `Missing Yellow Crossing source file ${file}`).toSatisfy(existsSync);
    }
    expect(yellowRuntimePng).toSatisfy(existsSync);
    expect(yellowRuntimeDat).toSatisfy(existsSync);
    expect(join(yellowSourceDir, 'manifest.json')).toSatisfy(existsSync);

    const png = readFileSync(yellowRuntimePng);
    expect(statSync(yellowRuntimePng).size).toBeGreaterThan(0);
    expect(png.readUInt32BE(16)).toBe(1024);
    expect(png.readUInt32BE(20)).toBe(384);

    const dat = readFileSync(yellowRuntimeDat, 'utf8');
    expect(dat).toContain('Yellow Crossing Addon of Hong Kong');
    expect(dat).toContain('Source1.jpg');
    expect(dat).toContain('Source2.gif');
    expect(dat).toContain('runtime_png=yellow_crossing_hk_surface_road.png');

    const manifest = JSON.parse(readFileSync(join(yellowSourceDir, 'manifest.json'), 'utf8'));
    expect(manifest).toEqual(expect.objectContaining({
      userApprovedReuse: true,
      runtimePng: 'yellow_crossing_hk_surface_road.png',
    }));
    expect(manifest.sourceFiles).toEqual(expect.arrayContaining(['Source1.jpg', 'Source2.gif']));
    expect(manifest.blockedSource).toBe('base Pak128 surface road sheet');
  });
});
