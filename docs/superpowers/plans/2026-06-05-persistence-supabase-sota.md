# Durable Supabase Persistence (SOTA) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** One shared, self-reclaiming sqlx pool across all 6 Postgres stores (transaction-pooler-correct) + graceful health degradation, so the server persists to Supabase indefinitely and a transient persist hiccup never blanks the map.

**Architecture:** `sqlx::PgPool` is internally `Arc`-backed — cloning it shares ONE underlying connection pool. So a single pool is built once in `build_app_from_config` and `.clone()`d into each store; store structs and all query sites stay unchanged. The pool is tuned for the Supabase pooler (bounded, `idle_timeout`/`max_lifetime` to release slots, `statement_cache_capacity(0)` for the transaction pooler). Separately, the liveness state machine + health gate + frontend gate are changed so only a sustained outage (`Stale`) hard-blocks; a transient failure (`Degraded`) renders the map with a non-blocking banner that auto-clears.

**Tech Stack:** Rust (sqlx 0.8, axum), TypeScript/Vite frontend (@bufbuild/protobuf), Supabase Postgres.

**Spec:** `docs/superpowers/specs/2026-06-05-persistence-supabase-sota-design.md`

---

## Verified facts (pinned against `origin/main` 63fab8c, worktree `abutown-vtraders`)

**Cargo discipline (MANDATORY):** every cargo via the serial wrapper on the isolated tmp target, from the worktree root, scoped, background+poll, clear orphans first:
```bash
cd /Users/ramonfuglister/Coding/abutown-vtraders
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server <FILTER>
# fmt: scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
```
Never `--workspace --all-targets`. `pgrep -f cargo` and clear orphans before launching.

### KEY INSIGHT — share via `PgPool::clone`, NOT `Arc<PgPool>`
`sqlx::Pool` is `Arc`-internally; `PgPool::clone()` returns a handle to the **same** pool/connections. So: store structs keep `pool: PgPool` **unchanged**, all `.execute(&self.pool)` / `.fetch_*(&self.pool)` sites **unchanged**, `pool_for_test()` unchanged. Only the *constructor* changes (take a pool instead of dialing one). This is the idiomatic sqlx shared-pool pattern and the lowest-risk refactor.

### The 6 stores (each currently dials its OWN pool — `PgPoolOptions::new().max_connections(5).connect(url)`)
- `postgres_events.rs`: `PostgresWorldEventStore { pool: PgPool }` (:28); `connect(database_url) -> Result<Self, WorldEventStoreError>` (:34) runs `WORLD_EVENTS_MIGRATION` then `CHUNK_RECOVERY_MIGRATION`; **NO `pool_for_test()`**.
- `postgres_snapshots.rs`: `PostgresChunkSnapshotStore { pool: PgPool, world_id: WorldId }` (:61); `connect(database_url, world_id, _compatibility) -> Result<Self, ChunkSnapshotStoreError>` (:68) runs `CHUNK_SNAPSHOTS_MIGRATION`+`SNAPSHOT_COMPATIBILITY_MIGRATION`; **NO `pool_for_test()`**.
- `postgres_mobility.rs`: `PostgresMobilitySnapshotStore { pool: PgPool }` (:23); `connect(database_url)` (:29) runs migrations THEN imperative `migrate_legacy_agent_birth_ticks(&pool).await?` (:60); HAS `pool_for_test()` (:65).
- `postgres_economy.rs`: `PostgresEconomySnapshotStore { pool: PgPool }` (:12); `connect(database_url)` (:18); HAS `pool_for_test()` (:39).
- `postgres_economy_events.rs`: `PostgresEconomyEventStore { pool: PgPool }` (:13); `connect(database_url)` (:19); HAS `pool_for_test()` (:40).
- `card_hand.rs`: `CardHandStoreInner::{Memory(...), Postgres(PgPool)}` (:56); `CardHandStore::postgres(database_url)` (:73) runs `CARD_HAND_MIGRATION`, error type `CardHandError::Database(String)`.
- sqlx = `0.8` with `tls-rustls` (workspace Cargo.toml:29). `PgConnectOptions::statement_cache_capacity`, `PgPoolOptions::{idle_timeout,max_lifetime,test_before_acquire,acquire_timeout,min_connections}` are all available.

