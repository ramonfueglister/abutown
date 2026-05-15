export type AssetRole =
  | 'terrain.grass'
  | 'terrain.water'
  | 'terrain.riverbank'
  | 'road.straight'
  | 'road.curve'
  | 'road.intersection'
  | 'road.bridge'
  | 'rail.straight'
  | 'rail.station'
  | 'building.residential.low'
  | 'building.commercial.mid'
  | 'building.civic'
  | 'building.industrial'
  | 'vegetation.tree'
  | 'detail.park'
  | 'detail.industry'
  | 'detail.dock'
  | 'detail.quay'
  | 'vehicle.bus'
  | 'vehicle.truck'
  | 'vehicle.delivery.van'
  | 'vehicle.cooling.truck'
  | 'vehicle.tanker'
  | 'vehicle.concrete.mixer'
  | 'vehicle.bulk.truck'
  | 'vehicle.car.transporter'
  | 'vehicle.train.engine'
  | 'vehicle.train.wagon'
  | 'agent.pedestrian';

export type Rect = { x: number; y: number; width: number; height: number };
export type Point = { x: number; y: number };
export type CleanupPolicy = 'none' | 'pak128';

export type AssetProvenance = {
  sourcePath: string;
  datPath?: string;
  license: 'Artistic-2.0';
  revision: string;
};

export type AssetFrame = {
  role: AssetRole;
  path: string;
  source: Rect;
  anchor: Point;
  baseline: number;
  scale: number;
  cleanup: CleanupPolicy;
  provenance: AssetProvenance;
  direction?: 'N' | 'NE' | 'E' | 'SE' | 'S' | 'SW' | 'W' | 'NW';
  variant?: string;
};

export type AssetPackDefinition = {
  id: string;
  tile: { width: number; height: number };
  assets: AssetFrame[];
};

export type AssetPack = {
  id: string;
  tile: { width: number; height: number };
  all: () => AssetFrame[];
  resolve: (role: AssetRole) => AssetFrame | undefined;
  require: (role: AssetRole) => AssetFrame;
};

export function missingAssetRoleError(packId: string, role: AssetRole): string {
  return `Asset pack ${packId} does not define required role ${role}`;
}

export function createAssetPack(definition: AssetPackDefinition): AssetPack {
  const assetsByRole = new Map<AssetRole, AssetFrame>();
  for (const asset of definition.assets) assetsByRole.set(asset.role, asset);

  return {
    id: definition.id,
    tile: { ...definition.tile },
    all: () => [...definition.assets],
    resolve: (role) => assetsByRole.get(role),
    require: (role) => {
      const asset = assetsByRole.get(role);
      if (!asset) throw new Error(missingAssetRoleError(definition.id, role));
      return asset;
    },
  };
}
