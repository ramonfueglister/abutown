import type { LocalPedestrianAgent } from './pedestrianAgents';

export type PedestrianAgentInspectorRow = {
  label: string;
  value: string;
};

export type PedestrianAgentInspector = {
  title: string;
  rows: PedestrianAgentInspectorRow[];
};

export function buildPedestrianAgentInspector(agent: LocalPedestrianAgent | null): PedestrianAgentInspector | null {
  if (!agent) return null;
  return {
    title: agent.id,
    rows: [
      { label: 'State', value: agent.state },
      { label: 'Tile', value: formatCoord(agent.coord) },
      { label: 'Next', value: formatCoord(agent.nextCoord) },
      { label: 'Speed', value: agent.speed.toFixed(2) },
      { label: 'Sprite', value: agent.spriteSheet },
    ],
  };
}

function formatCoord(coord: { x: number; y: number }): string {
  return `${coord.x.toFixed(1)}, ${coord.y.toFixed(1)}`;
}
