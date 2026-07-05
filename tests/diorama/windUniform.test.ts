import { describe, expect, it } from 'vitest';
import { windAmplitude } from '../../src/diorama/ksw/windUniform';

describe('windAmplitude', () => {
  it('is 0 in calm air and grows monotonically', () => {
    expect(windAmplitude(0)).toBe(0);
    expect(windAmplitude(3)).toBeGreaterThan(0);
    expect(windAmplitude(8)).toBeGreaterThan(windAmplitude(3));
  });
  it('saturates for storms (cap 1.2)', () => {
    expect(windAmplitude(40)).toBeCloseTo(1.2, 5);
    expect(windAmplitude(15)).toBeLessThanOrEqual(1.2);
  });
});
