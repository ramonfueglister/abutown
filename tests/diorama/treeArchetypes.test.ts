import { describe, expect, it } from 'vitest';
import {
  BROAD_FAMILIES, CONIFER_FAMILIES, SEEDS_PER_FAMILY,
  allArchetypes, archetypeIndexFor, buildArchetype,
} from '../../src/diorama/ksw/geo/treeArchetypes';

const posHash = (g: import('three').BufferGeometry): string => {
  const a = g.getAttribute('position').array as Float32Array;
  let h = 0;
  for (let i = 0; i < a.length; i++) h = (h * 31 + Math.round(a[i] * 1e4)) | 0;
  return String(h);
};

describe('treeArchetypes', () => {
  it('is deterministic: same family+seed → identical geometry', () => {
    const a = buildArchetype('spreading', 2);
    const b = buildArchetype('spreading', 2);
    expect(posHash(a.geometry)).toBe(posHash(b.geometry));
  });

  it('different seeds of a family produce different geometry', () => {
    expect(posHash(buildArchetype('oval', 0).geometry)).not.toBe(posHash(buildArchetype('oval', 1).geometry));
  });

  it('normalizes to y ∈ [0,1] and reports a consistent crownRadius', () => {
    for (const arch of allArchetypes()) {
      const p = arch.geometry.getAttribute('position').array as Float32Array;
      let minY = Infinity, maxY = -Infinity, maxR = 0;
      for (let i = 0; i < p.length; i += 3) {
        minY = Math.min(minY, p[i + 1]);
        maxY = Math.max(maxY, p[i + 1]);
        maxR = Math.max(maxR, Math.hypot(p[i], p[i + 2]));
      }
      expect(minY).toBeCloseTo(0, 3);
      expect(maxY).toBeCloseTo(1, 3);
      expect(arch.crownRadius).toBeCloseTo(maxR, 3);
    }
  });

  it('carries aPuff vec4 with wood marked -1 and crown puffIndex >= 0', () => {
    const arch = buildArchetype('spreading', 0);
    const ap = arch.geometry.getAttribute('aPuff');
    expect(ap.itemSize).toBe(4);
    const w = ap.array as Float32Array;
    const flags = new Set<number>();
    for (let i = 3; i < w.length; i += 4) flags.add(Math.sign(Math.max(-1, w[i])));
    expect(flags.has(-1)).toBe(true); // trunk exists
    expect(flags.has(1) || flags.has(0)).toBe(true); // crown exists
  });

  it('allArchetypes is families × seeds in stable order', () => {
    const all = allArchetypes();
    expect(all.length).toBe((BROAD_FAMILIES.length + CONIFER_FAMILIES.length) * SEEDS_PER_FAMILY);
    expect(all[0].family).toBe(BROAD_FAMILIES[0]);
    expect(all[0].seed).toBe(0);
  });

  it('every archetype stays under the vertex budget (< 1500 vertices)', () => {
    for (const arch of allArchetypes()) {
      const count = arch.geometry.getAttribute('position').count;
      expect(count).toBeLessThan(1500);
    }
  });

  it('archetypeIndexFor is deterministic, kind-respecting, and spread out', () => {
    const broadRange = BROAD_FAMILIES.length * SEEDS_PER_FAMILY;
    const seen = new Set<number>();
    for (let i = 0; i < 200; i++) {
      const idx = archetypeIndexFor(i * 13.7, i * 7.3, 'broad');
      expect(idx).toBeGreaterThanOrEqual(0);
      expect(idx).toBeLessThan(broadRange);
      seen.add(idx);
    }
    expect(seen.size).toBeGreaterThan(broadRange / 2); // actually varied
    const all = allArchetypes().length;
    const cIdx = archetypeIndexFor(5, 5, 'conifer');
    expect(cIdx).toBeGreaterThanOrEqual(broadRange);
    expect(cIdx).toBeLessThan(all);
    expect(archetypeIndexFor(5, 5, 'conifer')).toBe(cIdx);
  });
});
