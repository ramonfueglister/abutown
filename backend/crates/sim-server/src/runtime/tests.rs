use super::*;
use abutown_protocol::{ChunkStateDto, TileKindDto};
use super::base_world_expectations::{
    expected_base_world_car_count, expected_base_world_car_routes,
};

fn populated_flow_field_cache() -> sim_core::routing::FlowFieldCache {
    use sim_core::routing::{
        Edge, EdgeId, EdgeKind, FlowFieldCache, FlowFieldCacheKey, FlowFieldScope, Graph, Node,
        NodeId, NodeKind, RoutingProfile, RoutingProfileKey,
    };

    let graph = Graph::new(
        vec![
            Node {
                id: NodeId(0),
                position: (0.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
            Node {
                id: NodeId(1),
                position: (1.0, 0.0),
                kind: NodeKind::Intersection,
                legacy_id: None,
            },
        ],
        vec![
            Edge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                polyline: vec![(0.0, 0.0), (1.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: None,
            },
            Edge {
                id: EdgeId(1),
                from: NodeId(1),
                to: NodeId(0),
                polyline: vec![(1.0, 0.0), (0.0, 0.0)],
                length: 1.0,
                kind: EdgeKind::Footway,
                speed_limit: 1.0,
                capacity: 1,
                legacy_id: None,
            },
        ],
    );
    let mut cache = FlowFieldCache::with_capacity(2);
    cache
        .get_or_build(
            &graph,
            FlowFieldCacheKey::all_edges(NodeId(1), RoutingProfileKey::Walk, 0),
            RoutingProfile::for_key(RoutingProfileKey::Walk),
            FlowFieldScope::AllEdges,
        )
        .expect("test flow field should build");
    assert_eq!(cache.len(), 1);
    cache
}

fn expected_abutopia_chunks() -> Vec<ChunkCoordDto> {
    (0..=3)
        .flat_map(|y| (0..=6).map(move |x| ChunkCoordDto { x, y }))
        .collect()
}

fn expected_abutopia_chunk_coords() -> Vec<ChunkCoord> {
    expected_abutopia_chunks()
        .into_iter()
        .map(|coord| ChunkCoord {
            x: coord.x,
            y: coord.y,
        })
        .collect()
}

#[test]
fn simulation_runtime_holds_world_directly() {
    let runtime = SimulationRuntime::new();
    // After Task 9 dissolved MobilityWorld, SimulationRuntime owns the
    // shared bevy World + Schedule directly.
    let _world: &sim_core::bevy_ecs::world::World = &runtime.world;
    let _schedule: &sim_core::bevy_ecs::schedule::Schedule = &runtime.schedule;
}

#[test]
fn runtime_materializes_base_world_instead_of_demo_chunks() {
    let fixture_root = workspace_root().join("data/worlds/abutopia");
    let runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
        .expect("base world fixture must load");
    let summary = runtime.world_summary();

    assert_eq!(summary.world_id.0, "abutopia");
    assert_eq!(summary.chunk_size, 32);
    assert_eq!(summary.loaded_chunks, expected_abutopia_chunks());
}

#[test]
fn runtime_seeds_backend_pedestrian_from_base_world() {
    let fixture_root = workspace_root().join("data/worlds/abutopia");
    let runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
        .expect("base world fixture must load");
    let agents = sim_core::mobility::api::agents(&runtime.world);
    let vehicles = sim_core::mobility::api::vehicles(&runtime.world);

    assert_eq!(agents.len(), 300);
    assert!(agents.iter().any(|agent| agent.id.0 == "agent:walk:0"));
    assert!(agents.iter().any(|agent| agent.id.0 == "agent:walk:299"));
    assert!(vehicles.is_empty());
}

#[test]
fn runtime_keeps_base_world_agents_concrete_after_viewport_unsubscribe() {
    let fixture_root = workspace_root().join("data/worlds/abutopia");
    let mut runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
        .expect("base world fixture must load");
    let base_world = base_world_fixture();
    let expected_agents = expected_base_world_agent_count(&base_world);
    let first_agent_id = sim_core::ids::AgentId("agent:walk:0".to_string());
    let (x, y) = sim_core::mobility::api::world_coord_for_agent(&runtime.world, &first_agent_id)
        .expect("seeded base-world agent has a world coordinate");
    let agent_chunk = sim_core::mobility::chunk_of(x, y, runtime.chunk_size);

    runtime.apply_subscription_diff([&agent_chunk], []);
    for _ in 0..2 {
        runtime.tick_world_mobility();
    }
    runtime.apply_subscription_diff([], [&agent_chunk]);
    for _ in 0..35 {
        runtime.tick_world_mobility();
    }

    let snapshot = runtime.mobility_persist_snapshot();

    assert_eq!(
        snapshot.agents.len(),
        expected_agents,
        "base-world agents must remain concrete for health-gated persistence after viewport unsubscribe"
    );
    assert!(
        snapshot
            .flow_cells
            .values()
            .all(|cell| cell.population < 1.0),
        "base-world agents must not be folded into anonymous flow cells"
    );
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-server crate lives under backend/crates/sim-server")
        .to_path_buf()
}

fn base_world_fixture() -> sim_core::base_world::BaseWorldBundle {
    sim_core::base_world::BaseWorldBundle::load_from_dir(
        workspace_root().join("data/worlds/abutopia"),
    )
    .expect("base world fixture loads")
}

#[test]
fn runtime_has_populated_routing_graph() {
    let network = base_world_fixture().to_city_network();
    let runtime = SimulationRuntime::new_from_network(&network);
    let world = &runtime.world;
    let graph = world.resource::<sim_core::routing::Graph>();
    assert!(
        graph.node_count() > 0,
        "graph must have nodes after hydration"
    );
    assert!(
        graph.edge_count() > 0,
        "graph must have edges after hydration"
    );
    let traffic_routes = world.resource::<sim_core::routing::TrafficRoutes>();
    assert_eq!(traffic_routes.count(), 0);
    let spatial = world.resource::<sim_core::routing::NodeSpatialIndex>();
    assert_eq!(spatial.size(), graph.node_count());
}

#[test]
fn runtime_has_pathfinding_resources() {
    let runtime = SimulationRuntime::new();
    assert!(
        runtime
            .world
            .contains_resource::<sim_core::routing::PathCache>()
    );
}

#[test]
fn runtime_installs_flow_field_cache() {
    let runtime = SimulationRuntime::new();
    assert!(
        runtime
            .world
            .contains_resource::<sim_core::routing::FlowFieldCache>()
    );
    assert_eq!(
        runtime
            .world
            .resource::<sim_core::routing::FlowFieldCache>()
            .len(),
        0
    );
}

#[test]
fn runtime_installs_economy_plugin() {
    let runtime = SimulationRuntime::new();
    assert!(
        runtime
            .world
            .contains_resource::<sim_core::economy::AccountBook>()
    );
    assert!(
        runtime
            .world
            .contains_resource::<sim_core::economy::OrderBook>()
    );
    assert!(
        runtime
            .world
            .contains_resource::<sim_core::economy::TradeLedger>()
    );
}

#[test]
fn runtime_installs_hpa_index_for_seeded_graph() {
    let network = base_world_fixture().to_city_network();
    let runtime = SimulationRuntime::new_from_network(&network);
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();

    assert!(hpa.cluster_count() > 0);
    assert_eq!(hpa.portal_count(), 0);
    assert!(hpa.cluster_count() <= graph.node_count());
}

#[test]
fn runtime_can_find_seeded_hierarchical_walk_path() {
    let network = base_world_fixture().to_city_network();
    let runtime = SimulationRuntime::new_from_network(&network);
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();
    let walk_edge = graph
        .edges()
        .iter()
        .find(|edge| edge.kind == sim_core::routing::EdgeKind::Footway)
        .expect("seeded runtime should contain a footway edge");

    let (path, stats) = sim_core::routing::HpaRouter::find_path(
        graph,
        hpa,
        sim_core::routing::PathRequest {
            from: walk_edge.from,
            to: walk_edge.to,
            profile: sim_core::routing::RoutingProfileKey::Walk,
        },
        sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Walk),
    )
    .expect("seeded footway edge endpoints should route through HPA");

    assert!(!path.edges.is_empty());
    assert!(stats.corridor_cluster_count >= 1);
    assert!(
        path.edges
            .iter()
            .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::Footway)
    );
}

#[test]
fn runtime_can_find_seeded_walk_path() {
    let network = base_world_fixture().to_city_network();
    let runtime = SimulationRuntime::new_from_network(&network);
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let walk_edge = graph
        .edges()
        .iter()
        .find(|edge| edge.kind == sim_core::routing::EdgeKind::Footway)
        .expect("seeded runtime should contain a footway edge");
    let path = sim_core::routing::AStarRouter::find_path(
        graph,
        sim_core::routing::PathRequest {
            from: walk_edge.from,
            to: walk_edge.to,
            profile: sim_core::routing::RoutingProfileKey::Walk,
        },
        sim_core::routing::RoutingProfile::for_key(sim_core::routing::RoutingProfileKey::Walk),
    )
    .expect("seeded footway edge endpoints should be connected by the routing graph");
    assert!(!path.edges.is_empty());
    assert!(
        path.edges
            .iter()
            .all(|edge| graph.edge(edge.edge_id).kind == sim_core::routing::EdgeKind::Footway)
    );
}

#[test]
fn runtime_uses_sidewalk_footway_geometry_from_base_world() {
    let network = base_world_fixture().to_city_network();
    let runtime = SimulationRuntime::new_from_network(&network);
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let edge = graph.edge(
        graph
            .edge_by_legacy("link:walk:corridor:1")
            .expect("south sidewalk footway exists"),
    );

    assert_eq!(edge.kind, sim_core::routing::EdgeKind::Footway);
    assert_eq!(edge.polyline.first().copied(), Some((106.0, 64.51)));
    assert_eq!(edge.polyline.last().copied(), Some((117.0, 64.51)));
    assert!(edge.polyline.iter().all(|(_, y)| (*y - 64.0).abs() > 0.001));
}

#[test]
fn runtime_uses_grass_footways_from_base_world() {
    let base_world = base_world_fixture();
    let runtime = SimulationRuntime::new_with_event_store_and_base_world(
        Box::new(InMemoryWorldEventStore::default()),
        base_world.clone(),
    )
    .expect("base world runtime should construct");
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let grass_edges: Vec<_> = graph
        .edges()
        .iter()
        .filter(|edge| {
            edge.kind == sim_core::routing::EdgeKind::Footway
                && edge
                    .legacy_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("link:walk:grass:"))
        })
        .collect();

    assert!(
        !grass_edges.is_empty(),
        "base world graph has grass footways"
    );
    for edge in grass_edges {
        for (x, y) in &edge.polyline {
            assert_eq!(
                base_world.tile_kind_at(x.floor() as i32, y.floor() as i32),
                sim_core::tile::TileKind::Grass,
                "grass footway endpoints must stay on grass tiles"
            );
        }
    }
}

