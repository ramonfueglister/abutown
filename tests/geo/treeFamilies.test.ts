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

  // Finding 1 (Critical, task-2-brief.md): kind darf nicht ALLEIN aus dem
  // fehlenden Node-Tag folgen — sonst vetot familyFor jede Konifere, sobald
  // kein explizites leaf_type am Baum selbst hängt (der Normalfall: 8/7271
  // im vorherigen Bake). Familie zuerst (aus dem Kontext-Mix), kind folgt.
  it('ohne explizites leaf_type: Nadelwald-Kontext liefert überwiegend kind=conifer', () => {
    let conifer = 0;
    for (let i = 0; i < 200; i++) {
      const s = treeSpec({}, i * 13.7, i * 7.1, { green: 'wood', leafType: 'needleleaved' });
      if (s.kind === 'conifer') conifer++;
    }
    expect(conifer).toBeGreaterThan(100); // Mehrheit
  });

  it('ohne explizites leaf_type: mixedwood-Kontext liefert 15-45% Koniferen-Anteil', () => {
    let conifer = 0;
    const N = 400;
    for (let i = 0; i < N; i++) {
      const s = treeSpec({}, i * 11.3, i * 5.9, { green: 'wood' }); // kein leafType → mixedwood
      if (s.kind === 'conifer') conifer++;
    }
    const ratio = conifer / N;
    expect(ratio).toBeGreaterThan(0.15);
    expect(ratio).toBeLessThan(0.45);
  });

  it('explizites tags.leaf_type=broadleaved bleibt broad, auch im Nadelwald-Kontext', () => {
    for (let i = 0; i < 100; i++) {
      const s = treeSpec({ leaf_type: 'broadleaved' }, i * 9.1, i * 4.3, { green: 'wood', leafType: 'needleleaved' });
      expect(s.kind).toBe('broad');
    }
  });
});

describe('tiny-crown guard', () => {
  it('ein OSM diameter_crown=0.5 erzeugt kein trunk-only-Skelett (r floored auf 30% der Familien-Erwartung)', () => {
    const s = treeSpec({ diameter_crown: '0.5' }, 12.5, -44.25, { green: 'park' });
    // Roher Tag-Wert wäre r=0.25; der Guard hebt auf mindestens 0.3 × sized.r.
    expect(s.r).toBeGreaterThan(0.25);
    expect(s.r).toBeGreaterThanOrEqual(0.3 * 1); // sized.r ist je Familie > 1 (siehe sizeFor-Bounds-Test)
  });
});