### app wiring (`app/mod.rs`)
- Submodule decls at :50-54 (add `mod db;` here). `SNAPSHOT_INTERVAL = 5s`, `MOBILITY_PERSISTENCE_FRESHNESS_WINDOW = 15s` (:56-60).
- `build_app_from_config` :422-461 — the 6 `connect()` calls + `hydrate_from_stores` (consumes 4 boxed stores, returns 3 back) + `AppState::new_with_stores`.
- `health_response_for_state` :531-544 — sets `health.ok = health.ok && runtime_agents_ok && !matches!(persistence.status, Degraded | Stale)`. ← the gate to decouple.
- `persist_snapshots_once` :1285-1425 — liveness hooks: `begin_attempt` :1324, `record_failure` :1341/:1367, `record_success` :1378.
- `ServerConfig` (config.rs) parses env into fields incl. `database_url`; **no pool-size knob exists** (add `ABUTOWN_DB_MAX_CONNECTIONS` read inside db.rs via `std::env`, no ServerConfig change needed).

### liveness (`persistence_liveness.rs`)
- `MobilityPersistenceHealthStatus { Starting, Healthy, Degraded, Stale }` (:4, derives Copy).
- `snapshot_at(now)` :107-139 — the status match (order load-bearing):
```rust
let status = match (inner.last_attempt, inner.last_success, inner.consecutive_failures) {
    (None, None, _) => Starting,
    (_, _, failures) if failures > 0 => Degraded,   // <- ANY failure => Degraded, ignores freshness
    (_, Some(_), 0) if freshness.is_some_and(|age| age <= self.freshness_window) => Healthy,
    (_, Some(_), 0) => Stale,
    _ => Degraded,
};
```
- Existing tests :168-237: `starts_before_first_attempt`, `success_is_healthy_until_freshness_window_expires` (Healthy@14.9s, Stale@16.1s, window 15s), `failure_after_attempt_is_degraded_and_redacted`, `failure_redacts_all_supabase_secret_tokens`. Helper `at(ms)=UNIX_EPOCH+ms`; window `Duration::from_secs(15)`.

### frontend gate (`src/`)
- `backendGate.ts`: `isAcceptableBackendPersistenceHealth` :98-100 accepts only `undefined|'starting'|'healthy'`; `requireBackend` :52-67 (fetch `/health`, throws `BackendHealthError(payload)` when not ok); `formatBackendHealthError` :91.
- `mobilityProtocol.ts`: `BackendPersistenceStatusDto = 'starting'|'healthy'|'degraded'|'stale'` (:140); `BackendPersistenceHealthDto` (:142); `healthResponseFromProto` (:491).
- `appRuntime.ts`: `startAppRuntime` :77-116 calls `requireBackend` ONCE at :81; on throw → `renderBackendRequired`. **No `/health` re-poll exists.**
- `backendRequiredView.ts`: `renderBackendRequired` :19-47 (full-screen `backend-required-panel`).
- `main.ts`: boot :116-160; `renderBackendRequired` wrapper :162; `installRuntimeDiagnostics` :451.
- Tests: `tests/backend/backendGate.test.ts:127-231` (currently fails-closed on `degraded`/`stale`).

### Gotchas
- `&self.pool` query sites stay valid (field stays `PgPool`). Do NOT introduce `Arc<PgPool>`.
- `PostgresMobilitySnapshotStore::with_pool` MUST still run `migrate_legacy_agent_birth_ticks(&pool)` after the structural migrations.
- Stores are constructed SEQUENTIALLY in `build_app_from_config` (each `await`ed) → their idempotent `CREATE TABLE IF NOT EXISTS` migrations against the shared pool do not race; no extraction to a central migration runner needed.
- `card_hand` error is `CardHandError::Database`, not `…StoreError::unavailable` — keep per-store error mapping.
- No snapshot schema/serde change in this plan → **no `DELETE FROM economy_snapshots` needed.**
- The transaction-pooler `.env` URL switch (`:5432`→`:6543`) is an OPERATOR/config step, not code — the code is made pooler-mode-agnostic via `statement_cache_capacity(0)`.

