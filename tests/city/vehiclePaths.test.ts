import { describe, expect, it } from 'vitest';
import { hasTeleportingVehicleSegment, makeNonDespawningVehicleLoop } from '../../src/city/vehiclePaths';

describe('vehicle paths', () => {
  it('turns an open road corridor into a loop without a despawn jump', () => {
    const loop = makeNonDespawningVehicleLoop([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
    ]);

    expect(loop).toEqual([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
      { x: 1, y: 0 },
    ]);
    expect(hasTeleportingVehicleSegment(loop)).toBe(false);
  });

  it('detects modulo wrap jumps that would make cars disappear and reappear', () => {
    expect(hasTeleportingVehicleSegment([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 12, y: 0 },
    ])).toBe(true);
  });
});