#[test]
fn runtime_restores_grass_footways_bidirectionally() {
    let base_world = base_world_fixture();
    let runtime = SimulationRuntime::new_with_event_store_and_base_world(
        Box::new(InMemoryWorldEventStore::default()),
        base_world,
    )
    .expect("base world runtime should construct");
    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let grass_edge = graph
        .edges()
        .iter()
        .find(|edge| {
            edge.kind == sim_core::routing::EdgeKind::Footway
                && edge
                    .legacy_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("link:walk:grass:"))
        })
        .expect("base world graph has grass footways");

    assert!(
        graph.outgoing(grass_edge.to).iter().any(|edge_id| {
            let candidate = graph.edge(*edge_id);
            candidate.kind == sim_core::routing::EdgeKind::Footway
                && candidate.to == grass_edge.from
        }),
        "restored grass footway endpoints need a reverse footway so walkers do not reset at dead ends"
    );
}

#[test]
fn set_mobility_for_test_refreshes_hpa_index() {
    let network = base_world_fixture().to_city_network();
    let mut runtime = SimulationRuntime::new_from_network(&network);

    runtime.set_mobility_for_test(sim_core::mobility::seed::from_network(
        &network,
        SEED_DENSITY,
    ));

    let graph = runtime.world.resource::<sim_core::routing::Graph>();
    let hpa = runtime.world.resource::<sim_core::routing::HpaIndex>();
    let expected =
        sim_core::routing::HpaIndex::build(graph, sim_core::routing::HpaConfig::default())
            .expect("current graph should build an HPA index");

    assert_eq!(hpa.cluster_count(), expected.cluster_count());
    assert_eq!(hpa.portal_count(), expected.portal_count());
    for node in graph.nodes() {
        assert_eq!(
            hpa.cluster_of_node(node.id),
            expected.cluster_of_node(node.id)
        );
    }
}

