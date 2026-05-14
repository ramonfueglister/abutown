import { describe, expect, it } from 'vitest';
import {
  ROAD_EAST,
  ROAD_NORTH,
  ROAD_SOUTH,
  ROAD_WEST,
  buildTrafficIntersections,
  directionForRoadStep,
} from '../../src/traffic/intersections';

describe('traffic intersections', () => {
  it('creates deterministic intersection ids for road nodes with degree three or higher', () => {
    const intersections = buildTrafficIntersections([
      { coord: { x: 4, y: 5 }, mask: ROAD_NORTH | ROAD_EAST | ROAD_SOUTH },
      { coord: { x: 1, y: 2 }, mask: ROAD_EAST | ROAD_WEST },
      { coord: { x: 8, y: 9 }, mask: ROAD_NORTH | ROAD_EAST | ROAD_SOUTH | ROAD_WEST },
    ]);

    expect(intersections).toEqual([
      {
        intersectionId: 'intersection:4:5',
        coord: { x: 4, y: 5 },
        connectedDirections: ['north', 'east', 'south'],
      },
      {
        intersectionId: 'intersection:8:9',
        coord: { x: 8, y: 9 },
        connectedDirections: ['north', 'east', 'south', 'west'],
      },
    ]);
  });

  it('classifies route steps into approach directions', () => {
    expect(directionForRoadStep({ x: 2, y: 1 }, { x: 2, y: 2 })).toBe('north');
    expect(directionForRoadStep({ x: 3, y: 2 }, { x: 2, y: 2 })).toBe('east');
    expect(directionForRoadStep({ x: 2, y: 3 }, { x: 2, y: 2 })).toBe('south');
    expect(directionForRoadStep({ x: 1, y: 2 }, { x: 2, y: 2 })).toBe('west');
  });
});