---

## Part 1 — One shared, self-reclaiming pool

### Task 1: `db.rs` — the shared pool factory

**Files:** Create `backend/crates/sim-server/src/db.rs`; Modify `backend/crates/sim-server/src/app/mod.rs:50-54` (add `mod db;`).

- [ ] **Step 1: Write `db.rs`:**
```rust
//! Single shared Postgres pool for all stores. sqlx Pool is Arc-internal, so one
//! pool cloned into each store shares the same bounded connection set. Tuned for the
//! Supabase pooler: bounded, self-reclaiming (idle/lifetime), and prepared-statement
//! caching disabled so it is correct on the TRANSACTION pooler (:6543) as well as session.
use std::time::Duration;
use std::str::FromStr;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

/// Default pool ceiling; override with `ABUTOWN_DB_MAX_CONNECTIONS`. Sized well under
/// the Supabase pooler client limit so all stores together never exhaust it.
const DEFAULT_MAX_CONNECTIONS: u32 = 8;

pub async fn connect_shared_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let max_connections = std::env::var("ABUTOWN_DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_MAX_CONNECTIONS);
    // statement_cache_capacity(0): REQUIRED on the Supabase transaction pooler
    // (multiplexed backends cannot reuse prepared statements); harmless on session mode.
    let connect_options = PgConnectOptions::from_str(database_url)?.statement_cache_capacity(0);
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Some(Duration::from_secs(30)))
        .max_lifetime(Some(Duration::from_secs(900)))
        .test_before_acquire(true)
        .connect_with(connect_options)
        .await
}
```

- [ ] **Step 2: Add `mod db;`** to `app/mod.rs` near :50 (alongside `mod base_world_response;` / `mod proto_convert;`), as `mod db;` + `use db::connect_shared_pool;` (or reference as `db::connect_shared_pool`).

