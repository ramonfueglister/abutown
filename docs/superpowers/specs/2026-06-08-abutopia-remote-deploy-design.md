# Abutopia Remote Deploy — Design

**Date:** 2026-06-08
**Status:** approved (brainstorming), pending spec review
**Goal:** Make the one shared abutopia world **publicly viewable** over the internet — a single long-lived `sim-server` on a managed PaaS (Fly.io), serving many concurrent browser clients over `wss://`, persisting to the existing Supabase Postgres. After deploy, opening the public frontend URL in a clean browser renders the live abutopia (`routed > 0`, citizens shopping at 9002).

---

## 1. Grounding (why this shape — no delulu)

Decided from a 4-agent grounding pass (code + literature, 2026-06-08):

- **The DB-connection scaling problem does not exist for this deploy.** "Many browser clients" is **WebSocket fan-out to one sim-server**, not Postgres connections — browsers never talk to Postgres. One server ⇒ one bounded DB pool.
- **`:6543` (transaction pooler) is a dead end for sqlx and is NOT pursued.** `statement_cache_capacity(0)` only disables sqlx's *client-side* cache; the driver still issues protocol-level Parse/Bind for every typed query, and transaction mode swaps backends between Parse and Bind, so it fails (`prepared statement "sqlx_s_1" already exists`). Supavisor's transaction-mode prepared-statement support is SQL-level `PREPARE`/`EXECUTE`, not the protocol path sqlx uses; the tracking issues remain **open as of Jan 2026** (sqlx#3198, sqlx#3850, supavisor#239). Making `:6543` work would require rewriting every typed query to raw param-free SQL (loses compile-time checking, invites injection) — disproportionate and against the project's clean-simulation values.
- **`:5432` (session pooler) is correct and already de-risked.** The reason `:6543` was originally chosen — connection exhaustion from 6 independent never-closing 5-connection pools (≤30 clients) — is **already fixed on `origin/main`** by the single shared self-reclaiming pool (`db.rs`: `DEFAULT_MAX_CONNECTIONS=8`, `min_connections(0)`, `idle_timeout(30s)`, `max_lifetime(900s)`, `acquire_timeout(10s)`, `test_before_acquire(true)`). That fix is port-agnostic, so one server on `:5432` stays well under free/small-tier caps (~20–25 free) with margin.
- **The persistence-stale render gate does not threaten the deploy.** Production config (`app/mod.rs:60-61`): window **15s**, persist cadence **5s**, failure tolerance **2** (`persistence_liveness.rs:6`). A 1–2.8s write succeeds well inside the window; `Stale` requires 15+ continuous seconds of zero successes (`persistence_liveness.rs:119-125`). `origin/main` also renders on `degraded` (only `stale` hard-blocks — `src/backend/backendGate.ts:99`). Co-locating the server in an EU Fly region near Supabase `eu-west-1` cuts the write latency further (the 1–2.8s included trans-Atlantic distance), adding headroom.

**Consequence:** the deploy needs **no DB/pooler redesign and no staleness-window change** — only public hosting plumbing + one bind-address fix. `DELETE FROM mobility_snapshots`/`economy_snapshots` for abutopia is the standard one-time fresh-seed before first boot (regenerates bindings via the #86 fix).

**Cited grounding sources:** sqlx#3198, sqlx#3850 / PR#3863 (0.8.6), supavisor#239, pgbouncer FAQ (1.21.0); origin/main `db.rs`, `app/mod.rs:60-61`, `persistence_liveness.rs:6,119-125`, `src/backend/mobilityClient.ts:318`.

---

## 2. Architecture — one writer, many viewers

```
 many browsers ──https──▶  Vercel (static frontend build)
       │                         │  VITE_ABUTOWN_BACKEND_URL = https://<app>.fly.dev
       └────────────wss://───────┴────────────▶  Fly.io Machine (1, fixed)
                                                     sim-server :8080
                                                     ├─ axum WS + /health
                                                     └─ sqlx pool (≤8) ──▶ Supabase :5432 (session pooler, verify-full)
```

- **Exactly one** sim-server instance (`min_machines_running = 1`, no autoscale). The in-memory world is the single source of truth; a second instance would double-write the same `world_id`. This is the hard constraint behind "many clients → one server".
- Fly terminates TLS at the edge; the app speaks plain HTTP/WS internally on `:8080`. Browsers use `wss://` (the client upgrades the scheme from the `https://` backend URL — `mobilityClient.ts:318`, already correct).
- Frontend is a static vite build on Vercel, built with `VITE_ABUTOWN_BACKEND_URL=https://<app>.fly.dev`.
- Persistence: existing shared 8-connection pool → Supabase `:5432`, `sslmode=verify-full` against the bundled public CA.

---

## 3. Components

### 3.1 Code change — bindable listen host (`backend/crates/sim-server/src/main.rs`)
Today `main.rs:18` hardcodes `127.0.0.1:{port}`; only `LISTEN_PORT` is env-driven. A container must bind `0.0.0.0`.

- Add `LISTEN_HOST` env, **default `127.0.0.1`** (dev stays loopback-only — no accidental exposure). The container sets `LISTEN_HOST=0.0.0.0`.
- Parse `format!("{host}:{port}")` into `SocketAddr`; keep the existing `LISTEN_PORT` handling and the `tracing::info!(%addr, …)` line.
- TDD: a unit test for the host/port→addr resolution helper (extract a small `fn resolve_listen_addr(host, port) -> Result<SocketAddr>` so it's testable without binding a socket): default host `127.0.0.1`, override `0.0.0.0`, invalid host → error.

This is the only production-code change. Everything else is deploy artifacts.

### 3.2 `Dockerfile` (repo root)
Multi-stage:
- **Builder:** `rust:1-bookworm` (match the workspace toolchain), `cargo build --release --manifest-path backend/Cargo.toml -p sim-server`.
- **Runtime:** `debian:bookworm-slim` + `ca-certificates`. `COPY` the release binary, the world bundle `data/worlds/abutopia/`, and the Supabase CA cert. `WORKDIR /app` so `ABUTOWN_BASE_WORLD_PATH` default (`data/worlds/abutopia`, relative to CWD) resolves; alternatively set `ABUTOWN_BASE_WORLD_PATH=/app/data/worlds/abutopia` explicitly.
- `ENV LISTEN_HOST=0.0.0.0 LISTEN_PORT=8080`. `EXPOSE 8080`. `CMD ["/app/sim-server"]`.

### 3.3 `.dockerignore` (repo root)
Exclude `backend/target`, `node_modules`, `.git`, `.worktrees`, `dist`, `.playwright-mcp`, `docs` — keep the build context small (the 8 MB `public/simutrans-assets/` is frontend-only, not needed by the backend image).

### 3.4 Supabase CA cert — `deploy/supabase-prod-ca.crt`
The Supabase pooler CA (`prod-ca-2021.crt`) is a **public** root certificate (currently only in the gitignored `.certs/`). Commit a copy to `deploy/supabase-prod-ca.crt` (outside `.certs/`) so the image build and CI have it. `COPY deploy/supabase-prod-ca.crt /app/certs/supabase-ca.crt`; set `PGSSLROOTCERT=/app/certs/supabase-ca.crt`. (It is a CA cert, not a credential — safe to commit.)

### 3.5 `fly.toml` (repo root)
- `app = "abutown-abutopia"` (or user-chosen), `primary_region = "lhr"` (EU, near Supabase `eu-west-1`).
- `[build] dockerfile = "Dockerfile"`.
- `[http_service]`: `internal_port = 8080`, `force_https = true`, `auto_stop_machines = false`, `auto_start_machines = false`, `min_machines_running = 1`. **No autoscaling.**
- `[[http_service.checks]]` (or `[checks]`): HTTP `GET /health`, healthy on 200.
- `[[vm]]`: shared-cpu, 512MB–1GB (sim + pool; adjust after observing).

### 3.6 Secrets / runtime config (Fly secrets, not in image)
- `DATABASE_URL` = the existing Supabase `:5432` session-pooler URL, `?sslmode=verify-full`.
- `PGSSLROOTCERT` = `/app/certs/supabase-ca.crt` (path in image; safe as plain env).
- `CORS_ALLOWED_ORIGINS` = the Vercel production origin (e.g. `https://abutown.vercel.app`) — set after the frontend URL is known.
- `ABUTOWN_DB_MAX_CONNECTIONS` = `8` (confirm against the actual tier ceiling from the Supabase dashboard).
- `RUST_LOG` = `warn,sim_server=info,economy::liveness=info`.

### 3.7 Frontend (Vercel)
- Build command `npm run build` (vite wrapper), output `dist/`.
- Build-time env `VITE_ABUTOWN_BACKEND_URL = https://<app>.fly.dev`.
- The chunk/WS client derives `wss://` automatically; no code change.
- After the Vercel URL exists, set the backend `CORS_ALLOWED_ORIGINS` to it and redeploy the Fly secret.

---

## 4. Deploy flow (runbook — controller + user)

Interactive logins are run by the user via `! fly auth login` / `! vercel login` (the `!` prefix runs in-session). Then:

1. **Fresh-seed** abutopia on the remote DB (authorized): `DELETE FROM mobility_snapshots WHERE world_id='abutopia';` `DELETE FROM economy_snapshots WHERE world_id='abutopia';` (clears any old `home_market=0` records; the #86 fix rebinds on next boot).
2. **Backend:** `fly launch --no-deploy` (generate/confirm app), `fly secrets set DATABASE_URL=… CORS_ALLOWED_ORIGINS=… ABUTOWN_DB_MAX_CONNECTIONS=8 RUST_LOG=…`, `fly deploy`. Poll `https://<app>.fly.dev/health` → `ok=true`, persistence `healthy`/`degraded` (not `stale`), `world_id=abutopia`.
3. **Frontend:** `vercel --prod` with `VITE_ABUTOWN_BACKEND_URL=https://<app>.fly.dev`. Note the prod URL.
4. **Wire CORS:** `fly secrets set CORS_ALLOWED_ORIGINS=https://<vercel-url>` (triggers a backend restart).
5. **Verify** (acceptance below).

---

## 5. Error handling & constraints

- **Single instance is load-bearing.** Document in `fly.toml` comments + runbook: never scale `count > 1`; the sim is a single-writer in-memory world.
- **Cold start / restart:** on boot the server hydrates from the snapshot (or fresh-seeds an empty world) — the #86 rebind runs every hydrate, so bindings survive restarts. Fly restart = sim resumes from the last persisted tick (frozen-time model).
- **DB outage:** the liveness gate degrades (banner) and hard-blocks only on sustained `stale`; transient slow/failed writes are tolerated (§1).
- **CORS misconfig** is the most likely first-deploy failure (frontend `/health` fetch blocked) — the runbook sets `CORS_ALLOWED_ORIGINS` explicitly to the Vercel origin.

---

## 6. Testing & acceptance

**In-repo (CI gate, must pass before merge):**
- `resolve_listen_addr` unit tests (default/override/invalid).
- Full Rust gate (fmt, clippy `-D warnings`, sim-server + sim-core tests), frontend (typecheck/vitest/build), e2e render-smoke — unchanged behavior; the `LISTEN_HOST` change must not regress the dev stack (which omits `LISTEN_HOST` → stays `127.0.0.1`).
- `docker build` succeeds locally (image builds, binary + world bundle + cert present).

**Live (post-deploy, controller + user):**
- `GET https://<app>.fly.dev/health` → `ok=true`, persistence not `stale`, `world_id=abutopia`.
- Open `https://<vercel-url>` in a clean browser → abutopia renders over `wss://`, no "persistence stale" overlay, no mixed-content/CORS console errors.
- Backend log shows `economy::liveness … routed > 0`.
- **Two simultaneous browsers** both see the live, ticking world (confirms WS fan-out).

---

## 7. Out of scope (explicitly deferred)

- Horizontal scaling / multiple sim-servers / autoscale (single-writer constraint; would need a different persistence architecture).
- Any `:6543` transaction-pooler fix (grounded dead end).
- Staleness-window tuning (current 15s is grounded-sufficient; revisit only if live data shows `stale` under EU co-location).
- Auth/multi-tenant, custom domains, CDN tuning, observability/metrics beyond `/health` + logs.
- Fixing the misleading `:6543`-is-safe comment in `db.rs` + the SOTA spec — tracked separately as a docs follow-up (not blocking this deploy).

---

## 8. References

- Grounding workflow (2026-06-08): sqlx#3198, sqlx#3850, supavisor#239, pgbouncer FAQ 1.21.0.
- origin/main: `backend/crates/sim-server/src/{main.rs,config.rs,db.rs,app/mod.rs,persistence_liveness.rs}`, `src/backend/mobilityClient.ts:318`, `data/worlds/abutopia/`.
- Related: `docs/superpowers/specs/2026-06-05-persistence-supabase-sota-design.md` (shared pool), `2026-06-07-abutopia-live-visible-design.md` (the local-PG predecessor).