#[test]
fn set_mobility_for_test_refreshes_flow_field_cache() {
    let network = base_world_fixture().to_city_network();
    let mut runtime = SimulationRuntime::new_from_network(&network);
    runtime.world.insert_resource(populated_flow_field_cache());

    runtime.set_mobility_for_test(sim_core::mobility::seed::from_network(
        &network,
        SEED_DENSITY,
    ));

    assert!(
        runtime
            .world
            .contains_resource::<sim_core::routing::FlowFieldCache>()
    );
    assert_eq!(
        runtime
            .world
            .resource::<sim_core::routing::FlowFieldCache>()
            .len(),
        0
    );
}

#[test]
fn hydration_spawns_chunk_entity_per_loaded_chunk() {
    let runtime = SimulationRuntime::new();
    let world = &runtime.world;
    let by_coord = world.resource::<sim_core::world::resources::ChunksByCoord>();
    let expected = expected_abutopia_chunk_coords();
    assert_eq!(by_coord.0.len(), expected.len());
    for coord in expected {
        assert!(by_coord.0.contains_key(&coord));
    }
}
use sim_core::persistence::{
    InMemoryChunkSnapshotStore, InMemoryEconomySnapshotStore, InMemoryMobilitySnapshotStore,
    build_chunk_snapshot,
};

fn tile_pulse(message: ServerMessageDto) -> TilePulseDeltaDto {
    let ServerMessageDto::TilePulse(delta) = message else {
        panic!("message should be a tile pulse");
    };
    delta
}

