import { describe, expect, it } from 'vitest';
import {
  buildPedestrianAgents,
  findNearestPedestrianAgent,
  type PedestrianAgentSource,
} from '../../src/render/pedestrianAgents';

const pedestrians: PedestrianAgentSource[] = [
  {
    path: [{ x: 10, y: 12 }, { x: 14, y: 12 }, { x: 14, y: 16 }],
    offset: 0.5,
    speed: 0.9,
    laneOffset: -0.2,
    sprite: { sheet: 'pedestrians-1' },
  },
  {
    path: [{ x: 40, y: 8 }, { x: 40, y: 12 }],
    offset: 1.25,
    speed: 1.1,
    laneOffset: 0.4,
    sprite: { sheet: 'pedestrians-2' },
  },
];

describe('pedestrian agents', () => {
  it('projects existing rendered pedestrians into stable local agents', () => {
    const agents = buildPedestrianAgents(pedestrians);

    expect(agents).toEqual([
      {
        id: 'agent:pedestrian:0',
        kind: 'pedestrian',
        state: 'walking',
        coord: { x: 12, y: 12 },
        pathIndex: 0,
        nextCoord: { x: 14, y: 12 },
        speed: 0.9,
        laneOffset: -0.2,
        spriteSheet: 'pedestrians-1',
      },
      {
        id: 'agent:pedestrian:1',
        kind: 'pedestrian',
        state: 'walking',
        coord: { x: 40, y: 11 },
        pathIndex: 1,
        nextCoord: { x: 40, y: 8 },
        speed: 1.1,
        laneOffset: 0.4,
        spriteSheet: 'pedestrians-2',
      },
    ]);
  });

  it('finds the nearest projected pedestrian agent inside the click radius', () => {
    const agents = buildPedestrianAgents(pedestrians);
    const hit = findNearestPedestrianAgent(agents, { x: 25, y: 25 }, (coord) => ({ x: coord.x * 2, y: coord.y * 2 }), 4);

    expect(hit?.id).toBe('agent:pedestrian:0');
  });

  it('returns null when no pedestrian agent is close enough to the click', () => {
    const agents = buildPedestrianAgents(pedestrians);
    const hit = findNearestPedestrianAgent(agents, { x: 25, y: 25 }, (coord) => ({ x: coord.x * 2, y: coord.y * 2 }), 1);

    expect(hit).toBeNull();
  });
});
