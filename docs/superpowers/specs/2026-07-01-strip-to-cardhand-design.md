# Strip Abutown to Card-Hand + Supabase Login

**Date:** 2026-07-01
**Status:** Design approved, pending spec review

## Goal

Remove the entire economy/mobility simulation from the app. Keep only the
**card hand** UI with **Supabase login**, backed by a minimal HTTP server.
Preserve reusable *scaffolding* so a **new** simulation can be grown later,
and rely on git history to resurrect the old engine's reusable pieces on
demand.

## Non-goals

- Building any new simulation now (that is a separate future project).
- Changing card-hand behavior, styling, or the Supabase auth flow.
- Changing hosting: stays Vercel (frontend `dist`) + Fly (backend) + Supabase.

## Decisions (from brainstorming)

1. **Backend:** slim the existing Rust axum server down — do *not* replace it
   with pure Supabase/PostgREST.
2. **Cleanup:** delete everything that is *sim-only*, except explicitly-kept
   scaffolding (below).
3. **Deploy:** keep Vercel + Fly + Supabase; adjust `Dockerfile`/`fly.toml`.
4. **Scaffolding kept** (a new sim starts faster; not "dead sim gameplay"):
   `cargo-serial.sh`, one browser-smoke template, a trivial `/ws` route stub,
   and the proto/buf toolchain reduced to a placeholder schema.

Git history retains the full old simulation; "delete" means "remove from the
active tree", not "lose".

## Target end state

- A static Vite page that renders **only** the card-hand shell + login.
- A tiny axum server exposing `/health`, `/cards`, `/card-hand` (GET/PUT), and
  a no-op `/ws` stub — authenticated via Supabase JWT (JWKS), persisting hands
  in Postgres `user_card_hands`.
- A proto/buf toolchain that provably runs end-to-end against a placeholder
  message (ready for new sim messages).

## Architecture

### Frontend (`src/`)

**Keep & trim**
- `index.html`: remove `<canvas id="game">`; keep the `<script type="module">`.
- `main.ts`: reduce to a minimal bootstrap — import `style.css` and call
  `mountCardHandView()` directly (no `appRuntime`).
- `cardHand/cardHandView.ts`, `cardHand/cardHandState.ts`: unchanged.
- `backend/backendGate.ts`: keep only `resolveBackendBaseUrl` (the sole symbol
  the card hand imports). Remove sim health/`requireBackend` logic. If cleaner,
  inline `resolveBackendBaseUrl` into the card-hand module and delete the file.
- `style.css`: keep `.card-*` rules; remove map/game/HUD styles.

**Delete (sim-only)**
- `render/`, `city/`, `cameraController.ts`, `projection.ts`, `types.ts`.
- All of `app/` (`appRuntime.ts`, `vitalsHud.ts`, `persistenceBanner.ts`,
  `interaction.ts`, `entitySelection.ts`, `backendRequiredView.ts`,
  `runtimeDiagnostics.ts`).
- `backend/` except `backendGate.ts`: `economyState.ts`, `mobilityState.ts`,
  `mobilityClient.ts`, `mobilityProtocol.ts`, `chunkSubscriptionClient.ts`,
  `baseWorldClient.ts`, `simTime.ts`, `proto/`, plus their `.test.ts` files.

### Backend (`backend/crates/`)

**Keep**
- Crate `sim-server` with: `main.rs` (unchanged), `config.rs`, `db.rs`,
  `card_hand.rs`, migration `202605150002_card_hand_core.sql`.
- Crate `protocol` — reduced to a placeholder `.proto` (all sim messages
  removed; one minimal example message added) so buf + prost-build still run.

**Rewrite**
- `app/mod.rs`: replace with a small router —
  `/health`, `/cards` (GET), `/card-hand` (GET + PUT), `/ws` (trivial upgrade
  stub) — wiring `CorsLayer`, `CardHandStore`, `AuthVerifier` via
  `build_app_from_config`. Remove every sim import (runtime, snapshots,
  base_world, proto_convert, commands) and the sim tests; keep/adapt the
  card-hand HTTP tests.
- `lib.rs`: reduce `pub mod` list to `app`, `card_hand`, `config`, `db`.

**Delete (sim-only)**
- Crate `sim-core` (entire engine).
- In `sim-server/src`: `runtime/`, `runtime_view.rs`, `persistence_liveness.rs`,
  `persistence_plugin.rs`, `postgres_economy.rs`, `postgres_economy_events.rs`,
  `postgres_events.rs`, `postgres_mobility.rs`, `postgres_snapshots.rs`,
  `commands.rs`, `bin/e2e_server.rs`, `app/base_world_response.rs`,
  `app/proto_convert.rs`.
- Non-card migrations (if any) under `sim-server/migrations/`.

**Cargo manifests**
- `backend/Cargo.toml` workspace `members`: `["crates/protocol", "crates/sim-server"]`.
  Prune now-unused workspace deps (e.g. `bevy_ecs`, `criterion`).
