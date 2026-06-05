import { resolveBackendBaseUrl } from './backendGate';

export type BaseWorldPoint = {
  readonly x: number;
  readonly y: number;
};

export type BaseWorldTerrainKind =
  | 'grass'
  | 'water'
  | 'riverbank'
  | 'park'
  | 'forest'
  | 'reserve'
  | 'plaza';

export type BaseWorldRoadKind = 'street' | 'bridge';

export type BaseWorldResponse = {
  readonly schema_version: number;
  readonly world_id: string;
  readonly chunk_size: number;
  readonly world_tiles: { readonly width: number; readonly height: number };
  readonly terrain: {
    readonly tiles: readonly (BaseWorldPoint & { readonly kind: BaseWorldTerrainKind })[];
  };
  readonly transport: {
    readonly roads: readonly (BaseWorldPoint & { readonly kind: BaseWorldRoadKind; readonly mask: number })[];
    readonly rails: readonly (BaseWorldPoint & { readonly mask: number })[];
    readonly arterial_paths: readonly BaseWorldPath[];
    readonly rail_paths: readonly BaseWorldPath[];
    readonly pedestrian_corridors: readonly BaseWorldPath[];
  };
  readonly buildings: {
    readonly footprints: readonly BaseWorldBuildingFootprint[];
  };
  readonly decorations: {
    readonly trees: readonly BaseWorldPoint[];
    readonly details: readonly BaseWorldDecorationDetail[];
  };
};

export type BaseWorldPath = {
  readonly id: string;
  readonly points: readonly BaseWorldPoint[];
};

export type BaseWorldBuildingFootprint = {
  readonly id: string;
  readonly tiles: readonly BaseWorldPoint[];
  readonly sheet?: string;
  readonly frame?: number;
  readonly district?: string;
};

export type BaseWorldDecorationDetail = BaseWorldPoint & {
  readonly category: string;
  readonly asset_category: string;
};

export async function requireBaseWorld(options: { baseUrl?: string; fetchImpl?: typeof fetch } = {}): Promise<BaseWorldResponse> {
  const baseUrl = options.baseUrl ?? resolveBackendBaseUrl();
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  if (!fetchImpl) throw new Error('Base world fetch transport unavailable');

  const response = await fetchImpl(new URL('/base-world', baseUrl).toString());
  if (!response.ok) throw new Error(`Base world HTTP ${response.status}`);
  const payload = await response.json() as BaseWorldResponse;
  validateBaseWorld(payload);
  return payload;
}

function validateBaseWorld(payload: BaseWorldResponse): void {
  if (payload.world_id !== 'abutopia') {
    throw new Error(`Unexpected base world id: ${payload.world_id}`);
  }
  // schema_version 2 added the authored markets layer (economy on-map view).
  if (payload.schema_version !== 1 && payload.schema_version !== 2) throw new Error(`Unexpected base world schema: ${payload.schema_version}`);
  if (payload.chunk_size !== 32) throw new Error(`Unexpected base world chunk size: ${payload.chunk_size}`);
  if (payload.world_tiles.width !== 224 || payload.world_tiles.height !== 128) {
    throw new Error('Unexpected base world dimensions');
  }
  if (payload.transport.roads.length !== 10) throw new Error('Base world roads layer is incomplete');
  if (payload.transport.rails.length !== 0) throw new Error('Base world rails layer is incomplete');
  if (payload.transport.arterial_paths.length !== 0) throw new Error('Base world arterial layer is incomplete');
  if (payload.transport.pedestrian_corridors.length !== 2) throw new Error('Base world pedestrian layer is incomplete');
  const pedestrianIds = payload.transport.pedestrian_corridors.map((path) => path.id);
  if (!pedestrianIds.includes('corridor:sidewalk:north') || !pedestrianIds.includes('corridor:sidewalk:south')) {
    throw new Error('Base world pedestrian sidewalks are incomplete');
  }
  if (payload.transport.pedestrian_corridors.some((path) => path.points.some((point) => point.y === 64))) {
    throw new Error('Base world pedestrian sidewalks must not use the road centerline');
  }
  if (payload.buildings.footprints.length !== 2) throw new Error('Base world buildings layer is incomplete');
  if (payload.decorations.trees.length !== 0) throw new Error('Base world trees layer is incomplete');
}