/// Regression for the live `routed=0` bug: the production hydrate path
/// (`hydrate_from_stores`, used by every running server) must seed the economy
/// markets BEFORE spawning agents, so the spawn-time binding guard in
/// `mobility::api::spawn_agent_from_record` can resolve each citizen's
/// home/work market from the (now-present) `Markets` resource.
///
/// Before the fix `seed_from_markets_layer` ran AFTER `apply_into_world`, so
/// every seeded pedestrian was spawned into a world with an empty `Markets`
/// resource and frozen at `home_market = 0` (unbound) — which then persisted and
/// survived every restart. Economy attribution filters candidates by
/// `observed_markets.contains(home_market)`, and 0 is never a market id, so the
/// on-map economy reported `routed = 0` forever even though the corridor
/// pedestrians stand on market 9002's tile. The fresh `new()` path already
/// seeded before spawning, which is why this only reproduced live (and why the
/// economy tests, which spawn fresh citizens after seeding, masked it).
#[tokio::test]
async fn hydrate_binds_seed_pedestrians_to_their_home_market() {
    let base_world = base_world_fixture();

    let (mut runtime, _chunk_store, _mobility_store, _economy_store) =
        SimulationRuntime::hydrate_from_stores(
            Box::new(InMemoryWorldEventStore::default()),
            Box::new(InMemoryChunkSnapshotStore::default()),
            Box::new(InMemoryMobilitySnapshotStore::default()),
            Box::new(InMemoryEconomySnapshotStore::default()),
            &base_world,
        )
        .await
        .expect("hydrate abutopia from empty stores");

    // Only agents carry `MarketBinding`, so an unfiltered query enumerates the
    // seeded pedestrians' home-market bindings.
    let mut binding_query = runtime.world.query::<&sim_core::mobility::MarketBinding>();
    let home_markets: Vec<u32> = binding_query
        .iter(&runtime.world)
        .map(|binding| binding.home_market)
        .collect();

    assert!(
        !home_markets.is_empty(),
        "abutopia seeds corridor pedestrians"
    );
    assert!(
        home_markets.iter().all(|&m| m != 0),
        "no seeded pedestrian may be left unbound (home_market=0); {} of {} were unbound",
        home_markets.iter().filter(|&&m| m == 0).count(),
        home_markets.len(),
    );
    // Every `corridor:sidewalk:south` pedestrian (x 106..=115 at y 64.51) is
    // nearest to market 9002 ([111.5, 64.51]); 9001/9003/9004 are far away.
    assert!(
        home_markets.iter().all(|&m| m == 9002),
        "corridor pedestrians must bind to market 9002; distinct home markets seen: {:?}",
        {
            let mut distinct: Vec<u32> = home_markets.clone();
            distinct.sort_unstable();
            distinct.dedup();
            distinct
        },
    );
}

#[test]
fn runtime_summarizes_abutopia_loaded_chunk() {
    let runtime = SimulationRuntime::new();

    let summary = runtime.world_summary();

    assert_eq!(summary.chunk_size, 32);
    assert_eq!(summary.world_id.0, "abutopia");
    assert_eq!(summary.loaded_chunks, expected_abutopia_chunks());
}

#[test]
fn runtime_returns_snapshot_for_abutopia_chunk() {
    let runtime = SimulationRuntime::new();

    let visible = runtime
        .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
        .expect("visible chunk loaded");

    assert_eq!(visible.coord, ChunkCoordDto { x: 0, y: 0 });
    assert!(runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).is_some());
    assert!(runtime.chunk_snapshot(ChunkCoord { x: 1, y: 0 }).is_some());
    assert!(runtime.chunk_snapshot(ChunkCoord { x: 7, y: 0 }).is_none());
}

#[test]
fn runtime_pulses_loaded_abutopia_chunks_in_order() {
    let mut runtime = SimulationRuntime::new();

    let first = tile_pulse(runtime.next_pulse());
    let second = tile_pulse(runtime.next_pulse());
    let third = tile_pulse(runtime.next_pulse());
    let fourth = tile_pulse(runtime.next_pulse());

    assert_eq!(first.tick, 1);
    assert_eq!(first.version, 1);
    assert_eq!(first.coord, ChunkCoordDto { x: 0, y: 0 });
    assert!(first.local_index < 1024);
    assert_eq!(second.tick, 2);
    assert_eq!(second.coord, ChunkCoordDto { x: 1, y: 0 });
    assert_eq!(third.tick, 3);
    assert_eq!(third.coord, ChunkCoordDto { x: 2, y: 0 });
    assert_eq!(fourth.tick, 4);
    assert_eq!(fourth.coord, ChunkCoordDto { x: 3, y: 0 });
}

