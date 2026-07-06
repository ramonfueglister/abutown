import { describe, expect, it } from 'vitest';
// @ts-ignore — plain-ESM bake lib
import { familyFor, sizeFor, treeSpec, GROWTH } from '../../scripts/geo/lib/style.mjs';

describe('familyFor', () => {
  it('Nadelwald → überwiegend conic', () => {
    let conic = 0;
    for (let i = 0; i < 200; i++) {
      const f = familyFor(i * 13.7, i * 7.1, 'conifer', { green: 'wood', leafType: 'needleleaved' });
      if (f === 'conic') conic++;
      expect(['conic', 'slender']).toContain(f);
    }
    expect(conic).toBeGreaterThan(120); // ~70%
  });
  it('Laubwald → oval/spreading/tall, nie conifer-Familien', () => {
    for (let i = 0; i < 100; i++) {
      expect(['oval', 'spreading', 'tall']).toContain(
        familyFor(i * 3.3, i * 9.9, 'broad', { green: 'forest', leafType: 'broadleaved' }),
      );
    }
  });
  it('deterministisch: gleiche Koordinate → gleiche Familie', () => {
    expect(familyFor(101.5, -33.25, 'broad', {})).toBe(familyFor(101.5, -33.25, 'broad', {}));
  });
});

describe('sizeFor', () => {
  it('liefert Spreizung statt Uniform-Defaults (≥3 m Höhen-Spanne über 100 Bäume)', () => {
    const hs = [];
    for (let i = 0; i < 100; i++) hs.push(sizeFor('oval', i * 17.3, i * 5.7).h);
    expect(Math.max(...hs) - Math.min(...hs)).toBeGreaterThan(3);
  });
  it('bleibt unter h∞ und über Sapling-Minimum', () => {
    for (let i = 0; i < 100; i++) {
      const { h, r } = sizeFor('spreading', i * 7.7, i * 3.1);
      expect(h).toBeGreaterThan(3);
      expect(h).toBeLessThan(GROWTH.spreading.hInf);
      expect(r).toBeGreaterThan(1);
    }
  });
});

describe('treeSpec', () => {
  it('explizite OSM-Tags gewinnen weiterhin', () => {
    const s = treeSpec({ height: '17', diameter_crown: '8' }, 5, 5, { green: 'park' });
    expect(s.h).toBe(17);
    expect(s.r).toBe(4);
  });
  it('trägt family', () => {
    expect(treeSpec({}, 5, 5, { green: 'park' }).family).toBeDefined();
  });
});
