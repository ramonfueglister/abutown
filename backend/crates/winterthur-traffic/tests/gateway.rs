//! Integration test for the WS gateway (Task 8): connect a real WebSocket
//! client, subscribe to AOI cells, and assert the keyframe-then-delta protocol.
//!
//! The sim is driven **manually** (the ECS `Schedule` ticked directly, as in
//! `tests/shell.rs`) rather than by the 10 Hz wall-clock loop, so the test is
//! deterministic and fast: we tick to warm up the fleet, discover which cells
//! hold vehicles from the same `Core` the server publishes from, subscribe to
//! them, then tick more and read frames off the socket. The axum `/traffic`
//! server runs on its own task sharing the `Registry` with the publisher.

use std::path::PathBuf;
use std::time::Duration;

use bevy_ecs::prelude::*;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use abutown_protocol::traffic::{TrafficClientMsg, TrafficServerMsg};
use traffic_net::TrafficNet;
use winterthur_traffic::cells::CellGrid;
use winterthur_traffic::gateway::{self, Registry, make_publisher};
use winterthur_traffic::shell::{self, CoreRes, SnapshotHook};

fn data_path(file: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.pop();
    p.push("data/winterthur");
    p.push(file);
    p
}

fn load_real_net() -> TrafficNet {
    let p = data_path("trafficnet.json");
    let json = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    traffic_net::load(&json).expect("real Winterthur bake must validate")
}