#[tokio::test]
async fn collect_provider_items_routes_dirty_chunk_to_chunk_store() {
    // Issue #1 acceptance: construct a runtime, mutate a tile (so a
    // chunk becomes dirty), drive the persist path via SnapshotProviders
    // (not the legacy `collect_chunk_snapshots()` shortcut), and verify
    // a `ChunkSnapshotStore` receives the chunk snapshot.
    use sim_core::persistence::InMemoryChunkSnapshotStore;

    let mut runtime = SimulationRuntime::new();
    // Mutate the authored abutopia chunk to make sure the dirty path
    // through SnapshotProviders is exercised.
    runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: "command:provider-path:1".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 9,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect("command applies cleanly");

    let items = runtime.collect_provider_items();
    // Expect at least one chunk item (for the dirty chunk) and exactly
    // one mobility item.
    let chunk_items: Vec<_> = items.iter().filter(|i| i.key.kind == "chunk").collect();
    let mobility_items: Vec<_> = items.iter().filter(|i| i.key.kind == "mobility").collect();
    assert!(
        !chunk_items.is_empty(),
        "expected at least one chunk SnapshotItem from provider path",
    );
    assert_eq!(
        mobility_items.len(),
        1,
        "MobilitySnapshotProvider emits exactly one item per collect",
    );

    // Dispatch chunk items to the in-memory ChunkSnapshotStore via the
    // same code path as the persist loop in `app.rs`.
    let mut store = InMemoryChunkSnapshotStore::default();
    let compatibility = base_world_fixture().snapshot_compatibility();
    for item in chunk_items {
        let dto: abutown_protocol::ChunkSnapshotDto = serde_json::from_slice(&item.payload)
            .expect("provider emits valid ChunkSnapshotDto JSON");
        ChunkSnapshotStore::write_snapshot(&mut store, dto, &compatibility)
            .await
            .expect("in-memory store write");
    }

    let stored = store
        .read_snapshot(ChunkCoord { x: 0, y: 0 }, &compatibility)
        .expect("snapshot for the mutated chunk landed in the store");
    assert_eq!(stored.coord, abutown_protocol::ChunkCoordDto { x: 0, y: 0 });
}

#[tokio::test]
async fn runtime_collects_chunk_snapshots_and_marks_persisted() {
    use sim_core::persistence::InMemoryChunkSnapshotStore;

    let mut runtime = SimulationRuntime::new();
    let mut store = InMemoryChunkSnapshotStore::default();
    let compatibility = base_world_fixture().snapshot_compatibility();

    let snapshots = runtime.collect_chunk_snapshots();
    assert_eq!(snapshots.len(), 0);

    // After marking persisted with no further events and within the 30s ceiling,
    // the registry must skip every chunk.
    assert_eq!(runtime.collect_chunk_snapshots().len(), 0);

    // A new event on one chunk re-arms only that chunk for the next collect.
    runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: "command:persist-test:1".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 11,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect("command should apply");

    let next_snapshots = runtime.collect_chunk_snapshots();
    assert_eq!(next_snapshots.len(), 1);
    for snapshot in &next_snapshots {
        store.write_snapshot(snapshot.clone(), &compatibility);
    }
    let next_coords: Vec<ChunkCoord> = next_snapshots
        .iter()
        .map(|s| ChunkCoord {
            x: s.coord.x,
            y: s.coord.y,
        })
        .collect();
    runtime.mark_chunk_snapshots_persisted(&next_coords);

    let visible = store
        .read_snapshot(ChunkCoord { x: 0, y: 0 }, &compatibility)
        .expect("visible snapshot reflects new event");
    assert!(visible.tiles.iter().any(|tile| {
        tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
    }));
}

#[tokio::test]
async fn runtime_applies_set_tile_kind_command_and_appends_event() {
    let mut runtime = SimulationRuntime::new();

    let applied = runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: "command:test:1".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 11,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect("command should apply");

    let abutown_protocol::WorldEventDto::TileKindSet(event) = &applied.event;
    assert!(event.event_id.starts_with("event:"));
    assert_eq!(event.command_id, "command:test:1");
    assert_eq!(event.version, 1);
    assert_eq!(event.local_index, 11);
    assert_eq!(event.kind, abutown_protocol::TileKindDto::Water);
    assert_eq!(runtime.event_count(), 1);

    let snapshot = runtime
        .chunk_snapshot(sim_core::ids::ChunkCoord { x: 0, y: 0 })
        .expect("mutated chunk snapshot exists");
    assert!(snapshot.tiles.iter().any(|tile| {
        tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
    }));
}

#[tokio::test]
async fn runtime_rejects_commands_for_other_worlds() {
    let mut runtime = SimulationRuntime::new();

    let rejection = runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("other-world".to_string()),
                command_id: "command:test:2".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 11,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect_err("wrong world should reject");

    assert_eq!(rejection.code, "wrong_world");
    assert_eq!(runtime.event_count(), 0);
}

#[tokio::test]
async fn runtime_rejects_commands_for_unloaded_chunks() {
    let mut runtime = SimulationRuntime::new();

    let rejection = runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: "command:test:3".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 9, y: 9 },
                local_index: 11,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect_err("unloaded chunk should reject");

    assert_eq!(rejection.code, "chunk_not_loaded");
    assert_eq!(runtime.event_count(), 0);
}

