import { describe, expect, it } from 'vitest';
import {
  buildVehicleRoadLoops,
  hasIllegalVehicleUTurn,
  hasTeleportingVehicleSegment,
  makeNonDespawningVehicleLoop,
  splitVehicleRoadSegments,
} from '../../src/city/vehiclePaths';

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

  it('splits requested corridors into road-only segments and removes rail tiles', () => {
    const roadKeys = new Set(['0:0', '1:0', '2:0', '4:0', '5:0', '6:0']);
    const railKeys = new Set(['2:0']);

    expect(splitVehicleRoadSegments([
      { x: 0, y: 0 },
      { x: 1, y: 0 },
      { x: 2, y: 0 },
      { x: 3, y: 0 },
      { x: 4, y: 0 },
      { x: 5, y: 0 },
      { x: 6, y: 0 },
    ], { roadKeys, railKeys })).toEqual([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
      ],
      [
        { x: 4, y: 0 },
        { x: 5, y: 0 },
        { x: 6, y: 0 },
      ],
    ]);
  });

  it('builds vehicle loops only from road tiles that are not rail tiles', () => {
    const roadKeys = new Set(['0:0', '1:0', '4:0', '5:0', '6:0']);
    const railKeys = new Set(['2:0']);

    const loops = buildVehicleRoadLoops([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
        { x: 3, y: 0 },
        { x: 4, y: 0 },
        { x: 5, y: 0 },
        { x: 6, y: 0 },
      ],
    ], { roadKeys, railKeys });

    expect(loops).toEqual([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
      ],
      [
        { x: 4, y: 0 },
        { x: 5, y: 0 },
        { x: 6, y: 0 },
        { x: 5, y: 0 },
      ],
    ]);
    expect(loops.every((loop) => !hasTeleportingVehicleSegment(loop))).toBe(true);
    expect(loops.flat().every((coord) => roadKeys.has(`${coord.x}:${coord.y}`))).toBe(true);
    expect(loops.flat().every((coord) => !railKeys.has(`${coord.x}:${coord.y}`))).toBe(true);
  });

  it('closes connected corridors through the road graph instead of making sudden u-turns', () => {
    const roadKeys = new Set([
      '0:0', '1:0', '2:0',
      '0:1', '2:1',
      '0:2', '1:2', '2:2',
    ]);

    const loops = buildVehicleRoadLoops([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
      ],
    ], { roadKeys });

    expect(loops).toEqual([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
        { x: 2, y: 1 },
        { x: 2, y: 2 },
        { x: 1, y: 2 },
        { x: 0, y: 2 },
        { x: 0, y: 1 },
      ],
    ]);
    expect(loops.every((loop) => !hasTeleportingVehicleSegment(loop))).toBe(true);
    expect(loops.every((loop) => !hasIllegalVehicleUTurn(loop, { roadKeys }))).toBe(true);
  });

  it('still allows u-turns on true dead-end road tiles', () => {
    const roadKeys = new Set(['0:0', '1:0', '2:0']);

    const loops = buildVehicleRoadLoops([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
      ],
    ], { roadKeys });

    expect(loops).toEqual([
      [
        { x: 0, y: 0 },
        { x: 1, y: 0 },
        { x: 2, y: 0 },
        { x: 1, y: 0 },
      ],
    ]);
    expect(loops.every((loop) => !hasIllegalVehicleUTurn(loop, { roadKeys }))).toBe(true);
  });
});
