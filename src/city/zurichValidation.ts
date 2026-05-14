import { key, type ZurichValidationResult, type ZurichWorld } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

export function validateZurichCity(world: ZurichWorld, transport: ZurichTransport, placement: ZurichPlacement): ZurichValidationResult {
  const errors: string[] = [];
  let roadRailOverlap = 0;
  let invalidBuildings = 0;
  let bridgeErrors = 0;

  for (const roadKey of transport.roads.keys()) {
    if (transport.rails.has(roadKey) && !transport.railCrossings.has(roadKey)) roadRailOverlap += 1;
  }

  for (const bridgeKey of transport.bridges) {
    const terrain = world.terrain.get(bridgeKey)?.kind;
    if (terrain !== 'water' && terrain !== 'riverbank') bridgeErrors += 1;
  }

  for (const building of placement.buildings) {
    const tileKey = key(building.coord);
    const terrain = world.terrain.get(tileKey)?.kind;
    if (terrain === 'water' || transport.roads.has(tileKey) || transport.rails.has(tileKey)) invalidBuildings += 1;
  }

  if (roadRailOverlap > 0) errors.push(`roadRailOverlap:${roadRailOverlap}`);
  if (bridgeErrors > 0) errors.push(`bridgeErrors:${bridgeErrors}`);
  if (invalidBuildings > 0) errors.push(`invalidBuildings:${invalidBuildings}`);

  return {
    valid: errors.length === 0,
    errors,
    stats: {
      roadTiles: transport.roads.size,
      railTiles: transport.rails.size,
      bridges: transport.bridges.size,
      railCrossings: transport.railCrossings.size,
      buildings: placement.buildings.length,
      trees: placement.trees.length,
      details: placement.details.length,
      reserveTiles: placement.reserveTiles.size,
      roadRailOverlap,
      bridgeErrors,
      invalidBuildings,
    },
  };
}