#[tokio::test]
async fn runtime_rejects_no_op_tile_kind_commands_without_appending_event() {
    let mut runtime = SimulationRuntime::new();
    let current_kind = runtime
        .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
        .and_then(|snapshot| {
            snapshot
                .tiles
                .into_iter()
                .find(|tile| tile.local_index == 11)
                .map(|tile| tile.kind)
        })
        .unwrap_or(abutown_protocol::TileKindDto::Grass);

    let rejection = runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: "command:test:4".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 11,
                kind: current_kind,
            },
        ))
        .await
        .expect_err("no-op command should reject");

    assert_eq!(rejection.code, "no_state_change");
    assert_eq!(runtime.event_count(), 0);
}

#[tokio::test]
async fn hydrate_from_stores_restores_chunk_from_snapshot_and_replays_tail_events() {
    // Seed: a chunk with tile 0 = Road at version 1, snapshotted.
    let mut authoring_chunk = Chunk::new(ChunkCoord { x: 0, y: 0 }, 32);
    authoring_chunk.set_tile_kind(0, TileKind::Road).unwrap();
    let snapshot = build_chunk_snapshot("abutopia", &authoring_chunk, ChunkActivity::Active);

    let mut snapshot_store = InMemoryChunkSnapshotStore::default();
    let base_world = base_world_fixture();
    let compatibility = base_world.snapshot_compatibility();
    ChunkSnapshotStore::write_snapshot(&mut snapshot_store, snapshot, &compatibility)
        .await
        .unwrap();

    // Tail event after the snapshot: tile 7 = Water at chunk_version 2.
    let tail_event = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id: "event:tail".to_string(),
        command_id: "command:tail".to_string(),
        world_id: WorldId("abutopia".to_string()),
        tick: 2,
        version: 2,
        coord: ChunkCoordDto { x: 0, y: 0 },
        local_index: 7,
        kind: TileKindDto::Water,
    });
    let mut event_store = InMemoryWorldEventStore::default();
    WorldEventStore::append(&mut event_store, tail_event)
        .await
        .unwrap();

    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(event_store),
        Box::new(snapshot_store),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();

    let restored = runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).unwrap();
    assert_eq!(restored.chunk_version, 2);
    let kinds: std::collections::HashMap<u16, TileKindDto> = restored
        .tiles
        .iter()
        .map(|t| (t.local_index, t.kind))
        .collect();
    assert_eq!(kinds.get(&0), Some(&TileKindDto::Road));
    assert_eq!(kinds.get(&7), Some(&TileKindDto::Water));
    assert_eq!(restored.chunk_state, ChunkStateDto::Active);
}

#[tokio::test]
async fn hydrate_from_stores_seeds_when_no_snapshot() {
    let base_world = base_world_fixture();
    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();

    let snap = runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).unwrap();
    assert_eq!(
        snap.chunk_version, 0,
        "base world chunks start at version 0 before player mutations"
    );
}

#[tokio::test]
async fn runtime_rejects_store_failure_without_mutating_chunk() {
    let mut runtime = SimulationRuntime::new_with_event_store(Box::new(
        sim_core::events::FailingWorldEventStore::new("database offline"),
    ));

    let before = runtime
        .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
        .expect("chunk exists");

    let rejection = runtime
        .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
            abutown_protocol::SetTileKindCommandDto {
                protocol_version: abutown_protocol::PROTOCOL_VERSION,
                world_id: abutown_protocol::WorldId("abutopia".to_string()),
                command_id: "command:test:store-failure".to_string(),
                coord: abutown_protocol::ChunkCoordDto { x: 0, y: 0 },
                local_index: 11,
                kind: abutown_protocol::TileKindDto::Water,
            },
        ))
        .await
        .expect_err("store failure should reject");

    assert_eq!(rejection.code, "event_store_unavailable");
    assert_eq!(runtime.event_count(), 0);
    assert_eq!(
        runtime
            .chunk_snapshot(ChunkCoord { x: 0, y: 0 })
            .expect("chunk still exists"),
        before
    );
}

#[tokio::test]
async fn duplicate_command_id_is_idempotent_and_writes_only_one_event() {
    use abutown_protocol::{
        ChunkCoordDto, ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto,
        WorldId,
    };

    let mut runtime = SimulationRuntime::new();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutopia".to_string()),
        command_id: "command:dup".to_string(),
        coord: ChunkCoordDto { x: 0, y: 0 },
        local_index: 12,
        kind: TileKindDto::Water,
    });

    let first = runtime.apply_client_command(command.clone()).await.unwrap();
    let second = runtime.apply_client_command(command).await.unwrap();

    assert_eq!(
        first.response, second.response,
        "duplicate command must return identical response"
    );
    assert_eq!(
        first.event, second.event,
        "duplicate command must return identical event"
    );
    assert_eq!(runtime.event_count(), 1, "only one event must be appended");
}

#[derive(Debug)]
struct RaceyEventStore {
    planted_winner: WorldEventDto,
    appended: bool,
}

impl RaceyEventStore {
    fn new(planted_winner: WorldEventDto) -> Self {
        Self {
            planted_winner,
            appended: false,
        }
    }
}

