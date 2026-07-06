import { describe, expect, it } from 'vitest';
import { transformLanduse } from '../../scripts/geo/lib/landuse.mjs';
import { transformNature } from '../../scripts/geo/lib/transform.mjs';
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

describe('transformNature Slice-2', () => {
  it('Baum im Gebäude-Footprint (+1 m) wird gedroppt', () => {
    const osmNature = { elements: [
      { type: 'node', lat: 0.00001, lon: 0.00001, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const fp = [[0, 0], [5, 0], [5, -5], [0, -5]]; // enthält den Baum (~1.1, -1.1)
    const { trees } = transformNature({ osmNature, projector, buildingFootprints: [fp] });
    expect(trees.length).toBe(0);
  });
  it('Baum im Park erhält family', () => {
    const osmNature = { elements: [
      { type: 'way', tags: { leisure: 'park' }, geometry: [
        { lon: 0, lat: 0 }, { lon: 0.001, lat: 0 }, { lon: 0.001, lat: -0.001 }, { lon: 0, lat: -0.001 }, { lon: 0, lat: 0 } ] },
      { type: 'node', lat: -0.0005, lon: 0.0005, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const { trees } = transformNature({ osmNature, projector });
    expect(trees.length).toBe(1);
    expect(['spreading', 'oval', 'tall']).toContain(trees[0].family);
  });

  // Finding 3 (Minor, task-2-brief.md): Margen-Band um den Footprint (Fund 2's
  // nearFootprint / FOOTPRINT_MARGIN = 1 m) war bisher untestet.
  it('Baum 0.5 m ausserhalb des Footprints wird gedroppt (innerhalb der Marge)', () => {
    const osmNature = { elements: [
      // fp: x in [0,5], z in [-5,0]; Baum bei x=5.5 (0.5 m rechts der Kante x=5)
      { type: 'node', lat: 0.00001 /* z ~ -1.11 */, lon: 5.5 / 111320, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const fp = [[0, 0], [5, 0], [5, -5], [0, -5]];
    const { trees } = transformNature({ osmNature, projector, buildingFootprints: [fp] });
    expect(trees.length).toBe(0);
  });

  it('Baum 2.5 m ausserhalb des Footprints bleibt (ausserhalb der Marge)', () => {
    const osmNature = { elements: [
      // Baum bei x=7.5 (2.5 m rechts der Kante x=5)
      { type: 'node', lat: 0.00001, lon: 7.5 / 111320, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const fp = [[0, 0], [5, 0], [5, -5], [0, -5]];
    const { trees } = transformNature({ osmNature, projector, buildingFootprints: [fp] });
    expect(trees.length).toBe(1);
  });

  // Re-review Finding (Important): footprintSegmentGrid only registered each
  // edge segment at 3 points (both endpoints + midpoint), queried via a 3x3
  // cell lookup (FOOTPRINT_CELL = 8 m). A segment longer than ~43 m therefore
  // has stretches whose 8 m grid cell is never within one cell of any of the
  // 3 registered points, so a tree sitting in the 1 m margin over such a
  // stretch is missed by every 3x3 query and wrongly kept. Regression: 60 m x
  // 10 m footprint, tree 0.5 m outside the 60 m edge at x=40 — registered
  // points are the two corners (gx=0, gx=7) and the midpoint (gx=3); x=40
  // falls in gx=5, which is not adjacent (within 1) to any of {0, 3, 7}, so a
  // 3x3 query there finds nothing under the old registration scheme.
  it('Baum 0.5 m ausserhalb einer langen (>43 m) Kante, fern von Ecken/Mitte, wird gedroppt', () => {
    const osmNature = { elements: [
      // fp: x in [0,60], z in [-10,0]; Baum bei x=40, z=0.5 (0.5 m ausserhalb der Kante z=0)
      { type: 'node', lat: -0.5 / 111320, lon: 40 / 111320, tags: { natural: 'tree' } },
    ]};
    const projector = { toLocal: (lon: number, lat: number) => [lon * 111320, -lat * 111320] };
    const fp = [[0, 0], [60, 0], [60, -10], [0, -10]];
    const { trees } = transformNature({ osmNature, projector, buildingFootprints: [fp] });
    expect(trees.length).toBe(0);
  });
});
