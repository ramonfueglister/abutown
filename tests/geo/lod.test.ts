import { describe, expect, it, vi } from 'vitest';
import { applyCityLod, cityLodState, lampLodVisibility, type CityLodRefs, type LampVis } from '../../src/diorama/ksw/geo/lod';

describe('cityLodState', () => {
  it('classifies rings', () => {
    expect(cityLodState(100, 'near')).toBe('near');
    expect(cityLodState(300, 'near')).toBe('mid');
    // midR is 1200 since the SOTA-2026 pass (facade raster stays on at the
    // city establishing framing, radius ~820).
    expect(cityLodState(900, 'mid')).toBe('mid');
    expect(cityLodState(1400, 'mid')).toBe('far');
  });
  it('hysteresis: no flip inside the band', () => {
    expect(cityLodState(155, 'near')).toBe('near'); // 150×1.1=165 upper band
    expect(cityLodState(166, 'near')).toBe('mid');
    expect(cityLodState(145, 'mid')).toBe('mid'); // 150×0.9=135 lower band
    expect(cityLodState(130, 'mid')).toBe('near');
  });
});

describe('lampLodVisibility', () => {
  const prev: LampVis = { hardware: true, glow: true };
  it('close in: both the lamp hardware and the glow render', () => {
    const v = lampLodVisibility(120, prev);
    expect(v.hardware).toBe(true);
    expect(v.glow).toBe(true);
  });
  it('city establishing framing (~820): hardware culled, glow kept', () => {
    // The regression: at radius 820 the facade LOD left lamps ON, so 17.9k
    // opaque posts/bulbs cluttered every distant street and scintillated.
    // Hardware must be OFF here; the (day-invisible) glow stays for the cozy
    // night atmosphere.
    const v = lampLodVisibility(820, prev);
    expect(v.hardware).toBe(false);
    expect(v.glow).toBe(true);
  });
  it('far horizon: both culled so the distance stays clean', () => {
    const v = lampLodVisibility(2000, prev);
    expect(v.hardware).toBe(false);
    expect(v.glow).toBe(false);
  });
  it('hysteresis: no flip-flop inside either band', () => {
    // hardwareR 300, glowR 1500, hysteresis 0.12 → hardware band [264, 336].
    expect(lampLodVisibility(310, { hardware: true, glow: true }).hardware).toBe(true); // was on, still inside band
    expect(lampLodVisibility(310, { hardware: false, glow: true }).hardware).toBe(false); // was off, still inside band
    expect(lampLodVisibility(340, { hardware: true, glow: true }).hardware).toBe(false); // above band → off
    expect(lampLodVisibility(260, { hardware: false, glow: true }).hardware).toBe(true); // below band → on
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
      footways: { visible: true } as CityLodRefs['footways'],
      // Trees no longer ring-toggle visibility (compaction + vertex collapse
      // handle distance LOD); the ring only drives tree shadows. Lamps left
      // applyCityLod entirely (own lampLodVisibility) in the 2026-07-07 fix.
      setTreeShadows,
    };
  }

  it('far: facade detail off + hides footways, shadows off', () => {
    const refs = makeRefs();
    applyCityLod('far', refs);
    expect(refs.setFacadeDetail).toHaveBeenCalledWith(false);
    expect(refs.footways?.visible).toBe(false);
    expect(refs.setTreeShadows).toHaveBeenCalledWith(false);
  });

  it('mid: facade detail on + shows footways, shadows off', () => {
    const refs = makeRefs();
    applyCityLod('mid', refs);
    expect(refs.setFacadeDetail).toHaveBeenCalledWith(true);
    expect(refs.footways?.visible).toBe(true);
    expect(refs.setTreeShadows).toHaveBeenCalledWith(false);
  });

  it('near: everything on, tree shadows on', () => {
    const refs = makeRefs();
    applyCityLod('near', refs);
    expect(refs.setFacadeDetail).toHaveBeenCalledWith(true);
    expect(refs.footways?.visible).toBe(true);
    expect(refs.setTreeShadows).toHaveBeenCalledWith(true);
  });

  it('is null-tolerant: partially-null refs do not throw', () => {
    const setTreeShadows = vi.fn();
    const setFacadeDetail = vi.fn();
    const refs: CityLodRefs = {
      setFacadeDetail,
      footways: { visible: true } as CityLodRefs['footways'],
      setTreeShadows,
    };
    expect(() => applyCityLod('far', refs)).not.toThrow();
    expect(() => applyCityLod('mid', refs)).not.toThrow();
    expect(() => applyCityLod('near', refs)).not.toThrow();
    expect(refs.footways?.visible).toBe(true);
  });
});
