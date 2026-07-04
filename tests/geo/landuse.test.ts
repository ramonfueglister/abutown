import { describe, expect, it } from 'vitest';
import { transformLanduse } from '../../scripts/geo/lib/landuse.mjs';
import { ANCHOR, makeProjector } from '../../scripts/geo/lib/project.mjs';

const way = { type: 'way', tags: { landuse: 'forest' }, geometry: [
  { lon: ANCHOR.lon, lat: ANCHOR.lat }, { lon: ANCHOR.lon + 0.001, lat: ANCHOR.lat },
  { lon: ANCHOR.lon + 0.001, lat: ANCHOR.lat + 0.001 }, { lon: ANCHOR.lon, lat: ANCHOR.lat } ] };

const unknownWay = { type: 'way', tags: { landuse: 'quarry' }, geometry: way.geometry };

describe('transformLanduse', () => {
  it('maps forest to Landcover 2 with a local-meter ring', () => {
    const out = transformLanduse({ osmLanduse: { elements: [way] }, projector: makeProjector(ANCHOR) });
    expect(out).toHaveLength(1);
    expect(out[0].kind).toBe(2);
    expect(out[0].ring.length).toBeGreaterThanOrEqual(3);
  });

  it('skips unknown landuse tags', () => {
    const out = transformLanduse({ osmLanduse: { elements: [unknownWay] }, projector: makeProjector(ANCHOR) });
    expect(out).toHaveLength(0);
  });

  it('maps every known kind correctly', () => {
    const mk = (tag: string) => ({ type: 'way', tags: { landuse: tag }, geometry: way.geometry });
    const elements = ['meadow', 'grass', 'wood', 'farmland', 'residential', 'commercial', 'basin'].map(mk);
    const out = transformLanduse({ osmLanduse: { elements }, projector: makeProjector(ANCHOR) });
    expect(out.map((o) => o.kind)).toEqual([1, 1, 2, 3, 4, 5, 6]);
  });
});
