import { projectIso } from '../projection';
import type { City, Coord, GeneratedPopulation, PopulationEntity, RoadEdge } from '../types';

export type AgentRenderQuality = 'ultra-low' | 'standard' | 'high';
export type AgentLod = 'density' | 'pixel' | 'citizen';

export type AgentViewport = {
  stageX: number;
  stageY: number;
  stageScale: number;
  viewportWidth: number;
  viewportHeight: number;
  quality: AgentRenderQuality;
};

export type AgentRenderSample = {
  agentId: string;
  roadEdgeId: string;
  x: number;
  y: number;
  lod: AgentLod;
  colorIndex: number;
  density: number;
};

export type AgentRenderStats = {
  simulatedAgents: number;
  visibleAgents: number;
  renderedSamples: number;
  aggregatedAgents: number;
  culledAgents: number;
  budget: number;
};

export type AgentRenderPlan = {
  samples: AgentRenderSample[];
  stats: AgentRenderStats;
};

type VisibleAgent = {
  entity: PopulationEntity;
  roadEdgeId: string;
  isoX: number;
  isoY: number;
};

export function buildAgentRenderPlan(city: City, population: GeneratedPopulation, viewport: AgentViewport): AgentRenderPlan {
  const roadsById = new Map(city.roadEdges.map((roadEdge) => [roadEdge.id, roadEdge]));
  const budget = renderBudget(viewport);
  const visibleAgents: VisibleAgent[] = [];

  for (const entity of population.entities) {
    const roadEdge = roadsById.get(entity.roadEdgeId);
    if (!roadEdge) continue;
    const point = withLaneOffset(pointOnRoad(roadEdge, entity.progress), entity.laneOffset);
    const iso = projectIso(point);
    const screenX = viewport.stageX + iso.x * viewport.stageScale;
    const screenY = viewport.stageY + iso.y * viewport.stageScale;
    if (isScreenVisible(screenX, screenY, viewport, 48)) {
      visibleAgents.push({ entity, roadEdgeId: roadEdge.id, isoX: iso.x, isoY: iso.y });
    }
  }

  const samples = sampleVisibleAgents(visibleAgents, budget, viewport.stageScale);
  return {
    samples,
    stats: {
      simulatedAgents: population.entities.length,
      visibleAgents: visibleAgents.length,
      renderedSamples: samples.length,
      aggregatedAgents: Math.max(0, visibleAgents.length - samples.length),
      culledAgents: population.entities.length - visibleAgents.length,
      budget,
    },
  };
}

function sampleVisibleAgents(visibleAgents: VisibleAgent[], budget: number, stageScale: number): AgentRenderSample[] {
  if (visibleAgents.length === 0 || budget <= 0) return [];

  const lod = lodForScale(stageScale);
  const sampleCount = Math.min(budget, visibleAgents.length);
  const samples: AgentRenderSample[] = [];
  for (let index = 0; index < sampleCount; index += 1) {
    const start = Math.floor((index * visibleAgents.length) / sampleCount);
    const end = Math.floor(((index + 1) * visibleAgents.length) / sampleCount);
    const visible = visibleAgents[start];
    samples.push({
      agentId: lod === 'density' ? `density:${visible.roadEdgeId}:${start}` : visible.entity.id,
      roadEdgeId: visible.roadEdgeId,
      x: visible.isoX,
      y: visible.isoY - lodYOffset(lod),
      lod,
      colorIndex: visible.entity.colorIndex,
      density: Math.max(1, end - start),
    });
  }
  return samples;
}

function renderBudget(viewport: AgentViewport): number {
  const baseBudget = viewport.quality === 'ultra-low' ? 600 : viewport.quality === 'standard' ? 1400 : 2400;
  const scaleFactor = viewport.stageScale < 0.75 ? 0.12 : viewport.stageScale < 1.25 ? 0.72 : 1;
  return Math.floor(baseBudget * scaleFactor);
}

function lodForScale(stageScale: number): AgentLod {
  if (stageScale >= 1.65) return 'citizen';
  if (stageScale >= 0.9) return 'pixel';
  return 'density';
}

function pointOnRoad(roadEdge: RoadEdge, progress: number): Coord {
  const index = Math.min(roadEdge.points.length - 1, Math.max(0, Math.floor(progress * roadEdge.points.length)));
  return roadEdge.points[index] ?? roadEdge.points[0];
}

function withLaneOffset(coord: Coord, laneOffset: number): Coord {
  return { x: coord.x + laneOffset, y: coord.y - laneOffset };
}

function lodYOffset(lod: AgentLod): number {
  if (lod === 'citizen') return 9;
  return lod === 'pixel' ? 5 : 2;
}

function isScreenVisible(screenX: number, screenY: number, viewport: AgentViewport, margin: number): boolean {
  return screenX >= -margin && screenY >= -margin && screenX <= viewport.viewportWidth + margin && screenY <= viewport.viewportHeight + margin;
}
