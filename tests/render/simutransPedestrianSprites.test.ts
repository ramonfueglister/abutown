import { describe, expect, it } from 'vitest';
import {
  candidateSimutransPedestrianSprites,
  SIMUTRANS_PEDESTRIAN_ASSET_PATHS,
  simutransPedestrianDisplayScale,
  simutransPedestrianFrameForGridDelta,
  simutransPedestrianFrameRect,
} from '../../src/render/simutransPedestrianSprites';
import { pak128AssetPack } from '../../src/assets/pak128Catalog';

describe('Simutrans pak128 pedestrian sprites', () => {
  const stalePedestrianRoot = ['pak128', 'pedestrians'].join('-');

  it('exposes only single-person pak128 pedestrian variants', () => {
    const sprites = candidateSimutransPedestrianSprites();
    const kinds = new Set(sprites.map((sprite) => sprite.kind));
    const sheets = new Set(sprites.map((sprite) => sprite.sheet));

    expect(sprites.length).toBeGreaterThanOrEqual(5);
    expect([...kinds]).toEqual(['pedestrian']);
    expect(sheets).toEqual(new Set(['pedestrians-1']));
  });

  it('loads pedestrian sheets from the active pak128 asset tree', () => {
    expect(SIMUTRANS_PEDESTRIAN_ASSET_PATHS['pedestrians-1']).toBe(pak128AssetPack.require('agent.pedestrian').path);
    expect(Object.values(SIMUTRANS_PEDESTRIAN_ASSET_PATHS).every((path) => path.startsWith('/simutrans-assets/pak128/'))).toBe(true);
    expect(Object.values(SIMUTRANS_PEDESTRIAN_ASSET_PATHS).every((path) => !path.includes(stalePedestrianRoot))).toBe(true);
  });

  it('maps pak128 DAT image coordinates to 128px source cells', () => {
    expect(simutransPedestrianFrameRect({ sheet: 'pedestrians-1', row: 2, kind: 'pedestrian', scale: 0.45 }, 'E')).toEqual({
      x: 256,
      y: 256,
      width: 128,
      height: 128,
    });
  });

  it('uses display scales large enough for cropped pak128 figures to remain visible', () => {
    const sprites = candidateSimutransPedestrianSprites();

    expect(Math.min(...sprites.filter((sprite) => sprite.kind === 'pedestrian').map((sprite) => sprite.scale))).toBeGreaterThanOrEqual(0.92);
    expect(Math.max(...sprites.map((sprite) => sprite.scale))).toBeLessThanOrEqual(0.98);
  });

  it('keeps pak128 agents small when the camera zooms in', () => {
    expect(simutransPedestrianDisplayScale(0.96, 0.56)).toBeGreaterThan(0.96);
    expect(simutransPedestrianDisplayScale(0.96, 2)).toBeLessThan(0.55);
  });

  it('selects Simutrans direction keys from grid movement', () => {
    expect(simutransPedestrianFrameForGridDelta({ x: 1, y: 0 })).toBe('SE');
    expect(simutransPedestrianFrameForGridDelta({ x: 0, y: 1 })).toBe('SW');
    expect(simutransPedestrianFrameForGridDelta({ x: -1, y: 0 })).toBe('NW');
    expect(simutransPedestrianFrameForGridDelta({ x: 0, y: -1 })).toBe('NE');
  });
});
