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
  readonly markets?: {
    readonly markets: readonly BaseWorldMarket[];
    readonly distances: readonly BaseWorldMarketDistance[];
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

export type BaseWorldMarket = {
  readonly id: number;
  readonly name: string;
  readonly anchor: readonly [number, number];
};

export type BaseWorldMarketDistance = {
  readonly from: number;
  readonly to: number;
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
  // schema_version 4 compacts Abutopia into the first-screen schematic city.
  if (payload.schema_version !== 4) throw new Error(`Unexpected base world schema: ${payload.schema_version}`);
  if (payload.chunk_size !== 32) throw new Error(`Unexpected base world chunk size: ${payload.chunk_size}`);
  if (payload.world_tiles.width !== 80 || payload.world_tiles.height !== 48) {
    throw new Error('Unexpected base world dimensions');
  }
  if (payload.transport.roads.length !== 52) throw new Error('Base world roads layer is incomplete');
  if (payload.transport.rails.length !== 0) throw new Error('Base world rails layer is incomplete');
  if (payload.transport.arterial_paths.length !== 0) throw new Error('Base world arterial layer is incomplete');
  if (payload.transport.pedestrian_corridors.length !== 4) throw new Error('Base world pedestrian layer is incomplete');
  const pedestrianIds = payload.transport.pedestrian_corridors.map((path) => path.id);
  for (const corridorId of ['corridor:edge:north', 'corridor:edge:east', 'corridor:edge:south', 'corridor:edge:west']) {
    if (!pedestrianIds.includes(corridorId)) {
      throw new Error('Base world pedestrian edge corridors are incomplete');
    }
  }
  if (payload.transport.pedestrian_corridors.some((path) => path.points.some((point) => point.x !== 8 && point.x !== 72 && point.y !== 8 && point.y !== 40))) {
    throw new Error('Base world pedestrian corridors must stay on the authored edge frame');
  }
  if (payload.buildings.footprints.length !== 10) throw new Error('Base world buildings layer is incomplete');
  const retiredBuildingSheet = ['old', 'houses'].join('');
  if (payload.buildings.footprints.some((building) => building.sheet === retiredBuildingSheet)) {
    throw new Error('Base world contains retired building sheet');
  }
  if (payload.decorations.trees.length !== 0) throw new Error('Base world trees layer is incomplete');
  if (payload.markets) {
    if (payload.markets.markets.length !== 4) throw new Error('Base world market sites are incomplete');
    if (payload.markets.distances.length !== 3) throw new Error('Base world market guide edges are incomplete');
    for (const market of payload.markets.markets) {
      if (
        typeof market.id !== 'number' ||
        typeof market.name !== 'string' ||
        !Array.isArray(market.anchor) ||
        market.anchor.length !== 2 ||
        !market.anchor.every(Number.isFinite)
      ) {
        throw new Error('Base world market site is invalid');
      }
    }
  }
}
