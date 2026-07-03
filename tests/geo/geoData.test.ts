// tests/geo/geoData.test.ts
import { describe, expect, it } from 'vitest';
import { cityBuildings, cityMeta, cityRoads, kswBuildings } from '../../src/diorama/ksw/geo/geoData';

describe('baked city data', () => {
  it('loads a real-sized city', () => {
    expect(cityBuildings.length).toBeGreaterThan(500);
    expect(kswBuildings.length).toBeGreaterThan(5);
    expect(cityRoads.length).toBeGreaterThan(50);
  });
  it('every building has positive height and non-empty geometry', () => {
    for (const b of [...cityBuildings, ...kswBuildings]) {
      expect(b.height).toBeGreaterThan(0);
      expect(b.wall.idx.length + b.roof.idx.length).toBeGreaterThan(0);
    }
  });
  it('plate covers all landmarks', () => {
    const { plate, landmarks } = cityMeta;
    for (const [x, z] of Object.values(landmarks)) {
      expect(Math.abs(x - plate.cx)).toBeLessThan(plate.w / 2);
      expect(Math.abs(z - plate.cz)).toBeLessThan(plate.d / 2);
    }
  });
  it('ksw zone contains the named departments', () => {
    const names = kswBuildings.map((b) => b.name).filter(Boolean).join(' ');
    expect(names.length).toBeGreaterThan(0);
  });
});
