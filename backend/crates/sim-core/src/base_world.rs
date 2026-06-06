use crate::city_network::{CityNetwork, NetworkCoord, NetworkPoint, WorldTiles};
use crate::ids::ChunkCoord;
use crate::persistence::SnapshotCompatibility;
use crate::scheduler::ChunkActivity;
use crate::tile::{TileKind, TileRecord};
use crate::world::systems::spawn_chunk_entity;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum BaseWorldError {
    #[error("base world manifest missing at {0}")]
    MissingManifest(PathBuf),
    #[error("failed to read base world file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse base world file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported base world schema version {0}")]
    UnsupportedSchema(u32),
    #[error("base world id mismatch: manifest has {manifest}, layer has {layer}")]
    WorldIdMismatch { manifest: String, layer: String },
    #[error("base world layer {0} is empty")]
    EmptyLayer(&'static str),
    #[error("base world coordinate {x},{y} is outside {width}x{height}")]
    OutOfBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    #[error("base world spawn group {group} references missing arterial {arterial_id}")]
    MissingArterialRef { group: String, arterial_id: String },
    #[error("base world spawn group {group} references missing corridor {corridor_id}")]
    MissingCorridorRef { group: String, corridor_id: String },
    #[error("base world dimensions {width}x{height} are too large to index")]
    WorldTooLarge { width: u32, height: u32 },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BaseWorldManifest {
    pub schema_version: u32,
    pub world_id: String,
    pub display_name: String,
    pub chunk_size: u16,
    pub world_tiles: WorldTiles,
    pub layers: BaseWorldLayerFiles,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BaseWorldLayerFiles {
    pub terrain: String,
    pub transport: String,
    pub buildings: String,
    pub decorations: String,
    pub spawns: String,
    pub markets: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TerrainLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub tiles: Vec<TerrainTile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TerrainTile {
    pub x: i32,
    pub y: i32,
    pub kind: TerrainKind,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TerrainKind {
    Grass,
    Water,
    Riverbank,
    Park,
    Forest,
    Reserve,
    Plaza,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub roads: Vec<RoadTile>,
    pub rails: Vec<RailTile>,
    pub arterial_paths: Vec<TransportPath>,
    pub rail_paths: Vec<TransportPath>,
    pub pedestrian_corridors: Vec<TransportPath>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoadTile {
    pub x: i32,
    pub y: i32,
    pub kind: String,
    pub mask: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RailTile {
    pub x: i32,
    pub y: i32,
    pub mask: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportPath {
    pub id: String,
    pub points: Vec<NetworkPoint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildingLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub footprints: Vec<BuildingFootprint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildingFootprint {
    pub id: String,
    pub tiles: Vec<NetworkCoord>,
    #[serde(default)]
    pub sheet: Option<String>,
    #[serde(default)]
    pub frame: Option<u32>,
    #[serde(default)]
    pub district: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpawnLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub pedestrian_groups: Vec<PedestrianSpawnGroup>,
    pub car_groups: Vec<CarSpawnGroup>,
    pub tram_lines: Vec<TramLineSpawn>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DecorationLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub trees: Vec<NetworkCoord>,
    pub details: Vec<DecorationDetail>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DecorationDetail {
    pub x: i32,
    pub y: i32,
    pub category: String,
    pub asset_category: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PedestrianSpawnGroup {
    pub id: String,
    pub corridor_id: String,
    pub agents_per_corridor: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CarSpawnGroup {
    pub id: String,
    pub arterial_id: String,
    pub cars_per_arterial: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TramLineSpawn {
    pub id: String,
    pub rail_path_ids: Vec<String>,
    pub trams: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketLayer {
    pub schema_version: u32,
    pub world_id: String,
    pub markets: Vec<MarketSpec>,
    pub distances: Vec<MarketDistanceSpec>,
    pub supply: Vec<SupplySpec>,
    pub demand: Vec<DemandSpec>,
    pub extractors: Vec<ExtractorSpec>,
    pub household: HouseholdSpec,
    pub opening_prices: Vec<OpeningPriceSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketSpec {
    pub id: u32,
    pub name: String,
    pub anchor: [f32; 2],
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketDistanceSpec {
    pub from: u32,
    pub to: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SupplySpec {
    pub actor: u64,
    pub market: u32,
    pub good: u16,
    pub qty: i64,
    pub min_price: i64,
    pub opening_inventory: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DemandSpec {
    pub actor: u64,
    pub market: u32,
    pub good: u16,
    pub qty: i64,
    pub max_price: i64,
    pub mpc_bps: i32,
    pub autonomous: i64,
    pub opening_cash: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractorSpec {
    pub actor: u64,
    pub market: u32,
    pub in_good: u16,
    pub out_good: u16,
    pub qty: i64,
    pub min_price: i64,
}

fn default_capita_baseline() -> i64 {
    1_000_000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HouseholdSpec {
    pub population: u64,
    /// Per-capita scaling baseline (`capita_factor = max(1, live_count / capita_baseline)`).
    /// Authored per world; LOWER it to ramp economic throughput + visible density up
    /// (e.g. 10 → ~30x at 300 citizens). Defaults to 1_000_000 = identity for worlds that
    /// omit it. Loaded as world data (serde-default is fine here — not a persisted snapshot).
    #[serde(default = "default_capita_baseline")]
    pub capita_baseline: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpeningPriceSpec {
    pub market: u32,
    pub good: u16,
    pub price: i64,
}

#[derive(Debug, Clone)]
pub struct BaseWorldBundle {
    pub manifest: BaseWorldManifest,
    pub terrain: TerrainLayer,
    pub transport: TransportLayer,
    pub buildings: BuildingLayer,
    pub decorations: DecorationLayer,
    pub spawns: SpawnLayer,
    pub markets: MarketLayer,
}

impl BaseWorldBundle {
    pub fn load_from_dir(root: impl AsRef<Path>) -> Result<Self, BaseWorldError> {
        let root = root.as_ref();
        let manifest_path = root.join("manifest.json");
        if !manifest_path.exists() {
            return Err(BaseWorldError::MissingManifest(manifest_path));
        }

        let manifest: BaseWorldManifest = read_json(&manifest_path)?;
        let terrain: TerrainLayer = read_json(&root.join(&manifest.layers.terrain))?;
        let transport: TransportLayer = read_json(&root.join(&manifest.layers.transport))?;
        let buildings: BuildingLayer = read_json(&root.join(&manifest.layers.buildings))?;
        let decorations: DecorationLayer = read_json(&root.join(&manifest.layers.decorations))?;
        let spawns: SpawnLayer = read_json(&root.join(&manifest.layers.spawns))?;
        let markets: MarketLayer = read_json(&root.join(&manifest.layers.markets))?;

        let bundle = Self {
            manifest,
            terrain,
            transport,
            buildings,
            decorations,
            spawns,
            markets,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn validate(&self) -> Result<(), BaseWorldError> {
        validate_schema(self.manifest.schema_version)?;
        validate_schema(self.terrain.schema_version)?;
        validate_schema(self.transport.schema_version)?;
        validate_schema(self.buildings.schema_version)?;
        validate_schema(self.decorations.schema_version)?;
        validate_schema(self.spawns.schema_version)?;
        validate_schema(self.markets.schema_version)?;
        self.validate_world_id(&self.terrain.world_id)?;
        self.validate_world_id(&self.transport.world_id)?;
        self.validate_world_id(&self.buildings.world_id)?;
        self.validate_world_id(&self.decorations.world_id)?;
        self.validate_world_id(&self.spawns.world_id)?;
        self.validate_world_id(&self.markets.world_id)?;

        if i32::try_from(self.manifest.world_tiles.width).is_err()
            || i32::try_from(self.manifest.world_tiles.height).is_err()
        {
            return Err(BaseWorldError::WorldTooLarge {
                width: self.manifest.world_tiles.width,
                height: self.manifest.world_tiles.height,
            });
        }

        if self.transport.roads.is_empty() {
            return Err(BaseWorldError::EmptyLayer("transport.roads"));
        }
        if self.transport.pedestrian_corridors.is_empty() {
            return Err(BaseWorldError::EmptyLayer("transport.pedestrian_corridors"));
        }
        if self.buildings.footprints.is_empty() {
            return Err(BaseWorldError::EmptyLayer("buildings.footprints"));
        }
        if self.markets.markets.is_empty() {
            return Err(BaseWorldError::EmptyLayer("markets.markets"));
        }

        for tile in &self.terrain.tiles {
            self.require_in_bounds(tile.x, tile.y)?;
        }
        for road in &self.transport.roads {
            self.require_in_bounds(road.x, road.y)?;
        }
        for rail in &self.transport.rails {
            self.require_in_bounds(rail.x, rail.y)?;
        }
        for path in self.transport_paths() {
            if path.points.is_empty() {
                return Err(BaseWorldError::EmptyLayer("transport.path.points"));
            }
            for point in &path.points {
                self.require_point_in_bounds(point.x, point.y)?;
            }
        }
        for footprint in &self.buildings.footprints {
            if footprint.tiles.is_empty() {
                return Err(BaseWorldError::EmptyLayer("buildings.footprint.tiles"));
            }
            for point in &footprint.tiles {
                self.require_in_bounds(point.x, point.y)?;
            }
        }
        for tree in &self.decorations.trees {
            self.require_in_bounds(tree.x, tree.y)?;
        }
        for detail in &self.decorations.details {
            self.require_in_bounds(detail.x, detail.y)?;
        }

        for group in &self.spawns.car_groups {
            if !self
                .transport
                .arterial_paths
                .iter()
                .any(|p| p.id == group.arterial_id)
            {
                return Err(BaseWorldError::MissingArterialRef {
                    group: group.id.clone(),
                    arterial_id: group.arterial_id.clone(),
                });
            }
        }
        for group in &self.spawns.pedestrian_groups {
            if !self
                .transport
                .pedestrian_corridors
                .iter()
                .any(|c| c.id == group.corridor_id)
            {
                return Err(BaseWorldError::MissingCorridorRef {
                    group: group.id.clone(),
                    corridor_id: group.corridor_id.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn world_id(&self) -> &str {
        &self.manifest.world_id
    }

    pub fn chunk_size(&self) -> u16 {
        self.manifest.chunk_size
    }

    pub fn world_tiles(&self) -> WorldTiles {
        self.manifest.world_tiles
    }

    pub fn snapshot_compatibility(&self) -> SnapshotCompatibility {
        SnapshotCompatibility::new(self.world_id(), self.manifest.schema_version)
    }

    pub fn chunk_coords(&self) -> Vec<ChunkCoord> {
        let chunk_size = u32::from(self.manifest.chunk_size);
        let chunks_x = self.manifest.world_tiles.width.div_ceil(chunk_size);
        let chunks_y = self.manifest.world_tiles.height.div_ceil(chunk_size);

        (0..chunks_y)
            .flat_map(|y| {
                (0..chunks_x).map(move |x| ChunkCoord {
                    x: i32::try_from(x).expect("world dims fit i32 — enforced by validate()"),
                    y: i32::try_from(y).expect("world dims fit i32 — enforced by validate()"),
                })
            })
            .collect()
    }

    pub fn spawn_all_chunks(&self, world: &mut World, initial_version: u64) {
        for coord in self.chunk_coords() {
            let tiles = self.tiles_for_chunk(coord, initial_version);
            spawn_chunk_entity(
                world,
                coord,
                self.manifest.chunk_size,
                tiles,
                initial_version,
                ChunkActivity::Warm,
            );
        }
    }

    pub fn tiles_for_chunk(&self, coord: ChunkCoord, version: u64) -> Vec<TileRecord> {
        let chunk_size = u32::from(self.manifest.chunk_size);
        let mut tiles = Vec::with_capacity((chunk_size * chunk_size) as usize);

        for local_y in 0..chunk_size {
            for local_x in 0..chunk_size {
                let x = coord.x * i32::from(self.manifest.chunk_size)
                    + i32::try_from(local_x).expect("world dims fit i32 — enforced by validate()");
                let y = coord.y * i32::from(self.manifest.chunk_size)
                    + i32::try_from(local_y).expect("world dims fit i32 — enforced by validate()");
                tiles.push(TileRecord {
                    kind: self.tile_kind_at(x, y),
                    version,
                    ..TileRecord::default()
                });
            }
        }

        tiles
    }

    pub fn tile_kind_at(&self, x: i32, y: i32) -> TileKind {
        if !self.point_in_bounds(x, y) {
            return TileKind::Grass;
        }

        if self
            .terrain
            .tiles
            .iter()
            .any(|tile| tile.x == x && tile.y == y && water_like(tile.kind))
        {
            return TileKind::Water;
        }

        if self
            .transport
            .roads
            .iter()
            .any(|path| path.x == x && path.y == y)
        {
            return TileKind::Road;
        }

        if self.buildings.footprints.iter().any(|footprint| {
            footprint
                .tiles
                .iter()
                .any(|point| point.x == x && point.y == y)
        }) {
            return TileKind::BuildingFootprint;
        }

        TileKind::Grass
    }

    pub fn to_city_network(&self) -> CityNetwork {
        CityNetwork {
            version: self.manifest.schema_version,
            world_id: self.manifest.world_id.clone(),
            chunk_size: self.manifest.chunk_size,
            world_tiles: self.manifest.world_tiles,
            arterial_paths: self
                .transport
                .arterial_paths
                .iter()
                .map(|path| path.points.clone())
                .collect(),
            pedestrian_corridors: self
                .transport
                .pedestrian_corridors
                .iter()
                .map(|path| path.points.clone())
                .collect(),
        }
    }

    fn validate_world_id(&self, layer_world_id: &str) -> Result<(), BaseWorldError> {
        if layer_world_id == self.manifest.world_id {
            Ok(())
        } else {
            Err(BaseWorldError::WorldIdMismatch {
                manifest: self.manifest.world_id.clone(),
                layer: layer_world_id.to_owned(),
            })
        }
    }

    fn require_in_bounds(&self, x: i32, y: i32) -> Result<(), BaseWorldError> {
        if self.point_in_bounds(x, y) {
            Ok(())
        } else {
            Err(BaseWorldError::OutOfBounds {
                x,
                y,
                width: self.manifest.world_tiles.width,
                height: self.manifest.world_tiles.height,
            })
        }
    }

    fn require_point_in_bounds(&self, x: f32, y: f32) -> Result<(), BaseWorldError> {
        if self.point_in_bounds_f32(x, y) {
            Ok(())
        } else {
            Err(BaseWorldError::OutOfBounds {
                x: x.floor() as i32,
                y: y.floor() as i32,
                width: self.manifest.world_tiles.width,
                height: self.manifest.world_tiles.height,
            })
        }
    }

    fn point_in_bounds(&self, x: i32, y: i32) -> bool {
        let Ok(width) = i32::try_from(self.manifest.world_tiles.width) else {
            return false;
        };
        let Ok(height) = i32::try_from(self.manifest.world_tiles.height) else {
            return false;
        };
        x >= 0 && y >= 0 && x < width && y < height
    }

    fn point_in_bounds_f32(&self, x: f32, y: f32) -> bool {
        x.is_finite()
            && y.is_finite()
            && x >= 0.0
            && y >= 0.0
            && x < self.manifest.world_tiles.width as f32
            && y < self.manifest.world_tiles.height as f32
    }

    fn transport_paths(&self) -> impl Iterator<Item = &TransportPath> {
        self.transport
            .arterial_paths
            .iter()
            .chain(self.transport.rail_paths.iter())
            .chain(self.transport.pedestrian_corridors.iter())
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, BaseWorldError> {
    let bytes = fs::read(path).map_err(|source| BaseWorldError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| BaseWorldError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn validate_schema(version: u32) -> Result<(), BaseWorldError> {
    if version == SUPPORTED_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(BaseWorldError::UnsupportedSchema(version))
    }
}

fn water_like(kind: TerrainKind) -> bool {
    matches!(kind, TerrainKind::Water | TerrainKind::Riverbank)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_abutopia() -> BaseWorldBundle {
        BaseWorldBundle::load_from_dir(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap()
                .join("data/worlds/abutopia"),
        )
        .expect("abutopia loads")
    }

    #[test]
    fn loads_markets_layer_from_abutopia_bundle() {
        let bundle = load_abutopia();
        assert_eq!(bundle.manifest.schema_version, 2);
        // Four authored markets, ids 9001..9004, ascending.
        let ids: Vec<u32> = bundle.markets.markets.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![9001, 9002, 9003, 9004]);
        // Cross-market distances: ONLY the two intended pairs (each one entry; the
        // factory mirrors both directions at seed time).
        let pairs: Vec<(u32, u32)> = bundle
            .markets
            .distances
            .iter()
            .map(|d| (d.from, d.to))
            .collect();
        assert_eq!(pairs, vec![(9001, 9002), (9003, 9004)]);
    }

    #[test]
    fn markets_layer_rejects_malformed_json() {
        // The serde Parse path (NO-FALLBACK): a markets.json with a TYPE MISMATCH
        // (market id is "not-a-number" instead of u32) must fail closed with
        // BaseWorldError::Parse — it must not reach validate() at all.
        let bundle_src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap()
            .join("data/worlds/abutopia");

        let tmp = tempfile::tempdir().expect("tempdir");
        let tmp_path = tmp.path();

        // Copy manifest.json
        std::fs::copy(
            bundle_src.join("manifest.json"),
            tmp_path.join("manifest.json"),
        )
        .expect("copy manifest");

        // Copy layers directory with all layer files
        let layers_src = bundle_src.join("layers");
        let layers_dst = tmp_path.join("layers");
        std::fs::create_dir(&layers_dst).expect("create layers dir");
        for entry in std::fs::read_dir(&layers_src).expect("read layers dir") {
            let entry = entry.expect("dir entry");
            std::fs::copy(entry.path(), layers_dst.join(entry.file_name()))
                .expect("copy layer file");
        }

        // Overwrite markets.json with valid JSON that has a type mismatch:
        // `id` is u32 but we supply a string — serde rejects this before validate().
        std::fs::write(
            layers_dst.join("markets.json"),
            r#"{
  "schema_version": 2,
  "world_id": "abutopia",
  "markets": [{"id": "not-a-number", "name": "Bad Market", "anchor": [0.0, 0.0]}],
  "distances": [],
  "supply": [],
  "demand": [],
  "extractors": [],
  "household": {"population": 1000},
  "opening_prices": []
}"#,
        )
        .expect("write malformed markets.json");

        let result = BaseWorldBundle::load_from_dir(tmp_path);
        assert!(
            matches!(result, Err(BaseWorldError::Parse { .. })),
            "expected BaseWorldError::Parse for malformed markets.json, got: {result:?}"
        );
    }

    #[test]
    fn markets_layer_rejects_wrong_world_id() {
        // A markets layer with a mismatched world_id must surface BaseWorldError::WorldIdMismatch
        // (NO-FALLBACK: validate() fails closed).
        let mut b = load_abutopia();
        b.markets.world_id = "not-abutopia".into();
        assert!(matches!(
            b.validate(),
            Err(BaseWorldError::WorldIdMismatch { .. })
        ));
    }

    #[test]
    fn markets_layer_rejects_empty_markets() {
        // A markets layer with no markets must surface BaseWorldError::EmptyLayer.
        let mut b = load_abutopia();
        b.markets.markets.clear();
        assert!(matches!(
            b.validate(),
            Err(BaseWorldError::EmptyLayer("markets.markets"))
        ));
    }

    #[test]
    fn validate_accepts_real_abutopia() {
        assert!(load_abutopia().validate().is_ok());
    }

    #[test]
    fn validate_rejects_car_group_with_missing_arterial() {
        let mut b = load_abutopia();
        // abutopia has no car_groups, so push a dangling one
        b.spawns.car_groups.push(CarSpawnGroup {
            id: "spawn:car:dangling".into(),
            arterial_id: "arterial:does-not-exist".into(),
            cars_per_arterial: 1,
        });
        assert!(matches!(
            b.validate(),
            Err(BaseWorldError::MissingArterialRef { .. })
        ));
    }

    #[test]
    fn validate_rejects_pedestrian_group_with_missing_corridor() {
        let mut b = load_abutopia();
        b.spawns.pedestrian_groups[0].corridor_id = "corridor:nope".into();
        assert!(matches!(
            b.validate(),
            Err(BaseWorldError::MissingCorridorRef { .. })
        ));
    }

    #[test]
    fn validate_rejects_world_dimensions_that_overflow_i32() {
        let mut b = load_abutopia();
        b.manifest.world_tiles.width = u32::MAX;
        assert!(matches!(
            b.validate(),
            Err(BaseWorldError::WorldTooLarge { .. })
        ));
    }

    fn bundle_with_pedestrian_points(points: Vec<NetworkPoint>) -> BaseWorldBundle {
        BaseWorldBundle {
            manifest: BaseWorldManifest {
                schema_version: 2,
                world_id: "test".into(),
                display_name: "Test".into(),
                chunk_size: 32,
                world_tiles: WorldTiles {
                    width: 16,
                    height: 8,
                },
                layers: BaseWorldLayerFiles {
                    terrain: "terrain.json".into(),
                    transport: "transport.json".into(),
                    buildings: "buildings.json".into(),
                    decorations: "decorations.json".into(),
                    spawns: "spawns.json".into(),
                    markets: "markets.json".into(),
                },
            },
            terrain: TerrainLayer {
                schema_version: 2,
                world_id: "test".into(),
                tiles: Vec::new(),
            },
            transport: TransportLayer {
                schema_version: 2,
                world_id: "test".into(),
                roads: vec![RoadTile {
                    x: 3,
                    y: 3,
                    kind: "street".into(),
                    mask: 10,
                }],
                rails: Vec::new(),
                arterial_paths: Vec::new(),
                rail_paths: Vec::new(),
                pedestrian_corridors: vec![TransportPath {
                    id: "corridor:test".into(),
                    points,
                }],
            },
            buildings: BuildingLayer {
                schema_version: 2,
                world_id: "test".into(),
                footprints: vec![BuildingFootprint {
                    id: "building:test".into(),
                    tiles: vec![NetworkCoord { x: 2, y: 3 }],
                    sheet: None,
                    frame: None,
                    district: None,
                }],
            },
            decorations: DecorationLayer {
                schema_version: 2,
                world_id: "test".into(),
                trees: Vec::new(),
                details: Vec::new(),
            },
            spawns: SpawnLayer {
                schema_version: 2,
                world_id: "test".into(),
                pedestrian_groups: Vec::new(),
                car_groups: Vec::new(),
                tram_lines: Vec::new(),
            },
            markets: MarketLayer {
                schema_version: 2,
                world_id: "test".into(),
                markets: vec![MarketSpec {
                    id: 1,
                    name: "Test Market".into(),
                    anchor: [2.0, 3.0],
                }],
                distances: Vec::new(),
                supply: Vec::new(),
                demand: Vec::new(),
                extractors: Vec::new(),
                household: HouseholdSpec {
                    population: 1000,
                    capita_baseline: default_capita_baseline(),
                },
                opening_prices: Vec::new(),
            },
        }
    }

    #[test]
    fn base_world_preserves_fractional_pedestrian_corridor_points() {
        let bundle = bundle_with_pedestrian_points(vec![
            NetworkPoint { x: 2.0, y: 2.49 },
            NetworkPoint { x: 13.0, y: 2.49 },
        ]);

        bundle
            .validate()
            .expect("fractional transport points are valid");
        let network = bundle.to_city_network();

        assert_eq!(network.pedestrian_corridors.len(), 1);
        assert_eq!(
            network.pedestrian_corridors[0],
            vec![
                NetworkPoint { x: 2.0, y: 2.49 },
                NetworkPoint { x: 13.0, y: 2.49 }
            ],
        );
    }

    #[test]
    fn base_world_rejects_invalid_float_transport_points() {
        let cases = [
            NetworkPoint {
                x: f32::NAN,
                y: 2.0,
            },
            NetworkPoint {
                x: f32::INFINITY,
                y: 2.0,
            },
            NetworkPoint { x: -0.1, y: 2.0 },
            NetworkPoint { x: 2.0, y: -0.1 },
            NetworkPoint { x: 16.0, y: 2.0 },
            NetworkPoint { x: 2.0, y: 8.0 },
        ];

        for point in cases {
            let bundle = bundle_with_pedestrian_points(vec![point]);
            assert!(
                matches!(
                    bundle.validate(),
                    Err(BaseWorldError::OutOfBounds {
                        x: _,
                        y: _,
                        width: 16,
                        height: 8,
                    })
                ),
                "expected point {point:?} to be out of bounds"
            );
        }
    }
}
