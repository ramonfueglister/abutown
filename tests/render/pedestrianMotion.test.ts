import { describe, expect, it } from 'vitest';
import { buildPedestrianLoop, pedestrianWalkingSpeed } from '../../src/render/pedestrianMotion';

describe('pedestrian motion', () => {
  it('closes back-and-forth paths without a teleporting wrap step', () => {
    const loop = buildPedestrianLoop([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
      { x: 2, y: 1 },
    ]);

    for (let index = 0; index < loop.length; index += 1) {
      const current = loop[index];
      const next = loop[(index + 1) % loop.length];
      expect(Math.abs(next.x - current.x) + Math.abs(next.y - current.y)).toBe(1);
    }
  });

  it('keeps all pedestrian speeds at walking pace', () => {
    const speeds = Array.from({ length: 32 }, (_, index) => pedestrianWalkingSpeed(index));

    expect(Math.min(...speeds)).toBeGreaterThanOrEqual(0.14);
    expect(Math.max(...speeds)).toBeLessThanOrEqual(0.22);
  });
});