#[async_trait::async_trait]
impl WorldEventStore for RaceyEventStore {
    async fn append(
        &mut self,
        _event: WorldEventDto,
    ) -> Result<(), sim_core::events::WorldEventStoreError> {
        self.appended = true;
        Err(sim_core::events::WorldEventStoreError::duplicate_command(
            "command:race",
        ))
    }
    async fn find_event_by_command(
        &self,
        _world_id: &str,
        command_id: &str,
    ) -> Result<Option<WorldEventDto>, sim_core::events::WorldEventStoreError> {
        // Pre-flight call (before append) returns None so we fall through to the append path.
        // Refetch call (after append, in the race handler) returns the planted winner.
        if self.appended && command_id == "command:race" {
            Ok(Some(self.planted_winner.clone()))
        } else {
            Ok(None)
        }
    }
    async fn read_chunk_events_since(
        &self,
        _world_id: &str,
        _coord: abutown_protocol::ChunkCoordDto,
        _after_chunk_version: u64,
    ) -> Result<Vec<WorldEventDto>, sim_core::events::WorldEventStoreError> {
        Ok(Vec::new())
    }
    async fn max_tick(
        &self,
        _world_id: &str,
    ) -> Result<Option<u64>, sim_core::events::WorldEventStoreError> {
        Ok(None)
    }
    async fn max_version(
        &self,
        _world_id: &str,
    ) -> Result<Option<u64>, sim_core::events::WorldEventStoreError> {
        Ok(None)
    }
}

#[tokio::test]
async fn race_handler_returns_winner_when_append_reports_duplicate() {
    use abutown_protocol::{
        ChunkCoordDto, ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto,
        TileKindSetEventDto, WorldEventDto, WorldId,
    };

    let winner = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id: "event:winner".to_string(),
        command_id: "command:race".to_string(),
        world_id: WorldId("abutopia".to_string()),
        tick: 7,
        version: 7,
        coord: ChunkCoordDto { x: 0, y: 0 },
        local_index: 0,
        kind: TileKindDto::Water,
    });
    let mut runtime =
        SimulationRuntime::new_with_event_store(Box::new(RaceyEventStore::new(winner.clone())));

    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutopia".to_string()),
        command_id: "command:race".to_string(),
        coord: ChunkCoordDto { x: 0, y: 0 },
        local_index: 13,
        kind: TileKindDto::Road,
    });

    let result = runtime.apply_client_command(command).await.unwrap();
    assert_eq!(
        result.event, winner,
        "race handler must return the planted winner event"
    );
    assert_eq!(result.response.event, winner);
}

#[tokio::test]
async fn hydrate_seeds_fresh_mobility_when_store_is_empty() {
    let base_world = base_world_fixture();
    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();

    assert_eq!(runtime.mobility_tick_for_test(), 0);
    assert_eq!(
        runtime.mobility_agent_count_for_test(),
        expected_base_world_agent_count(&base_world)
    );
    assert_eq!(
        runtime.mobility_vehicle_count_for_test(),
        expected_base_world_car_count(&base_world)
    );
}

#[tokio::test]
async fn hydrate_restores_mobility_from_store_when_present() {
    use sim_core::mobility::{extract_from_world, seed};

    let base_world = base_world_fixture();
    let (authored, _) =
        seed::from_base_world_bundle(&base_world).expect("base world mobility seed succeeds");
    let mut authored_snap = extract_from_world(&authored);
    authored_snap.tick = 7;

    let mut mobility_store = InMemoryMobilitySnapshotStore::default();
    MobilitySnapshotStore::write(
        &mut mobility_store,
        "abutopia",
        authored_snap.tick,
        &authored_snap,
        &base_world.snapshot_compatibility(),
    )
    .await
    .unwrap();

    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(mobility_store),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();

    assert_eq!(runtime.mobility_tick_for_test(), 7);
}

#[tokio::test]
async fn hydrate_resumes_evolved_demographic_snapshot() {
    use sim_core::ids::AgentId;
    use sim_core::mobility::{extract_from_world, seed};

    // A live world evolves away from the pristine seed: deaths remove seeded
    // walkers, births add `agent:born:*` citizens, and the tick advances. The
    // store read is already gated by (world_id, schema_version), so hydration
    // must resume exactly what the store returns — discarding it here was the
    // every-boot-restarts-at-tick-0 prod bug.
    let base_world = base_world_fixture();
    let (authored, _) =
        seed::from_base_world_bundle(&base_world).expect("base world mobility seed succeeds");
    let mut evolved_snap = extract_from_world(&authored);

    let dead = evolved_snap
        .agents
        .remove(&AgentId("agent:walk:0".to_string()))
        .expect("authored snapshot contains the seeded pedestrian");
    let born_id = AgentId("agent:born:agent:walk:1:42".to_string());
    let mut born = dead;
    born.id = born_id.clone();
    evolved_snap.agents.insert(born_id.clone(), born);
    evolved_snap.tick = 4242;

    let mut mobility_store = InMemoryMobilitySnapshotStore::default();
    MobilitySnapshotStore::write(
        &mut mobility_store,
        "abutopia",
        evolved_snap.tick,
        &evolved_snap,
        &base_world.snapshot_compatibility(),
    )
    .await
    .unwrap();

    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(mobility_store),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();

    assert_eq!(runtime.mobility_tick_for_test(), 4242);
    let restored = runtime.mobility_snapshot_for_persist();
    assert!(restored.agents.contains_key(&born_id));
    assert!(
        !restored
            .agents
            .contains_key(&AgentId("agent:walk:0".to_string()))
    );
}