- [ ] **Step 3: Write an opt-in integration test** `backend/crates/sim-server/tests/shared_pool.rs` (or in `db.rs` `#[cfg(test)]`) — gated on `ABUTOWN_TEST_DATABASE_URL` (skip if unset, like the existing store round-trip tests):
```rust
#[tokio::test]
async fn shared_pool_connects_and_pings() {
    let Ok(url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else { return; };
    let pool = sim_server::db::connect_shared_pool(&url).await.expect("connect");
    let one: i32 = sqlx::query_scalar("SELECT 1").fetch_one(&pool).await.expect("ping");
    assert_eq!(one, 1);
    // cloning shares the SAME pool (size invariant): both handles see one pool.
    let clone = pool.clone();
    assert_eq!(pool.size(), clone.size());
}
```
(Make `db` reachable from the test: `pub mod db;` if testing via the crate's public surface, or keep `mod db;` private and put the test inside `db.rs`. Pick whichever matches the crate's existing test exposure; the mobility store's `pool_for_test` pattern shows the convention.)

- [ ] **Step 4: Build + run (opt-in test no-ops without the env):**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server shared_pool
```
Expected: compiles + passes (test skips if `ABUTOWN_TEST_DATABASE_URL` unset; runs the ping if set).

- [ ] **Step 5: fmt + commit:**
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all
git add backend/crates/sim-server/src/db.rs backend/crates/sim-server/src/app/mod.rs backend/crates/sim-server/tests/shared_pool.rs
git commit -m "feat(server): shared Supabase-tuned Postgres pool factory (db.rs)"
```

### Task 2: Refactor the 5 simulation stores to `with_pool(pool: PgPool)`

**Files:** Modify `postgres_events.rs`, `postgres_snapshots.rs`, `postgres_mobility.rs`, `postgres_economy.rs`, `postgres_economy_events.rs` (all under `backend/crates/sim-server/src/`).

For EACH store, the mechanical transform (struct field + query sites + pool_for_test UNCHANGED):
- Replace the constructor `pub async fn connect(<url + extra args>) -> Result<Self, …Error>` with `pub async fn with_pool(pool: PgPool, <extra args>) -> Result<Self, …Error>`:
  - DELETE the `PgPoolOptions::new().max_connections(5).connect(database_url).await.map_err(...)?` block.
  - KEEP all migration execution, but run it against the passed `&pool` (same `include_str!` consts, same `.split(';')`/batch loops, same `.map_err(|e| …Error::unavailable(e.to_string()))`).
  - `Ok(Self { pool, <world_id if present> })`.
- For `postgres_mobility.rs`: KEEP the imperative `migrate_legacy_agent_birth_ticks(&pool).await?` after the structural migrations.
- For `postgres_snapshots.rs`: signature becomes `with_pool(pool: PgPool, world_id: WorldId, _compatibility: SnapshotCompatibility)`.
- ADD `pub fn pool_for_test(&self) -> &sqlx::PgPool { &self.pool }` to `postgres_events.rs` and `postgres_snapshots.rs` (mobility/economy/economy_events already have it).

- [ ] **Step 1: Apply the transform to all 5 files** (verify each `connect` body against the file before editing; preserve the exact migration consts + error types).
- [ ] **Step 2: (call sites compile in Task 4 — for now this task leaves `build_app_from_config` referencing the old `connect`; to keep the crate compiling, do Task 2 + Task 4 together OR temporarily keep `connect` as a thin wrapper `connect(url) { Self::with_pool(connect_shared_pool(url).await?, …).await }`).** Prefer: implement Task 2 and Task 4 in the same working session so the crate compiles at the commit boundary. Build:
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server
```
- [ ] **Step 3:** Run the existing opt-in store round-trip tests (they use `pool_for_test`) to confirm the stores still work end-to-end against `ABUTOWN_TEST_DATABASE_URL`:
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server postgres
```
- [ ] **Step 4: fmt + commit** the 5 store files.

### Task 3: Refactor `CardHandStore` onto the shared pool

**Files:** Modify `backend/crates/sim-server/src/card_hand.rs:56-94`.

- [ ] **Step 1:** Replace `pub async fn postgres(database_url: &str)` with `pub async fn with_pool(pool: PgPool) -> Result<Self, CardHandError>` — drop the `PgPoolOptions` block, keep `CARD_HAND_MIGRATION` execution against `&pool` (keep `CardHandError::Database(e.to_string())` mapping), wrap `Arc::new(CardHandStoreInner::Postgres(pool))`.
- [ ] **Step 2:** build + commit.

### Task 4: Wire the shared pool through `build_app_from_config`

**Files:** Modify `backend/crates/sim-server/src/app/mod.rs:422-461`.

- [ ] **Step 1:** Create the pool ONCE, pass clones:
```rust
let pool = db::connect_shared_pool(&config.database_url).await?;
let event_store    = PostgresWorldEventStore::with_pool(pool.clone()).await?;
let snapshot_store = PostgresChunkSnapshotStore::with_pool(
    pool.clone(),
    abutown_protocol::WorldId(base_world.world_id().to_owned()),
    base_world.snapshot_compatibility(),
).await?;
let mobility_snapshot_store = PostgresMobilitySnapshotStore::with_pool(pool.clone()).await?;
let economy_snapshot_store  = PostgresEconomySnapshotStore::with_pool(pool.clone()).await?;
let economy_event_store     = PostgresEconomyEventStore::with_pool(pool.clone()).await?;
let card_hands              = CardHandStore::with_pool(pool.clone()).await?;
```
(The `hydrate_from_stores` call + `AppState::new_with_stores` below are UNCHANGED — they still take the boxed trait objects.)
- [ ] **Step 2: Build the whole crate — expect OK:**
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server
```
- [ ] **Step 3:** Remove any temporary `connect` wrappers left from Task 2. Run `rg -n "PgPoolOptions::new\(\)" backend/crates/sim-server/src` — expect the ONLY remaining match to be inside `db.rs`. fmt + commit.

---

## Part 2 — Graceful degradation (a transient hiccup never blanks the map)

### Task 5: Liveness — tolerate transient failures; reserve Stale for real outage

**Files:** Modify `backend/crates/sim-server/src/persistence_liveness.rs:107-139` + tests :168-237.

- [ ] **Step 1: Write the new failing tests** (extend the module): success-then-1-failure while fresh stays **Healthy**; success-then-3-failures while fresh is **Degraded**; success older than the window is **Stale**; attempted-but-never-succeeded is **Stale**. NOTE the existing `failure_after_attempt_is_degraded_and_redacted` (`persistence_liveness.rs:197`) does `begin_attempt` + `record_failure` with **no prior success** → under the new match that is `(Some(_), None) => Stale` (a backend that has never persisted IS a real outage). **Rename it `failure_without_prior_success_is_stale_and_redacted`, change its status assertion from `Degraded` to `Stale`, and KEEP both redaction assertions verbatim.** (The Degraded path is now covered by the new `sustained_failures_after_success_are_degraded_not_stale` test below, which needs a prior success + >2 failures.)
```rust
#[test]
fn transient_failures_after_success_stay_healthy_within_tolerance() {
    let t = MobilityPersistenceLiveness::new(Duration::from_secs(15));
    let a = t.begin_attempt("abutopia".into(), 1, at(0));
    t.record_success(a, at(0));
    let a = t.begin_attempt("abutopia".into(), 2, at(1_000));
    t.record_failure(a, "boom", at(1_000));               // 1 failure, fresh success
    assert_eq!(t.snapshot_at(at(2_000)).status, MobilityPersistenceHealthStatus::Healthy);
}
#[test]
fn sustained_failures_after_success_are_degraded_not_stale() {
    let t = MobilityPersistenceLiveness::new(Duration::from_secs(15));
    let a = t.begin_attempt("abutopia".into(), 1, at(0));
    t.record_success(a, at(0));
    for ms in [1_000, 2_000, 3_000] { let a = t.begin_attempt("abutopia".into(), 2, at(ms)); t.record_failure(a, "boom", at(ms)); }
    assert_eq!(t.snapshot_at(at(4_000)).status, MobilityPersistenceHealthStatus::Degraded); // >2 failures, still fresh
}
#[test]
fn stale_success_is_stale_even_with_no_failures() {
    let t = MobilityPersistenceLiveness::new(Duration::from_secs(15));
    let a = t.begin_attempt("abutopia".into(), 1, at(0));
    t.record_success(a, at(0));
    assert_eq!(t.snapshot_at(at(16_001)).status, MobilityPersistenceHealthStatus::Stale);
}
```
- [ ] **Step 2: Implement the new match** (replace :112-127). Put the const at MODULE level (top of the file, not inside the method):
```rust
/// Consecutive failed persist cycles tolerated while a recent success still holds
/// before the status drops from Healthy to Degraded.
const PERSIST_FAILURE_TOLERANCE: u32 = 2;
```
Then inside `snapshot_at`:
```rust
let fresh = freshness.is_some_and(|age| age <= self.freshness_window);
let status = match (inner.last_attempt, inner.last_success) {
    (None, None) => MobilityPersistenceHealthStatus::Starting,
    (_, Some(_)) if fresh && inner.consecutive_failures <= PERSIST_FAILURE_TOLERANCE =>
        MobilityPersistenceHealthStatus::Healthy,
    (_, Some(_)) if fresh => MobilityPersistenceHealthStatus::Degraded, // recent success but currently failing > tolerance
    (_, Some(_)) => MobilityPersistenceHealthStatus::Stale,             // last success older than the window
    (Some(_), None) => MobilityPersistenceHealthStatus::Stale,          // attempted, never succeeded → real outage
};
```
**Exhaustiveness:** these five arms ARE exhaustive — the unguarded `(_, Some(_))` arm covers `(None,Some)`+`(Some,Some)`, `(None,None)` and `(Some(_),None)` cover the rest. Do **NOT** add a trailing `_ =>` catchall: it would be an **unreachable pattern** and fail `clippy -D warnings`.
- [ ] **Step 3:** update `success_is_healthy_until_freshness_window_expires` if its 0-failure path still holds (it does: 0 failures + fresh = Healthy; +stale = Stale). Run:
```bash
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server persistence_liveness
```
Expected: all pass. fmt + commit.

### Task 6: Health gate — `ok=false` only on Stale

**Files:** Modify `backend/crates/sim-server/src/app/mod.rs:536-541`.

- [ ] **Step 1:** Change the gate so Degraded no longer forces `ok=false` (Degraded still reported in `health.persistence`):
```rust
health.ok = health.ok
    && runtime_agents_ok
    && persistence.status != MobilityPersistenceHealthStatus::Stale;
```
- [ ] **Step 2: Reconcile the existing health test.** `backend/crates/sim-server/src/app/tests.rs` has `failing_mobility_write_marks_health_degraded_with_redacted_error`, which asserts `!health.ok` and `persistence.status == Degraded`. READ it and reconcile against Task 5's new liveness:
  - If its scenario is a failing mobility write with **no prior success**, the new status is **Stale** → `!health.ok` STILL holds, but the status assertion must change `Degraded` → `Stale` (rename the test to `…marks_health_stale_…`).
  - If you make it a **transient** case (record a success, then >2 failures while fresh = Degraded), then `health.ok` is now **true** (Degraded no longer blocks) — update that assertion to `assert!(health.ok, …)` and keep `status == Degraded`.
  - Cleanest: keep the existing test as the **Stale → !health.ok** case (matching its current setup) AND add a new `degraded_persistence_keeps_health_ok` test (prior success + 3 failures → `health.ok == true`, `status == Degraded`) so both branches of the decoupled gate are covered.
  Build + run the app tests; fmt + commit.

### Task 7: Frontend gate accepts `degraded` (non-blocking)

**Files:** Modify `src/backend/backendGate.ts:98-100`; Modify `tests/backend/backendGate.test.ts:127-231`.

- [ ] **Step 1: Flip the failing tests** — `degraded` must now be ACCEPTED (no throw); only `stale` (and HTTP error / `ok:false` for non-persistence reasons) throws. Update the two `degraded` cases in `backendGate.test.ts` to assert `requireBackend` resolves and returns the payload; keep the `stale` case throwing.
- [ ] **Step 2: Implement:**
```ts
function isAcceptableBackendPersistenceHealth(value: BackendPersistenceHealthDto | undefined): boolean {
  return value === undefined || value.status === 'starting' || value.status === 'healthy' || value.status === 'degraded';
}
```
- [ ] **Step 3:**
```bash
npm run typecheck && npx vitest run tests/backend/backendGate
```
Expected: PASS. Commit.

### Task 8: Live `/health` re-poll + non-blocking degraded banner

**Files:** Create `src/app/persistenceBanner.ts`; Modify `src/main.ts` (boot wiring + a periodic poll); Test `tests/app/persistenceBanner.test.ts`.

- [ ] **Step 1: Write the pure banner module + test** — a DOM-light, idempotent banner controller (no full-screen takeover):
```ts
// persistenceBanner.ts
export function setPersistenceBanner(doc: Document, status: 'healthy'|'degraded'|'starting'|'stale'|'down'): void {
  const existing = doc.querySelector('[data-persistence-banner]');
  if (status === 'healthy' || status === 'starting') { existing?.remove(); return; }
  const el = (existing as HTMLElement) ?? doc.createElement('div');
  el.setAttribute('data-persistence-banner', 'true');
  el.className = 'persistence-banner';
  el.textContent = status === 'degraded'
    ? 'Persistenz vorübergehend verzögert — Welt läuft, letzte Schreibvorgänge werden wiederholt.'
    : 'Persistenz offline — Daten werden derzeit nicht gespeichert.';
  if (!existing) doc.body.appendChild(el);
}
```
Test: applying `degraded` adds one banner; applying `healthy` removes it; repeated `degraded` does not duplicate.
- [ ] **Step 2: Add a periodic `/health` poll in `main.ts`** after boot (reuse `requireBackend`'s fetch/decode or a lighter `fetchBackendHealth`): every 5s, read persistence status and call `setPersistenceBanner`. On a `stale`/HTTP-error transition, escalate to the existing `renderBackendRequired` (full block) — but a `degraded` only shows the banner. Clear on recovery. (Mirror how `installRuntimeDiagnostics`/the boot loop are wired; do not add a second WebSocket.)
- [ ] **Step 3:**
```bash
npm run typecheck && npx vitest run tests/app/persistenceBanner
```
Expected: PASS. Commit.

---

## Part 3 — Verify against the REAL Supabase + browser-smoke

### Task 9: Real-Supabase verification

- [ ] **Step 1:** With `ABUTOWN_TEST_DATABASE_URL` set to the real Supabase URL, run the opt-in store + shared-pool tests:
```bash
ABUTOWN_TEST_DATABASE_URL="$DATABASE_URL" TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server postgres
```
(Source the real URL from the deploy `.env`; the transaction-pooler `:6543` endpoint is the target.)
- [ ] **Step 2: Connection-count proof** — boot the server (transaction pooler URL) and observe `SELECT count(*) FROM pg_stat_activity WHERE usename = current_user;` drops to ≤ pool max (8) and falls during idle (idle_timeout reclaims). Document the before/after.
- [ ] **Step 3: Canonical persistence smoke** (the real-Supabase gate):
```bash
node scripts/smoke-mobility-persistence.mjs
```
Expected: `/health` ok=true, persistence Healthy, exactly one `mobility_snapshots` row, sustained.

### Task 10: MANDATORY browser-smoke (frontend↔backend gate boundary)

**Files:** Create `scripts/smoke-persistence-degraded.mjs`; Modify `package.json`.

- [ ] **Step 1:** Adapt `scripts/smoke-economy-markets.mjs`/`smoke-mobility-persistence.mjs` patterns: boot the dev stack (isolated ports), force a transient Degraded (e.g. start with `ABUTOWN_DB_MAX_CONNECTIONS=1` + a concurrent slow query, or briefly point at an unreachable DB then restore), and assert via headless chromium that **the canvas renders** (`canvas.dataset.ready==='true'`, no `backend-required-panel`) and the `[data-persistence-banner]` appears, then clears on recovery. Assert a sustained outage (Stale) DOES show `renderBackendRequired`.
- [ ] **Step 2:** add `"smoke:persistence-degraded": "node scripts/smoke-persistence-degraded.mjs"`; run `npm run build && npm run smoke:persistence-degraded`. This is the acceptance gate for Part 2.

---

## Final gate (before push)
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-server -- -D warnings
TMPDIR=/tmp/abutown-vtraders-tmp CARGO_TARGET_DIR=/tmp/abutown-vtraders-target \
  scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
npm run typecheck && npx vitest run && npm run build && npm run test:e2e
```
All green → finishing-a-development-branch → PR. PR body: the RCA + that the operator must flip `.env DATABASE_URL` to the `:6543` transaction pooler at deploy; no snapshot DELETE needed.

---

## Self-review
- **Spec coverage:** shared pool (T1-T4), graceful degradation liveness/gate/frontend/banner+repoll (T5-T8), real-Supabase + browser-smoke verification (T9-T10). ✓
- **Key correction vs spec/grounding:** use `PgPool::clone` (sqlx pool is Arc-internal) NOT `Arc<PgPool>` — store fields + query sites unchanged, far lower risk. Documented in Verified-facts.
- **Placeholder scan:** db.rs, the liveness match, the gate line, the backendGate fn, the banner are verbatim. T2 is a mechanical transform with the exact per-store deltas + anchors (implementer reads each `connect` body). T8's main.ts re-poll wiring references the established boot pattern (no second socket).
- **Type consistency:** `connect_shared_pool -> PgPool`; every store `with_pool(pool: PgPool, …)`; `PgPool::clone()` at all 6 call sites; `MobilityPersistenceHealthStatus::Stale` used identically in T5/T6; `'degraded'` literal in T7/T8 matches `BackendPersistenceStatusDto`.
- **No-DELETE:** no snapshot schema/serde change → no economy_snapshots migration. ✓
