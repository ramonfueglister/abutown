import { inside, key, type ZurichValidationResult, type ZurichWorld } from './worldTypes';
import type { ZurichPlacement } from './zurichPlacement';
import type { ZurichTransport } from './zurichTransport';

export function validateZurichCity(world: ZurichWorld, transport: ZurichTransport, placement: ZurichPlacement): ZurichValidationResult {
  const errors: string[] = [];
  let roadRailOverlap = 0;
  let invalidBuildings = 0;
  let bridgeErrors = 0;
  let treeBuildingOverlap = 0;

  for (const roadKey of transport.roads.keys()) {
    if (transport.rails.has(roadKey) && !transport.railCrossings.has(roadKey)) roadRailOverlap += 1;
  }

  for (const bridgeKey of transport.bridges) {
    const terrain = world.terrain.get(bridgeKey)?.kind;
    if (terrain !== 'water' && terrain !== 'riverbank') bridgeErrors += 1;
  }

  for (const building of placement.buildings) {
    const tileKey = key(building.coord);
    const terrain = world.terrain.get(tileKey);
    if (!inside(building.coord, world.width, world.height) || !terrain || terrain.kind === 'water' || transport.roads.has(tileKey) || transport.rails.has(tileKey)) {
      invalidBuildings += 1;
    }
  }

  const buildingTiles = new Set(placement.buildings.map((building) => key(building.coord)));
  for (const tree of placement.trees) {
    if (buildingTiles.has(key(tree))) treeBuildingOverlap += 1;
  }

  if (roadRailOverlap > 0) errors.push(`roadRailOverlap:${roadRailOverlap}`);
  if (bridgeErrors > 0) errors.push(`bridgeErrors:${bridgeErrors}`);
  if (invalidBuildings > 0) errors.push(`invalidBuildings:${invalidBuildings}`);
  if (treeBuildingOverlap > 0) errors.push(`treeBuildingOverlap:${treeBuildingOverlap}`);

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
      treeBuildingOverlap,
    },
  };
}
