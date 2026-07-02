import { describe, expect, it } from 'vitest';
import { boxGeo, cyl, roundedBox, sph, tor } from '../../src/diorama/ksw/geometryCache';

describe('geometryCache', () => {
  it('roundedBox: same params return the same instance', () => {
    const a = roundedBox(1, 2, 3, 4, 0.2);
    const b = roundedBox(1, 2, 3, 4, 0.2);
    expect(a).toBe(b);
  });

  it('roundedBox: different params return different instances', () => {
    const a = roundedBox(1, 2, 3, 4, 0.2);
    const b = roundedBox(1, 2, 3, 4, 0.25);
    expect(a).not.toBe(b);
  });

  it('cyl: same params return the same instance', () => {
    const a = cyl(0.1, 0.2, 1.5, 12);
    const b = cyl(0.1, 0.2, 1.5, 12);
    expect(a).toBe(b);
  });

  it('cyl: different segment counts return different instances', () => {
    const a = cyl(0.1, 0.2, 1.5, 12);
    const b = cyl(0.1, 0.2, 1.5, 20);
    expect(a).not.toBe(b);
  });

  it('sph: same params return the same instance', () => {
    const a = sph(0.3, 16);
    const b = sph(0.3, 16);
    expect(a).toBe(b);
  });

  it('sph: different radius returns a different instance', () => {
    const a = sph(0.3, 16);
    const b = sph(0.35, 16);
    expect(a).not.toBe(b);
  });

  it('tor: same params return the same instance', () => {
    const a = tor(0.3, 0.05, 12, 32, Math.PI * 2);
    const b = tor(0.3, 0.05, 12, 32, Math.PI * 2);
    expect(a).toBe(b);
  });

  it('tor: different arc returns a different instance', () => {
    const a = tor(0.3, 0.05, 12, 32, Math.PI * 2);
    const b = tor(0.3, 0.05, 12, 32, Math.PI);
    expect(a).not.toBe(b);
  });

  it('boxGeo: same params return the same instance', () => {
    const a = boxGeo(1, 2, 3);
    const b = boxGeo(1, 2, 3);
    expect(a).toBe(b);
  });

  it('boxGeo: different params return different instances', () => {
    const a = boxGeo(1, 2, 3);
    const b = boxGeo(1, 2, 3.001);
    expect(a).not.toBe(b);
  });

  it('different geometry kinds with identical numeric params never collide', () => {
    const b = boxGeo(1, 2, 3);
    const c = cyl(1, 2, 3, 0);
    expect(b).not.toBe(c);
  });
});
