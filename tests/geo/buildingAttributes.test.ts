import { describe, expect, it } from 'vitest';
import { getBuildingHoverInfo } from '../../src/diorama/ksw/geo/buildingAttributes';
import { cityBuildings, kswBuildings } from '../../src/diorama/ksw/geo/geoData';

describe('getBuildingHoverInfo', () => {
  it('resolves every baked building id', () => {
    for (const b of [...cityBuildings, ...kswBuildings]) {
      expect(getBuildingHoverInfo(b.id)).toBeDefined();
    }
  });
  it('carries the enrichment through (coverage gates re-asserted client-side)', () => {
    const all = [...cityBuildings, ...kswBuildings];
    const zoned = all.filter((b) => getBuildingHoverInfo(b.id)?.bauzone).length;
    const gwred = all.filter((b) => getBuildingHoverInfo(b.id)?.gwrCategory).length;
    expect(zoned / all.length).toBeGreaterThan(0.85);
    expect(gwred / all.length).toBeGreaterThan(0.5);
  });
  it('unknown id → undefined', () => {
    expect(getBuildingHoverInfo('{NOPE}')).toBeUndefined();
  });
});