fn load_buildings() -> String {
    let p = data_path("buildings.json");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Collect the set of currently-occupied cells from the live `Core`.
fn busy_cells(world: &World, grid: &CellGrid) -> Vec<u32> {
    let core = &world.resource::<CoreRes>().0;
    let mut cells = std::collections::HashSet::new();
    for veh in 0..core.fleet.slots() as u32 {
        if let Some(view) = core.vehicle_view(veh)
            && let Some(c) = grid.cell_of_lane_s(view.lane, view.s)
        {
            cells.insert(c);
        }
    }
    let mut v: Vec<u32> = cells.into_iter().collect();
    v.sort_unstable();
    v
}

/// Build the world with the gateway publisher installed, plus the grid + the
/// shared registry. The axum server is started by the caller.
fn build_gateway_world(registry: &Registry) -> (World, Schedule, CellGrid) {
    let net = load_real_net();
    let buildings = load_buildings();
    let grid = CellGrid::build(&net);
    let (mut world, schedule) = shell::build_sim_with_buildings(net, 0, &buildings);
    world.insert_resource(make_publisher(grid.clone(), registry.clone()));
    (world, schedule, grid)
}

async fn start_server(registry: Registry) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = gateway::router(registry);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

/// Subscribe to `cells` via a binary `TrafficClientMsg`.
fn subscribe_msg(cells: &[u32]) -> Vec<u8> {
    TrafficClientMsg {
        subscribe_cells: cells.to_vec(),
        unsubscribe_cells: Vec::new(),
    }
    .encode_to_vec()
}

type WsClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(port: u16) -> WsClient {
    let url = format!("ws://127.0.0.1:{port}/traffic");
    let (ws, _resp) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws
}

/// Warm up the fleet by ticking the schedule directly (fast, no wall clock).
fn warmup(world: &mut World, schedule: &mut Schedule, ticks: u64) {
    for _ in 0..ticks {
        schedule.run(world);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_yields_keyframe_with_vehicles_then_deltas() {
    let registry = Registry::new();
    let port = start_server(registry.clone()).await;
    let (mut world, mut schedule, grid) = build_gateway_world(&registry);

    // Warm up ~40 sim-seconds so the central grid is busy.
    warmup(&mut world, &mut schedule, 400);
    let cells = busy_cells(&world, &grid);
    assert!(
        !cells.is_empty(),
        "warmup must leave at least one occupied cell"
    );

    // Connect + subscribe to every currently-busy cell (guarantees ≥1 vehicle
    // in some subscribed cell). Give the reader task a beat to apply it.
    let mut ws = connect(port).await;
    ws.send(WsMessage::Binary(subscribe_msg(&cells)))
        .await
        .unwrap();

    // Drive publishes: tick the schedule; every 2nd tick publishes. Read frames
    // off the socket until we see a keyframe carrying ≥1 vehicle, within 2 s.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut keyframe_vehicles = 0usize;
    let mut got_keyframe = false;
    let mut delta_frames = 0usize;

    while tokio::time::Instant::now() < deadline {
        // Tick a few times to emit publishes, then poll the socket.
        warmup(&mut world, &mut schedule, 4);

        // Drain whatever is ready without blocking past a short timeout.
        while let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(20), ws.next()).await
        {
            if let WsMessage::Binary(bytes) = msg {
                let server_msg = TrafficServerMsg::decode(bytes.as_ref()).unwrap();
                for frame in &server_msg.cells {
                    assert!(
                        cells.contains(&frame.cell),
                        "received a frame for an unsubscribed cell {}",
                        frame.cell
                    );
                    if frame.keyframe {
                        got_keyframe = true;
                        keyframe_vehicles = keyframe_vehicles.max(frame.vehicles.len());
                    } else {
                        delta_frames += 1;
                    }
                }
            }
        }

        if got_keyframe && keyframe_vehicles >= 1 && delta_frames >= 1 {
            break;
        }
    }

    assert!(
        got_keyframe,
        "expected a keyframe within 2 s of subscribing"
    );
    assert!(
        keyframe_vehicles >= 1,
        "keyframe carried no vehicles ({keyframe_vehicles})"
    );
    assert!(
        delta_frames >= 1,
        "expected at least one delta frame after the keyframe"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsubscribed_cells_yield_no_frames() {
    let registry = Registry::new();
    let port = start_server(registry.clone()).await;
    let (mut world, mut schedule, grid) = build_gateway_world(&registry);

    warmup(&mut world, &mut schedule, 400);
    let busy = busy_cells(&world, &grid);
    assert!(!busy.is_empty());

    // Pick a cell id that is NOT currently busy (and stays out of the busy set
    // for the short window we watch). Search from the top of the id space.
    let total = grid.cell_count();
    let busy_set: std::collections::HashSet<u32> = busy.iter().copied().collect();
    let quiet = (0..total)
        .rev()
        .find(|c| !busy_set.contains(c))
        .expect("some cell must be unoccupied");

    let mut ws = connect(port).await;
    ws.send(WsMessage::Binary(subscribe_msg(&[quiet])))
        .await
        .unwrap();

    // Tick for a while; we must receive NO cell frame (the subscribed cell is
    // empty — no keyframe with vehicles, and no deltas). An empty keyframe on
    // subscribe is allowed, but it must never carry vehicles or name any other
    // cell.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while tokio::time::Instant::now() < deadline {
        warmup(&mut world, &mut schedule, 4);
        while let Ok(Some(Ok(msg))) =
            tokio::time::timeout(Duration::from_millis(20), ws.next()).await
        {
            if let WsMessage::Binary(bytes) = msg {
                let server_msg = TrafficServerMsg::decode(bytes.as_ref()).unwrap();
                for frame in &server_msg.cells {
                    assert_eq!(
                        frame.cell, quiet,
                        "received a frame for a cell we never subscribed to"
                    );
                    assert!(
                        frame.vehicles.is_empty(),
                        "quiet cell frame unexpectedly carried vehicles"
                    );
                }
            }
        }
    }
}

/// The default no-op hook must remain installable and harmless — the seam is
/// unchanged for callers that don't want the gateway.
#[test]
fn default_hook_is_noop() {
    let net = load_real_net();
    let (mut world, mut schedule) = shell::build_sim_with_buildings(net, 0, &load_buildings());
    world.insert_resource(SnapshotHook::default());
    for _ in 0..10 {
        schedule.run(&mut world);
    }
}