#[tokio::test]
async fn hydrate_restores_activity_waypoints_for_persisted_base_world_mobility() {
    use sim_core::mobility::{extract_from_world, seed};

    let base_world = base_world_fixture();
    let (authored, _) =
        seed::from_base_world_bundle(&base_world).expect("base world mobility seed succeeds");
    let mut authored_snap = extract_from_world(&authored);
    authored_snap.tick = 7;

    let mut mobility_store = InMemoryMobilitySnapshotStore::default();
    MobilitySnapshotStore::write(
        &mut mobility_store,
        "abutopia",
        authored_snap.tick,
        &authored_snap,
        &base_world.snapshot_compatibility(),
    )
    .await
    .unwrap();

    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(mobility_store),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();

    let waypoints = runtime
        .world
        .resource::<sim_core::mobility::resources::ActivityWaypoints>();
    assert_eq!(
        waypoints.0.get("activity:home").copied(),
        Some((106.0, 64.51))
    );
    assert_eq!(
        waypoints.0.get("activity:destination").copied(),
        Some((117.0, 64.51))
    );
}

#[test]
fn expected_car_routes_skips_dangling_arterial_without_panicking() {
    let mut b = base_world_fixture(); // loads abutopia (has no car_groups)
    b.spawns
        .car_groups
        .push(sim_core::base_world::CarSpawnGroup {
            id: "spawn:car:dangling".into(),
            arterial_id: "arterial:missing".into(),
            cars_per_arterial: 3,
        });
    // Must not panic; the dangling group contributes nothing.
    let routes = expected_base_world_car_routes(&b);
    assert!(
        routes
            .keys()
            .all(|k| !k.contains("dangling") && !k.contains("missing"))
    );
}

#[tokio::test]
async fn hydrate_restores_economy_snapshot() {
    use sim_core::economy::{EconomicActorId, EconomyPersistSnapshot, Money, MoneyAccount};

    let base_world = base_world_fixture();
    let compat = base_world.snapshot_compatibility();

    // Pre-load an economy store with a snapshot carrying one account.
    let mut snap = EconomyPersistSnapshot::default();
    snap.accounts.push((
        EconomicActorId(1),
        MoneyAccount {
            available: Money(777),
            locked: Money(0),
        },
    ));
    let mut econ_store = InMemoryEconomySnapshotStore::default();
    econ_store
        .write(base_world.world_id(), 1, &snap, &compat)
        .await
        .unwrap();

    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(econ_store),
        &base_world,
    )
    .await
    .unwrap();

    let restored = runtime.economy_snapshot();
    assert_eq!(
        restored
            .accounts
            .iter()
            .find(|(a, _)| *a == EconomicActorId(1))
            .map(|(_, acc)| acc.available),
        Some(Money(777)),
        "economy account restored from snapshot store"
    );
}

#[test]
fn runtime_sets_population_carrying_capacity_from_base_world_seed_count() {
    let runtime = SimulationRuntime::new(); // fresh path, abutopia
    let cfg = runtime
        .world
        .resource::<sim_core::population::PopulationConfig>();
    let expected = expected_base_world_agent_count(&base_world_fixture()) as f32;
    assert!(expected > 0.0, "abutopia seeds >0 agents");
    assert_eq!(
        cfg.carrying_capacity, expected,
        "carrying capacity = base-world seed count"
    );
}

#[tokio::test]
async fn hydrate_with_empty_economy_store_bootstraps_demo_economy() {
    // A world with no persisted economy (brand-new, or created before the economy
    // existed) gets the demo economy bootstrapped on hydrate — this is what makes
    // the flow-demo markets visible in the always-hydrated live server.
    let base_world = base_world_fixture();
    let (runtime, _, _, _) = SimulationRuntime::hydrate_from_stores(
        Box::new(InMemoryWorldEventStore::default()),
        Box::new(InMemoryChunkSnapshotStore::default()),
        Box::new(InMemoryMobilitySnapshotStore::default()),
        Box::new(InMemoryEconomySnapshotStore::default()),
        &base_world,
    )
    .await
    .unwrap();
    let snap = runtime.economy_snapshot();
    assert_eq!(
        snap.markets.len(),
        4,
        "demo markets bootstrapped on empty hydrate (2 original + 2 flow-demo)"
    );
}