- `sim-server/Cargo.toml`: drop `sim-core` dep; drop `dashmap`, `tokio-stream`.
  Keep `abutown-protocol`, `axum` (with `ws`), `jsonwebtoken`, `reqwest`,
  `sqlx`, `uuid`, `arc-swap`, `serde*`, `tokio`, `tower-http`, `tracing*`,
  `dotenvy`, `thiserror`, `anyhow`, `prost`. Keep `tokio-tungstenite` dev-dep
  (for the `/ws` smoke), drop unused dev-deps.

### Auth & data (unchanged)

- `AuthVerifier::Supabase` fetches JWKS from
  `{SUPABASE_URL}/auth/v1/.well-known/jwks.json` and validates the bearer JWT.
- `CardHandStore` persists to Postgres `user_card_hands`; `/cards` returns the
  static `card_definitions()`.

## Toolchain & repo cleanup

**Kept scaffolding**
- `scripts/cargo-serial.sh`.
- `scripts/smoke-7a.mjs` — retained as the generic browser-smoke template.
- Proto/buf: `buf.yaml`, `buf.gen.yaml`, `scripts/generate-proto-ts.mjs`,
  `generate:proto` npm script, `@bufbuild/*` deps — all kept, now targeting the
  placeholder schema.

**`package.json`**
- Remove sim scripts: `dev:stack`, `preview:stack`, `smoke:*`,
  `generate:abutopia`, `lint:proto*` (keep `lint:proto` only if `buf.yaml`
  stays valid against the placeholder — otherwise drop).
- `build`: `npm run generate:proto && vite build` (drop the `scripts/build.mjs`
  wrapper, since the simutrans-assets copy problem it worked around is gone).
- `test:e2e`: removed (Playwright suite was sim-only). Keep `test` (vitest),
  `typecheck`, `dev`, `preview`, `build`, `generate:proto`.

**Delete**
- `scripts/*` except `cargo-serial.sh`, `smoke-7a.mjs`, `generate-proto-ts.mjs`,
  and `build.mjs` (build.mjs deleted).
- Sim test dirs under `tests/` (`app/`, `city/`, `render/`, `backend/` sim
  tests, `e2e/`, `scripts/`); keep only card-hand tests.
- `data/worlds/`, `public/simutrans-assets/` (if present), sim design specs
  under `docs/superpowers/specs/` (keep this spec), `progress.md`.

**Config rewrites**
- `Dockerfile`: drop `protobuf-compiler` install only if the placeholder proto
  no longer needs `protoc` at container-build time; keep whatever the reduced
  `protocol` crate's `prost-build` requires. Drop base-world/sim env and asset
  copies; build & run only `sim-server`.
- `fly.toml`: remove `ABUTOWN_BASE_WORLD_PATH` and other sim env; keep
  `DATABASE_URL`, `SUPABASE_URL`, `CORS_ALLOWED_ORIGINS`, listen host/port.
- `.env.example`: trim to `DATABASE_URL`, `PGSSLROOTCERT`, `SUPABASE_URL`,
  `VITE_SUPABASE_URL`, `VITE_SUPABASE_PUBLISHABLE_KEY`,
  `VITE_ABUTOWN_BACKEND_URL`, `CORS_ALLOWED_ORIGINS`.
- `CLAUDE.md`: rewrite conventions to the card-hand stack — keep the
  browser-smoke rule (still crosses the wire) and the cargo-serial rule; drop
  economy/progress.md/simutrans notes.
- Keep `deploy/supabase-prod-ca.crt` + `deploy/README.md` (Postgres TLS still
  needed), `supabase/config.toml`, `vite.config.ts`, `tsconfig*.json`,
  `vitest.config.ts`. Remove `playwright.config.ts` (e2e gone).

## Error handling

Unchanged from current card-hand code: missing/invalid bearer → 401 via
`CardHandError`; unknown `card_id` on PUT → 4xx; missing Supabase/DB env →
server fails fast at startup (`ServerConfig::from_env`). Frontend: missing
`VITE_SUPABASE_*` → card hand renders login-unavailable state (existing
behavior, unchanged).

## Testing / verification

1. **Rust:** `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`
   — card-hand HTTP tests green; `cargo build` for the workspace succeeds.
2. **Proto toolchain:** `npm run generate:proto` succeeds against the
   placeholder schema; generated TS compiles under `tsc`.
3. **Frontend:** `npm run test` (vitest card-hand), `npm run typecheck`,
   `npm run build` all pass.
4. **Browser smoke (mandatory — crosses the wire):** load the built page; assert
   the card-hand shell + Login button render, and that no requests to removed
   sim routes (`/world`, `/mobility`, `/economy`, `/ws` for data) occur. With
   real Supabase creds, a logged-in session loads `/card-hand`.

## Rollout

Single feature branch → PR. This is a large deletion; the PR is expected to be
red-line heavy. Deploy after merge: Fly redeploys the slim `sim-server`; Vercel
rebuilds the static `dist`. No Postgres migration beyond the existing
`user_card_hands` table (already live). No `DELETE FROM` needed — the removed
sim snapshot tables are simply no longer written.

## Open questions

None blocking. `lint:proto` retention depends on whether `buf.yaml` stays valid
against the placeholder; decide during implementation.
