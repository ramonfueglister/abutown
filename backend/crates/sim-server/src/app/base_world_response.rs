use serde::Serialize;
use sim_core::base_world::BaseWorldBundle;

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldResponse {
    pub schema_version: u32,
    pub world_id: String,
    pub chunk_size: u16,
    pub world_tiles: sim_core::city_network::WorldTiles,
    pub terrain: BaseWorldTerrainResponse,
    pub transport: BaseWorldTransportResponse,
    pub buildings: BaseWorldBuildingResponse,
    pub decorations: BaseWorldDecorationResponse,
    pub markets: BaseWorldMarketLayerResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldTerrainResponse {
    pub tiles: Vec<BaseWorldTerrainTileResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldTerrainTileResponse {
    pub x: i32,
    pub y: i32,
    pub kind: sim_core::base_world::TerrainKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldTransportResponse {
    pub roads: Vec<sim_core::base_world::RoadTile>,
    pub rails: Vec<sim_core::base_world::RailTile>,
    pub arterial_paths: Vec<sim_core::base_world::TransportPath>,
    pub rail_paths: Vec<sim_core::base_world::TransportPath>,
    pub pedestrian_corridors: Vec<sim_core::base_world::TransportPath>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldBuildingResponse {
    pub footprints: Vec<sim_core::base_world::BuildingFootprint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldDecorationResponse {
    pub trees: Vec<sim_core::city_network::NetworkCoord>,
    pub details: Vec<sim_core::base_world::DecorationDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseWorldMarketLayerResponse {
    pub markets: Vec<sim_core::base_world::MarketSpec>,
    pub distances: Vec<sim_core::base_world::MarketDistanceSpec>,
}

impl From<&BaseWorldBundle> for BaseWorldResponse {
    fn from(bundle: &BaseWorldBundle) -> Self {
        Self {
            schema_version: bundle.manifest.schema_version,
            world_id: bundle.world_id().to_owned(),
            chunk_size: bundle.chunk_size(),
            world_tiles: bundle.world_tiles(),
            terrain: BaseWorldTerrainResponse {
                tiles: bundle
                    .terrain
                    .tiles
                    .iter()
                    .map(|tile| BaseWorldTerrainTileResponse {
                        x: tile.x,
                        y: tile.y,
                        kind: tile.kind,
                    })
                    .collect(),
            },
            transport: BaseWorldTransportResponse {
                roads: bundle.transport.roads.clone(),
                rails: bundle.transport.rails.clone(),
                arterial_paths: bundle.transport.arterial_paths.clone(),
                rail_paths: bundle.transport.rail_paths.clone(),
                pedestrian_corridors: bundle.transport.pedestrian_corridors.clone(),
            },
            buildings: BaseWorldBuildingResponse {
                footprints: bundle.buildings.footprints.clone(),
            },
            decorations: BaseWorldDecorationResponse {
                trees: bundle.decorations.trees.clone(),
                details: bundle.decorations.details.clone(),
            },
            markets: BaseWorldMarketLayerResponse {
                markets: bundle.markets.markets.clone(),
                distances: bundle.markets.distances.clone(),
            },
        }
    }
}
