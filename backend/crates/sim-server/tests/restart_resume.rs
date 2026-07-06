//! Opt-in restart-resume proof (Task 13 Step 3): läuft NUR mit
//! `ABUTOWN_TEST_DATABASE_URL` (lokal z.B.
//! `postgres://…@127.0.0.1:5432/abutown_m1_test`), sonst sauberer Skip.
//!
//! Prozess-intern statt Prozess-kill: Runtime 1 lässt den Welt-Loop laufen,
//! bis der Store eine Snapshot-Zeile trägt (Persist-Flush alle 50 Ticks),
//! wird gedroppt (harter Abbruch aller Tasks), dann bootet Runtime 2 mit
//! demselben Store: der Resume-Pfad (`WorldStore::read` → Snapshot →
//! `log_resume`) muss greifen — beweisbar am Verhalten: die ersten Vitals
//! des zweiten Boots tragen `world_tick >= persistierter Tick` (ein frischer
//! Boot mit derselben gepinnten Boot-Uhr würde WIEDER beim Anker starten)
//! und der `/health`-Spiegel meldet `resumed`.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use abutown_protocol::live::{LiveClientMsg, LiveServerMsg};
use demand_gen::output;
use sim_server::app::{self, WorldHealth};
use sim_server::db::connect_shared_pool;
use sim_server::world::{PersistTarget, WorldArgs, run_world};
use sim_server::world_store::WorldStore;
use winterthur_traffic::clock::{WallClock, parse_hhmm};
use winterthur_traffic::demand::TripSchedule;
use world_core::econ::EconomySeed;
use world_core::{SeedParams, SimWorld};

const ECONOMY_JSON: &str = include_str!("../../../../data/winterthur/economy.json");

const SIMWORLD: &str = r#"{
  "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
  "buildings": [
    {"id":"{H1}","usage":1,"x":0.0,"z":0.0,"area_m2":200.0,"height_m":9.0,"access_edge":0,"access_offset":5.0},
    {"id":"{W2}","usage":2,"x":900.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":5,"access_offset":50.0}
  ]}"#;

fn fixture_net_json() -> String {
    let p = format!(
        "{}/../winterthur-traffic/tests/fixtures/diamond-gateway.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

fn empty_trips(net_json: &str, tag: &str) -> TripSchedule {
    let net_hash = *blake3::hash(net_json.as_bytes()).as_bytes();
    let mut bytes = Vec::new();
    output::write_trips(&mut bytes, &net_hash, &[], &[]).unwrap();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "sim-server-restart-test-{tag}-{}.bin",
        std::process::id()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&bytes).unwrap();
    TripSchedule::load(&path, net_json.as_bytes()).unwrap()
}

fn workday_clock(at: &str) -> WallClock {
    let now = Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap();
    WallClock::new(now, Some(parse_hhmm(at).expect("test HH:MM literal")))
}

fn world_args(
    net_json: &str,
    tag: &str,
    listener: TcpListener,
    snapshot: Option<world_core::persist::WorldCoreSnapshot>,
    persist: Option<PersistTarget>,
    health: Arc<WorldHealth>,
) -> WorldArgs {
    WorldArgs {
        net_json: net_json.to_string(),
        trips: empty_trips(net_json, tag),
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
        snapshot,
        persist,
        extra_router: app::build_app(),
        listener,
        health,
    }
}

#[test]
fn world_resumes_from_persisted_snapshot_across_runtimes() {
    let Ok(url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
        return; // opt-in: ohne lokale Test-DB sauber skippen
    };
    let net_json = fixture_net_json();
    let world_id = format!("test:{}", uuid::Uuid::now_v7());

    // ── Runtime 1: laufen lassen, bis der Store eine Zeile trägt ─────────
    let rt1 = tokio::runtime::Runtime::new().unwrap();
    let persisted_tick = rt1.block_on(async {
        let pool = connect_shared_pool(&url).await.expect("connect");
        let store = Arc::new(WorldStore::with_pool(pool).await.expect("migrate"));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let args = world_args(
            &net_json,
            "run1",
            listener,
            None,
            Some(PersistTarget {
                store: Arc::clone(&store),
                world_id: world_id.clone(),
            }),
            Arc::new(WorldHealth::default()),
        );
        tokio::spawn(async move {
            run_world(args).await.expect("world runtime must boot");
        });

        // Flush-Kadenz ist 50 Ticks (5 s); grosszügige Frist fürs erste Upsert.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(snap) = store.read(&world_id).await.expect("read") {
                break snap.clock.world_tick;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no snapshot row within 20 s — persist flush broken"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    drop(rt1); // harter „Prozess-Tod": alle Tasks abgebrochen
    assert!(persisted_tick > 0, "persisted world tick must be > 0");

    // ── Runtime 2: gleicher Store → Resume-Pfad ──────────────────────────
    let rt2 = tokio::runtime::Runtime::new().unwrap();
    rt2.block_on(async {
        let pool = connect_shared_pool(&url).await.expect("connect");
        let store = Arc::new(WorldStore::with_pool(pool.clone()).await.expect("migrate"));
        let snapshot = store
            .read(&world_id)
            .await
            .expect("read")
            .expect("runtime 1 must have persisted a snapshot");
        assert_eq!(snapshot.clock.world_tick, persisted_tick);
        assert!(
            !snapshot.citizens.is_empty(),
            "snapshot must carry the seeded citizens"
        );

        let health = Arc::new(WorldHealth::default());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let args = world_args(
            &net_json,
            "run2",
            listener,
            Some(snapshot),
            Some(PersistTarget {
                store: Arc::clone(&store),
                world_id: world_id.clone(),
            }),
            Arc::clone(&health),
        );
        tokio::spawn(async move {
            run_world(args).await.expect("world runtime must resume");
        });

        // Verhaltens-Beweis: die ersten Vitals des zweiten Boots setzen die
        // Uhr FORT (>= persistierter Tick; ein frischer Boot stünde wieder
        // beim 07:00-Anker < persisted_tick), und /health meldet resumed.
        let url = format!("ws://127.0.0.1:{port}/live");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(url).await.unwrap();
        let subscribe = LiveClientMsg {
            subscribe_cells: Vec::new(),
            unsubscribe_cells: Vec::new(),
            subscribe_vitals: Some(true),
        }
        .encode_to_vec();
        ws.send(WsMessage::Binary(subscribe)).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let now = tokio::time::Instant::now();
            assert!(now < deadline, "no vitals after resume within 5 s");
            let msg = tokio::time::timeout(deadline - now, ws.next())
                .await
                .expect("vitals in time")
                .expect("stream open")
                .expect("ws read");
            let WsMessage::Binary(bytes) = msg else {
                continue;
            };
            let decoded = LiveServerMsg::decode(bytes.as_ref()).expect("valid LiveServerMsg");
            if let Some(vitals) = decoded.vitals {
                assert!(
                    vitals.world_tick >= persisted_tick,
                    "resumed clock must continue: vitals tick {} < persisted {}",
                    vitals.world_tick,
                    persisted_tick
                );
                assert!(vitals.population > 0, "population survives the restart");
                break;
            }
        }
        assert!(
            health.resumed.load(Ordering::Relaxed),
            "boot must take the log_resume path"
        );

        // Aufräumen (test:-Namespace, parallel-sicher via uuid).
        sqlx::query("DELETE FROM world_core_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .execute(&pool)
            .await
            .expect("cleanup");
    });
}
