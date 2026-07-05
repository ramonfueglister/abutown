import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';

const PATH = 'data/winterthur/simworld.json';

describe('simworld artifact', () => {
  it('exists and is committed', () => {
    expect(existsSync(PATH)).toBe(true);
  });
  it('has plausible building inventory', () => {
    const w = JSON.parse(readFileSync(PATH, 'utf8'));
    expect(w.meta.anchor.lon).toBeCloseTo(8.7285, 4);
    expect(w.buildings.length).toBeGreaterThan(20000);
    expect(w.buildings.length).toBeLessThan(40000);
    const usages = new Set(w.buildings.map((b: any) => b.usage));
    expect(usages.has(1)).toBe(true); // residential
    expect(usages.has(2) || usages.has(3)).toBe(true); // work
    const withAccess = w.buildings.filter((b: any) => b.access_edge >= 0);
    expect(withAccess.length / w.buildings.length).toBeGreaterThan(0.9);
    for (const b of w.buildings.slice(0, 500)) {
      expect(typeof b.id).toBe('string');
      expect(Number.isFinite(b.x) && Number.isFinite(b.z)).toBe(true);
      expect(b.area_m2).toBeGreaterThan(0);
    }
  });
});
