# Durable Supabase Persistence (SOTA 2026) — Design Spec

**Date:** 2026-06-05
**Status:** Design → plan → implement (durable fix; user runs the real stack on remote Supabase)
**Branch:** `plan/persistence-supabase-sota` (off `origin/main` `63fab8c`)

## Goal

Make the sim-server persist **reliably and indefinitely** to the remote Supabase
Postgres, and make the frontend **never blank the map** because of a transient
persistence hiccup. State-of-the-art connection architecture for a Rust/sqlx app
against Supabase, plus graceful health degradation.

## Root cause (evidence-backed)

Two independent defects produced the observed "Backend required / persistence
stale" blank map.

### Defect A — connection-pool exhaustion (the persist failure root)
The server opens **6 independent sqlx `PgPool`s**, each
`PgPoolOptions::new().max_connections(5)` with **no other tuning** — verified at
`postgres_snapshots.rs:73`, `postgres_mobility.rs:30`, `postgres_economy.rs:19`,
`postgres_events.rs:35`, `postgres_economy_events.rs:20`, `card_hand.rs:74`. That
is **up to 30 connections** to the Supabase pooler.

With sqlx 0.8 defaults `idle_timeout=None` and `max_lifetime=None`, a connection,
once opened, is **never closed**. On the Supabase **session** pooler (`:5432`)
each client connection pins a dedicated server backend slot for its whole
lifetime. So the cumulative high-water mark of opened connections **permanently**
consumes pooler slots; small-tier Supabase caps pooler clients well below 30 →
new acquires block → with the 30 s default `acquire_timeout` they surface as
`pool timed out while waiting for an open connection` (observed in
`persist_snapshots_once`, `app/mod.rs:1367`). **Structural, not load-dependent** —
it will always eventually tip; repeated server restarts (each opening ~25
never-released connections) accelerate it.

### Defect B — one failure → blank map (no graceful degradation, no re-poll)
A single failed persist → `consecutive_failures > 0` →
`persistence_liveness.rs:121` reports **Degraded** on the FIRST failure →
`health_response_for_state` (`app/mod.rs:536`) sets `health.ok=false` for Degraded
**or** Stale → frontend `backendGate.ts:98` accepts only `starting|healthy` →
`requireBackend` throws → `renderBackendRequired` full-screen takeover. And
`requireBackend` runs **once at boot** (`appRuntime.ts:81`, no re-poll), so one
transient hiccup blanks the map until a manual reload.

### Meta-cause
CI and the isolated smokes use a fresh throwaway DB + clean checkout, so they
never exercised the real Supabase connection ceiling or a stale running
deployment. The fix must be verified against the **real** Supabase, not just CI.

## SOTA design

### Part 1 — one shared, self-reclaiming pool on the transaction pooler
- New module `backend/crates/sim-server/src/db.rs` exposing
  `connect_shared_pool(database_url) -> Result<Arc<PgPool>, sqlx::Error>` building
  **one** pool via `PgConnectOptions` + `PgPoolOptions`:
  - `max_connections`: default **8**, override `ABUTOWN_DB_MAX_CONNECTIONS`.
  - `min_connections(0)` — never pin idle slots.
  - `acquire_timeout(10s)` — fail fast (vs the 30 s default that masks exhaustion).
  - `idle_timeout(Some(30s))` + `max_lifetime(Some(900s))` — **the levers that
    release pooler slots**; the `None`/`None` defaults never do.
  - `test_before_acquire(true)` — drop dead/recycled connections.
  - **`statement_cache_capacity(0)`** on the `PgConnectOptions` — **mandatory** for
    the Supabase **transaction** pooler (`:6543`), which cannot reuse prepared
    statements across multiplexed backends. Safe on session mode too; makes the
    code pooler-mode-agnostic (SOTA: works on the recommended transaction pooler).
- `run_migrations(&pool)` in `db.rs` runs all five stores' idempotent
  `CREATE TABLE IF NOT EXISTS` DDL **once, sequentially**, before stores are
  built (removes the concurrent-DDL race a shared pool would otherwise create).
- Refactor the 5 sim stores + `CardHandStore` to take `pool: Arc<PgPool>` and
  **only** hold it (no `PgPoolOptions`, no DDL in their constructors). All
  `.execute(&self.pool)` / `.fetch_*` sites are unchanged (`&Arc<PgPool>` derefs
  to `&PgPool`). Keep `pool_for_test()`.
