import type { Agent, AgentKind, AgentPopulation, AgentRole, City, RoadEdge } from '../types';

type GenerateAgentsOptions = {
  count: number;
  seed: number;
};

export function generateAgents(city: City, options: GenerateAgentsOptions): AgentPopulation {
  if (options.count === 0) return emptyPopulation();

  const random = seededRandom(options.seed);
  const eligibleRoads = city.roadEdges.filter((edge) => edge.points.length > 1 && edge.modes.includes('pedestrian'));
  if (eligibleRoads.length === 0) {
    throw new Error('Cannot generate agents without eligible pedestrian road edges');
  }

  const weightedRoads = weightRoads(eligibleRoads);
  const agents: Agent[] = [];
  for (let index = 0; index < options.count; index += 1) {
    const roadEdge = pickWeightedRoad(weightedRoads, random());
    const kind = chooseKind(roadEdge, random());
    const role = chooseRole(kind, random());
    agents.push({
      id: `agent:${options.seed}:${index}`,
      kind,
      role,
      roadEdgeId: roadEdge.id,
      progress: random(),
      laneOffset: Number(((random() - 0.5) * (kind === 'vehicle' ? 0.42 : 0.26)).toFixed(3)),
      speedTilesPerSecond: speedFor(kind, role, random()),
      colorIndex: Math.floor(random() * 8),
    });
  }

  const segmentBuckets = new Map<string, Agent[]>();
  for (const agent of agents) {
    const bucket = segmentBuckets.get(agent.roadEdgeId) ?? [];
    bucket.push(agent);
    segmentBuckets.set(agent.roadEdgeId, bucket);
  }

  return {
    agents,
    segmentBuckets,
    stats: {
      totalAgents: agents.length,
      pedestrians: agents.filter((agent) => agent.kind === 'pedestrian').length,
      vehicles: agents.filter((agent) => agent.kind === 'vehicle').length,
    },
  };
}

function emptyPopulation(): AgentPopulation {
  return {
    agents: [],
    segmentBuckets: new Map(),
    stats: { totalAgents: 0, pedestrians: 0, vehicles: 0 },
  };
}

function seededRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
}

function weightRoads(roads: RoadEdge[]): Array<{ roadEdge: RoadEdge; cumulativeWeight: number }> {
  let cumulativeWeight = 0;
  return roads.map((roadEdge) => {
    const hierarchyWeight = roadEdge.hierarchy === 'primary' ? 4 : roadEdge.hierarchy === 'secondary' ? 2 : 1;
    cumulativeWeight += Math.max(1, roadEdge.points.length) * hierarchyWeight;
    return { roadEdge, cumulativeWeight };
  });
}

function pickWeightedRoad(weightedRoads: Array<{ roadEdge: RoadEdge; cumulativeWeight: number }>, randomValue: number): RoadEdge {
  const target = randomValue * (weightedRoads.at(-1)?.cumulativeWeight ?? 1);
  return weightedRoads.find((entry) => entry.cumulativeWeight >= target)?.roadEdge ?? weightedRoads[0].roadEdge;
}

function chooseKind(roadEdge: RoadEdge, randomValue: number): AgentKind {
  return roadEdge.modes.includes('car') && randomValue < 0.18 ? 'vehicle' : 'pedestrian';
}

function chooseRole(kind: AgentKind, randomValue: number): AgentRole {
  if (kind === 'vehicle') return randomValue < 0.72 ? 'worker' : 'service';
  if (randomValue < 0.52) return 'resident';
  return randomValue < 0.82 ? 'worker' : 'visitor';
}

function speedFor(kind: AgentKind, role: AgentRole, randomValue: number): number {
  return Number((kind === 'vehicle' ? 2.4 + randomValue * (role === 'service' ? 1.8 : 1.2) : 0.55 + randomValue * 0.65).toFixed(3));
}
