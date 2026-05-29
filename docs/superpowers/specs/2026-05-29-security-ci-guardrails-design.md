# Security + CI Guardrails Design

Date: 2026-05-29

## Status

Approved for specification (scope and approaches chosen by the implementer at
the user's direction; user mandate: *state-of-the-art solution throughout*).
This is **Plan 1** of a multi-plan hardening effort that came out of a
senior-level review of the codebase. The remaining buckets — splitting the
`mobility/systems.rs` (3545 lines) / `runtime.rs` / `app.rs` god files,
parameterising the frontend `CHUNK_SIZE`, and adding a permanent WS-payload
browser smoke — are explicitly **out of scope here** and will each get their
own spec → plan → implementation cycle.

This plan is sequenced first on purpose: it establishes the CI safety net
(clippy, fmt, type-checking, full-workspace tests) that the riskier refactors
in later plans will rely on to catch regressions.

## Goal

Two outcomes:

1. **Close the two real security holes** found in the review:
   - an authentication backdoor (`TEST_MODE_ACCEPT_ALL_JWTS`) that, if its env
     var is set, makes the production Supabase verifier accept *any* string
     that parses as a UUID as a valid login;
   - fully-open CORS (`CorsLayer::permissive()`).
2. **Widen the CI gate** so the class of rot it currently misses can no longer
   reach `main`: Rust lint/format, full-workspace + all-targets compilation,
   and TypeScript type-checking that includes the test suite.

The review proved the gate is too narrow with a concrete, *currently-failing*
example (see Background).

## Background: why the CI gate matters (verified, not hypothetical)

The current CI Rust job runs exactly one command:

```
cargo test --manifest-path backend/Cargo.toml -p sim-server
```

That compiles the `sim-server` test target only. As a result, the 2026-05-29
tram-retirement merge left **broken code on `main` that CI is structurally
unable to see**:

- `backend/crates/sim-core/examples/profile_lod_tick.rs` references
  `boarding_alighting_system`, a symbol deleted in the merge →
  `error[E0425]: cannot find value 'boarding_alighting_system'`. The example is
  never compiled by `-p sim-server`, so CI stays green.
- `backend/crates/sim-server/src/runtime.rs:106` `expected_base_world_car_count`
  is now only called from `#[cfg(test)]` code (lines 1820/1902/1945) → it is
  dead code in a non-test build → `clippy -D warnings` fails.
- `cargo fmt --check` is currently **dirty** (e.g. `protocol/src/lib.rs:496`,
  `mobility/systems.rs:597`).
- 45 test files under `tests/` are **never type-checked** (`tsconfig.json` has
  `"include": ["src"]`). Extending `tsc` to them surfaces 19 errors today: ~12
  are missing Node globals (config), and ~7 are genuine type defects in
  `tests/backend/mobilityClient.test.ts` (a loosely-typed `AgentMobility.state`
  mock — `case: string` instead of the literal union) and
  `tests/e2e/render-smoke.spec.ts` (`clickableVehicle` possibly `undefined`).
  This is exactly the "tests pass with stale/loose types while production
  breaks" failure mode CLAUDE.md documents from Phase 7a.

So the CI work has **prerequisite cleanup**: the gate cannot be tightened until
the code it newly compiles/checks is green. That cleanup is part of this plan.

## Chosen Approaches

### A1. Authentication backdoor — *remove it entirely*

**Decision: delete `TEST_MODE_ACCEPT_ALL_JWTS`, do not gate it.**

The codebase already has a clean auth abstraction:

```rust
pub enum AuthVerifier {
    LocalBearerUuid,            // accepts a raw UUID bearer token
    Supabase(Arc<JwksCache>),   // real RS256 JWT validation
}
```

- The Rust integration tests authenticate via `build_app`/`build_app_with_runtime`,
  which select `AuthVerifier::LocalBearerUuid` and pass `Bearer <uuid>`. They do
  **not** touch the backdoor.
- The only e2e test, `tests/e2e/render-smoke.spec.ts`, contains **zero** auth /
  session / token references. Without `VITE_SUPABASE_*` the frontend renders
  "Login unavailable" and never sends a bearer token, so the card-hand auth
  endpoint is never hit. The `TEST_MODE_ACCEPT_ALL_JWTS: "1"` line in
  `.github/workflows/ci.yml` is therefore **vestigial**.

A repo-wide search confirms `TEST_MODE_ACCEPT_ALL_JWTS` exists in exactly two
places: `card_hand.rs:220` and `ci.yml:99`. Removing both deletes the backdoor
from every build with **no loss of test coverage**.

This is the state-of-the-art outcome: the production binary ships **no auth
bypass of any kind**, and we did not add a feature flag or stub to preserve one.
(Considered and rejected as unnecessary: cfg-gating the env bypass behind a
Cargo feature; a config-selected `LocalBearerUuid` mode; an ephemeral local
JWKS stub minting real RS256 tokens. None are needed because nothing exercises
the bypass.)

**Follow-up noted, not done here:** there is currently no automated test of the
real `Supabase`/`JwksCache` validation path (signature, `iss`, expiry). Adding
one (a local JWKS stub + minted RS256 token) is a genuine coverage gap and is
recorded as a candidate for a later plan — but removing the backdoor does not
reduce existing coverage, so it does not block this one.

### A2. CORS — typed, fail-closed allow-list

Replace `CorsLayer::permissive()` (`app.rs:453`) with an explicit allow-list:

- Add `cors_allowed_origins: Vec<String>` to `ServerConfig`, parsed from a
  comma-separated `CORS_ALLOWED_ORIGINS` env var, following the existing
  `ServerConfig::from_pairs` pattern. Malformed origins fail fast at startup
  (parsed into `HeaderValue`/`axum::http::HeaderValue` once, not per request).
- **Fail-closed:** if the var is unset/empty, no cross-origin requests are
  allowed (same-origin only). No implicit localhost default in the binary.
- Allow only the methods/headers the API actually uses; never combine
  `allow_credentials(true)` with a wildcard origin.
- Local dev (`.env.example`) and the CI e2e job set
  `CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173`, so the browser smoke exercises
  the real locked-down path end-to-end (per CLAUDE.md's browser-smoke mandate).

### B. CI guardrails (with prerequisite cleanup)

Order matters: clean first, then tighten the gate, so each tightening lands green.

1. **`cargo fmt`** — run `cargo fmt --all` once to normalise; commit mechanically.
2. **clippy cleanup** — fix the broken `profile_lod_tick` example (drop/repair
   the dangling `boarding_alighting_system` reference), gate
   `expected_base_world_car_count` with `#[cfg(test)]` (or move it into the test
   module), then iterate until
   `cargo clippy --workspace --all-targets -- -D warnings` is clean. (clippy
   stops at the first error per crate, so expect to surface more once these two
   are fixed.)
3. **TypeScript test type-checking** — add a `tsconfig` for the test/script
   surface (Node + Vitest types) so `tests/` and `scripts/` are checked with the
   same `strict` options as `src`; fix the ~7 genuine type errors. Wire it so a
   single `tsc --noEmit` (or `tsc --build`) covers `src` + `tests` + `scripts`.
4. **Tighten `.github/workflows/ci.yml`**:
   - add `cargo fmt --manifest-path backend/Cargo.toml --all -- --check`
   - add `cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
   - change the test step to `cargo test --manifest-path backend/Cargo.toml --workspace`
     (was `-p sim-server`)
   - add a `tsc --noEmit` step covering tests
   - remove `--passWithNoTests` from `package.json`'s `test` script and from the
     `noRetiredAssets` CI step
   - remove the vestigial `TEST_MODE_ACCEPT_ALL_JWTS: "1"` env line; add
     `CORS_ALLOWED_ORIGINS` to the e2e job env

## Method

- **TDD where there is behaviour to assert** (RED → GREEN): auth rejects a
  non-Supabase token once the backdoor is gone; `ServerConfig` parses /
  rejects `CORS_ALLOWED_ORIGINS`; CORS responses carry the configured origin
  and deny others. Cleanup steps (fmt/clippy/tsc) are verification-gated rather
  than test-first.
- **Browser smoke** is mandatory for the CORS change because it crosses the
  frontend↔backend boundary (CLAUDE.md). The e2e job is the smoke.
- Every step ends with the relevant `cargo`/`npm`/`tsc` command actually run and
  its output confirmed before the step is called done.

## Success Criteria

- `TEST_MODE_ACCEPT_ALL_JWTS` appears nowhere in the repo; a Supabase-mode token
  that is not a valid JWT is rejected.
- `CorsLayer::permissive()` is gone; requests from a non-allow-listed origin are
  refused; the configured origin succeeds; the e2e smoke passes against the
  locked-down server.
- Locally green on a clean checkout: `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `tsc --noEmit` (incl. tests), `npm test` (no
  `--passWithNoTests`), `npm run test:e2e`.
- CI runs all of the above; the `profile_lod_tick` example compiles.

## Out of Scope (future plans)

- Splitting `mobility/systems.rs` / `runtime.rs` / `app.rs`.
- Replacing `std::sync::RwLock` in `JwksCache` with `ArcSwap` (poisoning risk).
- Frontend `CHUNK_SIZE` parameterisation + permanent WS-payload browser smoke.
- Converting startup/hydration `panic!`/`expect` to propagated `Result`s.
- A real Supabase JWKS validation test.
- Moving the 26 MB of committed PDFs/binaries out of git.

## Coordination: parallel agent active

Another agent is working concurrently on branch
`codex/remove-rail-tram-visuals` and is committing to `main` (e.g. `7752564`).
To avoid stepping on it:

- This work proceeds on its own branch (`plan/security-ci-guardrails`), created
  from the current `main` HEAD (which already includes the parallel agent's
  latest cleanup), executed in an **isolated git worktree**.
- Scope is near-disjoint: this plan touches auth/config/CI
  (`card_hand.rs`, `app.rs` CORS layer, `config.rs`, `ci.yml`, `package.json`,
  `tsconfig*`, `playwright.config.ts`), while the parallel agent is in the
  renderer/mobility/tram surface.
- **Known overlap risk:** the clippy prerequisites (`profile_lod_tick` example,
  `expected_base_world_car_count`) are fallout from the *parallel agent's own*
  tram-retirement work. They may fix these independently. Before implementing
  step B2, re-check whether `main` has moved and rebase; if they've already
  fixed an item, drop it from this plan rather than conflicting.
