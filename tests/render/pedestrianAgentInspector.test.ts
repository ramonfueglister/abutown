import { describe, expect, it } from 'vitest';
import { buildPedestrianAgentInspector } from '../../src/render/pedestrianAgentInspector';
import type { LocalPedestrianAgent } from '../../src/render/pedestrianAgents';

const agent: LocalPedestrianAgent = {
  id: 'agent:pedestrian:12',
  kind: 'pedestrian',
  state: 'walking',
  coord: { x: 42.25, y: 18.75 },
  pathIndex: 7,
  nextCoord: { x: 43, y: 19 },
  speed: 1.234,
  laneOffset: -0.25,
  spriteSheet: 'pedestrians-2',
};

describe('pedestrian agent inspector', () => {
  it('returns null when no pedestrian agent is selected', () => {
    expect(buildPedestrianAgentInspector(null)).toBeNull();
  });

  it('formats compact rows for the selected pedestrian agent', () => {
    expect(buildPedestrianAgentInspector(agent)).toEqual({
      title: 'agent:pedestrian:12',
      rows: [
        { label: 'State', value: 'walking' },
        { label: 'Tile', value: '42.3, 18.8' },
        { label: 'Next', value: '43.0, 19.0' },
        { label: 'Speed', value: '1.23' },
        { label: 'Sprite', value: 'pedestrians-2' },
      ],
    });
  });
});
