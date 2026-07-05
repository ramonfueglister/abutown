//! Opt-in-Integrationstest für den world_core_snapshots-Store: läuft NUR,
//! wenn `ABUTOWN_TEST_DATABASE_URL` gesetzt ist (Muster der bestehenden
//! opt-in-Tests, z.B. `db::tests::shared_pool_connects_and_pings`), sonst
//! sauberer Skip. Lokal:
//! `ABUTOWN_TEST_DATABASE_URL=postgres://…@127.0.0.1:5432/abutown_m1_test`.

use sim_server::db::connect_shared_pool;
use sim_server::world_store::WorldStore;
use world_core::persist::{
    CitizenSnap, EconSnap, PersistedWalk, WORLD_SNAPSHOT_VERSION, WorldCoreSnapshot,
};
use world_core::{CitizenState, WorldClock};

fn snapshot(tick: u64) -> WorldCoreSnapshot {
    WorldCoreSnapshot {
        version: WORLD_SNAPSHOT_VERSION,
        clock: WorldClock { world_tick: tick },
        citizens: vec![
            CitizenSnap {
                id: 0,
                home: 1,
                work: 0,
                state: CitizenState::AtHome,
                active_trip: None,
            },
            CitizenSnap {
                id: 1,
                home: 1,
                work: 0,
                state: CitizenState::Commuting {
                    trip: world_core::TripKind::ToWork,
                },
                active_trip: Some(PersistedWalk {
                    arrive_tick: tick + 715,
                    dest_building: 0,
                }),
            },
        ],
        building_states: vec![(2, world_core::BuildingLifecycle::Vacant)],
        econ: EconSnap::default(),
    }
}

#[tokio::test]
async fn write_read_roundtrip_upsert_and_unknown_world() {
    let Ok(url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
        return; // opt-in: ohne lokale Test-DB sauber skippen
    };
    let pool = connect_shared_pool(&url).await.expect("connect");
    let store = WorldStore::with_pool(pool.clone()).await.expect("migrate");
    let world_id = format!("test:{}", uuid::Uuid::now_v7());

    // write → read Roundtrip: Snapshot identisch, tick + version stimmen.
    let snap = snapshot(500);
    store.write(&world_id, 500, &snap).await.expect("write");
    let read = store.read(&world_id).await.expect("read").expect("row");
    assert_eq!(read, snap);
    assert_eq!(read.version, WORLD_SNAPSHOT_VERSION);
    assert_eq!(read.clock.world_tick, 500);
    let (tick, schema_version): (i64, i32) =
        sqlx::query_as("SELECT tick, schema_version FROM world_core_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .fetch_one(&pool)
            .await
            .expect("row columns");
    assert_eq!(tick, 500);
    assert_eq!(schema_version, WORLD_SNAPSHOT_VERSION as i32);

    // Fremde world_id → None (frische Welt).
    let other = format!("test:{}", uuid::Uuid::now_v7());
    assert!(store.read(&other).await.expect("read").is_none());

    // Zweiter write mit höherem tick überschreibt (Upsert, eine Zeile).
    let snap2 = snapshot(1_000);
    store.write(&world_id, 1_000, &snap2).await.expect("write2");
    let read2 = store.read(&world_id).await.expect("read").expect("row");
    assert_eq!(read2, snap2);
    assert_eq!(read2.clock.world_tick, 1_000);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM world_core_snapshots WHERE world_id = $1")
            .bind(&world_id)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(rows, 1, "upsert must not create a second row");

    // Aufräumen (test:-Namespace, parallel-sicher via uuid).
    sqlx::query("DELETE FROM world_core_snapshots WHERE world_id = $1")
        .bind(&world_id)
        .execute(&pool)
        .await
        .expect("cleanup");
}
