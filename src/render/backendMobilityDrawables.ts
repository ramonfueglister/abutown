import type { MobilityOverlayState } from '../backend/mobilityState';
import type { DirectionDto } from '../backend/mobilityProtocol';

export type Coord = { x: number; y: number };

export type SimutransPedestrianSpriteLike = {
  sheet: string;
  frameWidth?: number;
  frameHeight?: number;
};

export type VehicleSpriteLike = {
  sheet: string;
  frameWidth?: number;
  frameHeight?: number;
  scale?: number;
  role: string;
};

export type BackendPedestrian = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  laneOffset: number;
  sprite: SimutransPedestrianSpriteLike;
  direction: DirectionDto;
};

export type BackendCar = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSpriteLike;
  direction: DirectionDto;
};

const DIRECTION_VECTORS: Record<DirectionDto, Coord> = {
  n: { x: 0, y: -1 },
  ne: { x: 1, y: -1 },
  e: { x: 1, y: 0 },
  se: { x: 1, y: 1 },
  s: { x: 0, y: 1 },
  sw: { x: -1, y: 1 },
  w: { x: -1, y: 0 },
  nw: { x: -1, y: -1 },
};

function spriteIndexFromKey(key: string, modulus: number): number {
  const parts = key.split(':');
  const last = parts[parts.length - 1] ?? '0';
  const n = Number.parseInt(last, 10);
  if (Number.isNaN(n)) return 0;
  return ((n % modulus) + modulus) % modulus;
}

function syntheticPath(start: Coord, direction: DirectionDto): Coord[] {
  const vec = DIRECTION_VECTORS[direction];
  return [start, { x: start.x + vec.x, y: start.y + vec.y }];
}

export function pedestriansFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly SimutransPedestrianSpriteLike[],
): BackendPedestrian[] {
  if (sprites.length === 0) return [];
  const out: BackendPedestrian[] = [];
  // Deterministic order by id so tests and the renderer see stable input each frame.
  const agents = Array.from(state.agents.values())
    .map((entry) => entry.current)
    .sort((a, b) => a.id.localeCompare(b.id));
  for (const agent of agents) {
    const sprite = sprites[spriteIndexFromKey(agent.sprite_key, sprites.length)];
    out.push({
      id: agent.id,
      path: syntheticPath(agent.world_coord, agent.direction),
      offset: 0,
      speed: 0,
      laneOffset: 0,
      sprite,
      direction: agent.direction,
    });
  }
  return out;
}

export function carsFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly VehicleSpriteLike[],
): BackendCar[] {
  if (sprites.length === 0) return [];
  const out: BackendCar[] = [];
  const vehicles = Array.from(state.roadVehicles.vehicles.values())
    .map((entry) => entry.current)
    .sort((a, b) => a.id.localeCompare(b.id));
  for (const vehicle of vehicles) {
    const sprite = sprites[spriteIndexFromKey(vehicle.sprite_key, sprites.length)];
    out.push({
      id: vehicle.id,
      path: syntheticPath(vehicle.world_coord, vehicle.direction),
      offset: 0,
      speed: 0,
      sprite,
      direction: vehicle.direction,
    });
  }
  return out;
}