- `build_app_from_config` (`app/mod.rs:422`): `connect_shared_pool` once, migrate
  once, build all 6 stores from `Arc::clone`. Total connections bounded at 8,
  self-reclaiming. `hydrate_from_stores` unchanged (still gets `Box<dyn …>`).
- **Config (not code):** `.env` `DATABASE_URL` moves to the **transaction
  pooler** endpoint (`…pooler.supabase.com:6543`, same host/user, port 5432→6543).
  Keep `sslmode=verify-full` (correctness; cost amortized by pooling + fewer
  conns). `PGSSLROOTCERT` must be an absolute readable path. Document in
  `.env.example`.

### Part 2 — graceful degradation (decouple render from write-health)
- `persistence_liveness.rs`: tolerate one failed cycle — report **Degraded** only
  once `consecutive_failures` exceeds a small threshold (`> 2`); reserve **Stale**
  for "no success within the freshness window" (genuinely-prolonged outage).
- `app/mod.rs` `health_response_for_state`: `health.ok` is forced false by
  persistence **only when Stale**, not Degraded. Degraded is still reported in
  `health.persistence` so the client can show it.
- Frontend `backendGate.ts`: accept `degraded` as non-blocking
  (`starting|healthy|degraded` → ok; only `stale` blocks). Surface `degraded` as a
  **non-blocking banner**, not the full-screen takeover. Add a lightweight
  periodic `/health` re-poll so the banner auto-clears on recovery (no manual
  reload). Keep the hard takeover only for true backend-down / HTTP error / Stale.

## Scope & risk
New `db.rs` (~70 lines); mechanical edits to 5 store files + `card_hand.rs`
(`PgPool`→`Arc<PgPool>`, thinner `connect`, drop DDL); `app/mod.rs` wiring +
health gate; `persistence_liveness.rs` tolerance + test; frontend `backendGate.ts`
+ a small re-poll. **No schema/serde change → no `DELETE FROM economy_snapshots`.**
Trait-object boundary unchanged (no `+ Sync` churn). Medium scope, low data risk.

## Verification (against the REAL Supabase — not local pg, not only CI)
1. **Connection high-water:** before/after, `SELECT count(*) FROM pg_stat_activity
   WHERE usename = current_user;` — drops from ~30 to ≤ pool max (8) and **falls**
   when idle (proves slots are released).
2. **Sustained persistence:** run ≥30 min ticking, poll `GET /health` — status
   stays **Healthy**, `last_success` advances, mobility_tick increases, exactly one
   `mobility_snapshots` row updates.
3. **Canonical smoke:** `node scripts/smoke-mobility-persistence.mjs` (reads
   /health, asserts world_id=abutopia, ok=true, Healthy, one row) — green.
4. **Graceful-degradation:** force a transient write failure (e.g. `max_connections=1`
   + a slow concurrent query, or briefly block the DB) → map **stays rendered** with
   a non-blocking banner, auto-clears on recovery; a sustained outage (Stale) still
   hard-gates.
5. **Full gate (per run-full-ci-gate-before-push):** Rust fmt-check + clippy +
   `scripts/cargo-serial.sh test -p sim-server` (background+poll, clear orphans);
   set `ABUTOWN_TEST_DATABASE_URL` to the real Supabase URL so the opt-in store
   round-trip tests exercise the shared pool. Frontend typecheck + vitest (incl.
   new backendGate-degraded + liveness-tolerance tests) + build. Mandatory browser
   smoke (backendGate is on the frontend↔backend boundary): boot chromium, force
   Degraded, assert the canvas renders (no takeover). CI green (wait for ALL pass).

## Non-goals
No move off Supabase. No schema migration. No change to the economy/render code
(the on-map view #79 is unaffected and already correct). Multi-replica connection
sizing (`max/replicas`) is documented, not implemented.

## Open items for the operator (config, not code)
- Read the actual Supabase tier pooler ceiling from the dashboard to pick the
  largest safe `ABUTOWN_DB_MAX_CONNECTIONS` (8 is conservative-safe).
- Flip `.env` `DATABASE_URL` to the `:6543` transaction-pooler endpoint at deploy.
