import {
  isDirectionDto,
  isNumber,
  isObject,
  isString,
  isWorldCoordDto,
  type DirectionDto,
  type WorldCoordDto,
} from './mobilityProtocol';

export type RoadVehicleDto = {
  id: string;
  world_coord: WorldCoordDto;
  direction: DirectionDto;
  sprite_key: string;
};

export type RoadVehicleSnapshotDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  vehicles: RoadVehicleDto[];
};

export type RoadVehicleDeltaDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  changed: RoadVehicleDto[];
};

export type RoadVehicleDeltaServerMessage = RoadVehicleDeltaDto & {
  type: 'road_vehicle_delta';
};

function isRoadVehicleDto(value: unknown): value is RoadVehicleDto {
  if (!isObject(value)) return false;
  return (
    isString(value.id) &&
    isWorldCoordDto(value.world_coord) &&
    isDirectionDto(value.direction) &&
    isString(value.sprite_key)
  );
}

export function isRoadVehicleSnapshotDto(value: unknown): value is RoadVehicleSnapshotDto {
  if (!isObject(value)) return false;
  return (
    isNumber(value.protocol_version) &&
    isString(value.world_id) &&
    isNumber(value.tick) &&
    Array.isArray(value.vehicles) &&
    value.vehicles.every(isRoadVehicleDto)
  );
}

export function isRoadVehicleDeltaDto(
  value: unknown,
): value is RoadVehicleDeltaDto | RoadVehicleDeltaServerMessage {
  if (!isObject(value)) return false;
  return (
    isNumber(value.protocol_version) &&
    isString(value.world_id) &&
    isNumber(value.tick) &&
    Array.isArray(value.changed) &&
    value.changed.every(isRoadVehicleDto)
  );
}
