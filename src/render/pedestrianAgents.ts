export type PedestrianAgentCoord = {
  x: number;
  y: number;
};

export type PedestrianAgentSource = {
  path: PedestrianAgentCoord[];
  offset: number;
  speed: number;
  laneOffset: number;
  sprite: {
    sheet: string;
  };
};

export type LocalPedestrianAgent = {
  id: string;
  kind: 'pedestrian';
  state: 'walking';
  coord: PedestrianAgentCoord;
  pathIndex: number;
  nextCoord: PedestrianAgentCoord;
  speed: number;
  laneOffset: number;
  spriteSheet: string;
};

export function pedestrianAgentId(index: number): string {
  return `agent:pedestrian:${index}`;
}

export function buildPedestrianAgents(pedestrians: readonly PedestrianAgentSource[]): LocalPedestrianAgent[] {
  return pedestrians
    .filter((pedestrian) => pedestrian.path.length > 0)
    .map((pedestrian, index) => {
      const pathIndex = normalizedPathIndex(pedestrian);
      const nextCoord = pedestrian.path[(pathIndex + 1) % pedestrian.path.length];
      return {
        id: pedestrianAgentId(index),
        kind: 'pedestrian',
        state: 'walking',
        coord: pedestrianPosition(pedestrian, pathIndex),
        pathIndex,
        nextCoord,
        speed: pedestrian.speed,
        laneOffset: pedestrian.laneOffset,
        spriteSheet: pedestrian.sprite.sheet,
      };
    });
}

export function findNearestPedestrianAgent(
  agents: readonly LocalPedestrianAgent[],
  point: PedestrianAgentCoord,
  project: (coord: PedestrianAgentCoord) => PedestrianAgentCoord,
  radius: number,
): LocalPedestrianAgent | null {
  let nearest: { agent: LocalPedestrianAgent; distance: number } | null = null;
  for (const agent of agents) {
    const projected = project(agent.coord);
    const distance = Math.hypot(projected.x - point.x, projected.y - point.y);
    if (distance > radius) continue;
    if (!nearest || distance < nearest.distance) nearest = { agent, distance };
  }
  return nearest?.agent ?? null;
}

function normalizedPathIndex(pedestrian: PedestrianAgentSource): number {
  const base = Math.floor(pedestrian.offset);
  return ((base % pedestrian.path.length) + pedestrian.path.length) % pedestrian.path.length;
}

function pedestrianPosition(pedestrian: PedestrianAgentSource, pathIndex: number): PedestrianAgentCoord {
  const next = (pathIndex + 1) % pedestrian.path.length;
  const t = pedestrian.offset - Math.floor(pedestrian.offset);
  return {
    x: lerp(pedestrian.path[pathIndex].x, pedestrian.path[next].x, t),
    y: lerp(pedestrian.path[pathIndex].y, pedestrian.path[next].y, t),
  };
}

function lerp(start: number, end: number, t: number): number {
  return start + (end - start) * t;
}
