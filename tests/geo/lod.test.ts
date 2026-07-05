import { describe, expect, it, vi } from 'vitest';
import { applyCityLod, cityLodState, type CityLodRefs } from '../../src/diorama/ksw/geo/lod';

describe('cityLodState', () => {
  it('classifies rings', () => {
    expect(cityLodState(100, 'near')).toBe('near');
    expect(cityLodState(300, 'near')).toBe('mid');
    expect(cityLodState(900, 'mid')).toBe('far');
  });
  it('hysteresis: no flip inside the band', () => {
    expect(cityLodState(155, 'near')).toBe('near'); // 150×1.1=165 upper band
    expect(cityLodState(166, 'near')).toBe('mid');
    expect(cityLodState(145, 'mid')).toBe('mid'); // 150×0.9=135 lower band
    expect(cityLodState(130, 'mid')).toBe('near');
  });
});

describe('applyCityLod', () => {
  function makeRefs(): CityLodRefs & {
    setTreeShadows: ReturnType<typeof vi.fn>;
    setFacadeDetail: ReturnType<typeof vi.fn>;
  } {
    const setTreeShadows = vi.fn();
    const setFacadeDetail = vi.fn();
    return {
      setFacadeDetail,
      lamps: { visible: true } as CityLodRefs['lamps'],
      footways: { visible: true } as CityLodRefs['footways'],
      // Trees no longer ring-toggle visibility (compaction + vertex collapse
      // handle distance LOD); the ring only drives tree shadows.
      setTreeShadows,
    };
  }

  it('far: facade detail off + hides lamps/footways, shadows off', () => {
    const refs = makeRefs();
    applyCityLod('far', refs);
    expect(refs.setFacadeDetail).toHaveBeenCalledWith(false);
    expect(refs.lamps?.visible).toBe(false);
    expect(refs.footways?.visible).toBe(false);
    expect(refs.setTreeShadows).toHaveBeenCalledWith(false);
  });

  it('mid: facade detail on + shows footways/lamps, shadows off', () => {
    const refs = makeRefs();
    applyCityLod('mid', refs);
    expect(refs.setFacadeDetail).toHaveBeenCalledWith(true);
    expect(refs.lamps?.visible).toBe(true);
    expect(refs.footways?.visible).toBe(true);
    expect(refs.setTreeShadows).toHaveBeenCalledWith(false);
  });

  it('near: everything on, tree shadows on', () => {
    const refs = makeRefs();
    applyCityLod('near', refs);
    expect(refs.setFacadeDetail).toHaveBeenCalledWith(true);
    expect(refs.lamps?.visible).toBe(true);
    expect(refs.footways?.visible).toBe(true);
    expect(refs.setTreeShadows).toHaveBeenCalledWith(true);
  });

  it('is null-tolerant: partially-null refs do not throw', () => {
    const setTreeShadows = vi.fn();
    const setFacadeDetail = vi.fn();
    const refs: CityLodRefs = {
      setFacadeDetail,
      lamps: null,
      footways: { visible: true } as CityLodRefs['footways'],
      setTreeShadows,
    };
    expect(() => applyCityLod('far', refs)).not.toThrow();
    expect(() => applyCityLod('mid', refs)).not.toThrow();
    expect(() => applyCityLod('near', refs)).not.toThrow();
    expect(refs.footways?.visible).toBe(true);
  });
});
