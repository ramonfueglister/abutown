//! Task 13 integration proof (in-memory, no Postgres): the ONE sim-server
//! runtime — traffic + world-core + card-hand routes + `/live` gateway on a
//! single port — serves economy vitals to a real WebSocket client within 3 s
//! of subscribing.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use abutown_protocol::live::{LiveClientMsg, LiveServerMsg};
use demand_gen::output;
use sim_server::app::{self, WorldHealth};
use sim_server::world::{WorldArgs, run_world};
use winterthur_traffic::clock::{WallClock, parse_hhmm};
use winterthur_traffic::demand::TripSchedule;
use world_core::econ::EconomySeed;
use world_core::{SeedParams, SimWorld};

const ECONOMY_JSON: &str = include_str!("../../../../data/winterthur/economy.json");

/// 3-Gebäude-Fixture: Wohnhaus {H1} (200 m² × 3 Geschosse → 15 Bewohner,
/// im 2.5-km-Seed-Radius um (0,0)) an Edge 0, Workplace {W2} an Edge 5,
/// Unknown {X3} ohne Strassen-Zugang.
const SIMWORLD: &str = r#"{
  "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
  "buildings": [
    {"id":"{H1}","usage":1,"x":0.0,"z":0.0,"area_m2":200.0,"height_m":9.0,"access_edge":0,"access_offset":5.0},
    {"id":"{W2}","usage":2,"x":900.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":5,"access_offset":50.0},
    {"id":"{X3}","usage":0,"x":500.0,"z":500.0,"area_m2":50.0,"height_m":4.0,"access_edge":-1,"access_offset":0.0}
  ]}"#;

fn fixture_net_json() -> String {
    let p = format!(
        "{}/../winterthur-traffic/tests/fixtures/diamond-gateway.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

/// Leere Census-Tabelle (der einzige Verkehr sind Bürger), hash-gebunden ans
/// Fixture-Netz — Muster aus winterthur-traffic/tests/world_bridge.rs.
fn empty_trips(net_json: &str) -> TripSchedule {
    let net_hash = *blake3::hash(net_json.as_bytes()).as_bytes();
    let mut bytes = Vec::new();
    output::write_trips(&mut bytes, &net_hash, &[], &[]).unwrap();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "sim-server-live-ws-test-{}.bin",
        std::process::id()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&bytes).unwrap();
    TripSchedule::load(&path, net_json.as_bytes()).unwrap()
}

/// Pinned workday boot clock (Friday 2026-07-03) so the test never depends
/// on when it runs.
fn workday_clock(at: &str) -> WallClock {
    let now = Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap();
    WallClock::new(now, Some(parse_hhmm(at).expect("test HH:MM literal")))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_vitals_arrive_within_three_seconds() {
    let net_json = fixture_net_json();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let args = WorldArgs {
        net_json: net_json.clone(),
        trips: empty_trips(&net_json),
        sim_world: Arc::new(SimWorld::load(SIMWORLD).expect("fixture simworld must load")),
        economy: EconomySeed::from_json(ECONOMY_JSON).expect("economy.json must parse"),
        seed_params: SeedParams {
            center: (0.0, 0.0),
            radius_m: 2_500.0,
            residents_per_40m2: 1.0,
            seed: 42,
        },
        seed: 7,
        clock: workday_clock("07:00"),
        snapshot: None,
        persist: None, // in-memory: no Postgres in this test
        extra_router: app::build_app(),
        listener,
        health: Arc::new(WorldHealth::default()),
    };
    tokio::spawn(async move {
        run_world(args).await.expect("world runtime must boot");
    });

    // Real WS client on /live: subscribe to vitals only.
    let url = format!("ws://127.0.0.1:{port}/live");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url).await.unwrap();
    let subscribe = LiveClientMsg {
        subscribe_cells: Vec::new(),
        unsubscribe_cells: Vec::new(),
        subscribe_vitals: Some(true),
    }
    .encode_to_vec();
    ws.send(WsMessage::Binary(subscribe)).await.unwrap();

    // Within 3 s a LiveServerMsg with vitals.population > 0 must arrive
    // (live cadence is 1 Hz; the fixture seeds 15 citizens).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let now = tokio::time::Instant::now();
        assert!(now < deadline, "no vitals frame arrived within 3 s");
        let msg = tokio::time::timeout(deadline - now, ws.next())
            .await
            .expect("vitals within 3 s")
            .expect("stream must stay open")
            .expect("ws read");
        let WsMessage::Binary(bytes) = msg else {
            continue;
        };
        let decoded = LiveServerMsg::decode(bytes.as_ref()).expect("valid LiveServerMsg");
        if let Some(vitals) = decoded.vitals {
            assert!(
                vitals.population > 0,
                "vitals must carry the seeded population, got {}",
                vitals.population
            );
            assert_eq!(
                vitals.audit_ok, 1,
                "audit must be ok while the process lives"
            );
            assert!(vitals.world_tick > 0, "world clock must be running");
            assert!(
                !vitals.prices.is_empty(),
                "vitals must carry the seeded market prices"
            );
            break;
        }
    }
}
