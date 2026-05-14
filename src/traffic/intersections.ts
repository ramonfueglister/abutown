import {
  type TrafficCoord,
  type TrafficDirection,
  type TrafficIntersection,
  trafficIntersectionId,
} from './trafficTypes';

export const ROAD_NORTH = 1;
export const ROAD_EAST = 2;
export const ROAD_SOUTH = 4;
export const ROAD_WEST = 8;

export type TrafficRoadTile = {
  coord: TrafficCoord;
  mask: number;
};

export function buildTrafficIntersections(roads: Iterable<TrafficRoadTile>): TrafficIntersection[] {
  return [...roads]
    .filter((road) => roadDegree(road.mask) >= 3)
    .map((road) => ({
      intersectionId: trafficIntersectionId(road.coord),
      coord: { x: road.coord.x, y: road.coord.y },
      connectedDirections: connectedDirections(road.mask),
    }))
    .sort((a, b) => a.coord.y - b.coord.y || a.coord.x - b.coord.x);
}

export function directionForRoadStep(from: TrafficCoord, to: TrafficCoord): TrafficDirection | undefined {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  if (dx === 0 && dy === 1) return 'north';
  if (dx === -1 && dy === 0) return 'east';
  if (dx === 0 && dy === -1) return 'south';
  if (dx === 1 && dy === 0) return 'west';
  return undefined;
}

function connectedDirections(mask: number): TrafficDirection[] {
  const result: TrafficDirection[] = [];
  if ((mask & ROAD_NORTH) !== 0) result.push('north');
  if ((mask & ROAD_EAST) !== 0) result.push('east');
  if ((mask & ROAD_SOUTH) !== 0) result.push('south');
  if ((mask & ROAD_WEST) !== 0) result.push('west');
  return result;
}

function roadDegree(mask: number): number {
  return [ROAD_NORTH, ROAD_EAST, ROAD_SOUTH, ROAD_WEST]
    .filter((direction) => (mask & direction) !== 0)
    .length;
}
