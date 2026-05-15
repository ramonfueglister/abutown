import { describe, expect, it } from 'vitest';
import { pak128AssetPack, PAK128_REQUIRED_ROLES, PAK128_REVISION } from '../../src/assets/pak128Catalog';

describe('pak128 catalog', () => {
  const legacyAssetPattern = new RegExp(`${['open', 'gfx'].join('')}|${['open', 'ttd'].join('')}`, 'i');

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

  it('does not contain legacy asset paths', () => {
    for (const asset of pak128AssetPack.all()) {
      expect(asset.path).not.toMatch(legacyAssetPattern);
      expect(asset.provenance.sourcePath).not.toMatch(legacyAssetPattern);
    }
  });

  it('uses DAT-backed frame coordinates for known directional assets', () => {
    expect(pak128AssetPack.require('agent.pedestrian').source).toEqual({ x: 0, y: 128, width: 128, height: 128 });
    expect(pak128AssetPack.require('vehicle.bus').source.y).toBe(128);
  });
});
