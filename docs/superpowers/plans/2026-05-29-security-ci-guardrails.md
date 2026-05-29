# Security + CI Guardrails Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the JWT auth backdoor and permissive CORS, then widen the CI gate (clippy, fmt, full-workspace tests, TypeScript type-checking incl. tests) after cleaning the rot that the current narrow gate hides.

**Architecture:** Two workstreams. Security (Tasks 1–3) deletes the `TEST_MODE_ACCEPT_ALL_JWTS` bypass outright and replaces `CorsLayer::permissive()` with a typed, fail-closed allow-list threaded from `ServerConfig`. CI hardening (Tasks 4–7) first makes `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`, and `tsc` (incl. `tests/`) green, then wires them into `.github/workflows/ci.yml`.

**Tech Stack:** Rust (axum 0.8, tower-http CORS, bevy_ecs, sqlx), TypeScript (Vite 8, Vitest 4, Playwright), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-05-29-security-ci-guardrails-design.md`

**Branch / isolation:** Work on `plan/security-ci-guardrails` in an isolated git worktree (see `superpowers:using-git-worktrees`). The branch is based on a `main` HEAD that already includes the parallel agent's `7752564`.

**Parallel-agent note:** Another agent works on `codex/remove-rail-tram-visuals`. Tasks 4–5 touch its tram-retirement fallout (`profile_lod_tick.rs`, `expected_base_world_car_count`). **Before Task 5, run `git fetch && git log --oneline origin/main -5` and check whether those are already fixed upstream; if so, drop the corresponding step and rebase.**

---

## Task 1: Remove the JWT auth backdoor

**Files:**
- Modify: `backend/crates/sim-server/src/card_hand.rs:216-226`
- Test: `backend/crates/sim-server/tests/http.rs` (append a regression guard)

Context — current `JwksCache::validate` (the real Supabase verification path):

```rust
    async fn validate(&self, token: &str) -> Result<Uuid, CardHandError> {
        if let Ok(user_id) = self.try_validate(token) {
            return Ok(user_id);
        }
        if std::env::var("TEST_MODE_ACCEPT_ALL_JWTS").ok().as_deref() == Some("1") {
            return Uuid::parse_str(token).map_err(|_| CardHandError::InvalidAuth);
        }

        self.refresh().await?;
        self.try_validate(token)
    }
```

- [ ] **Step 1: Write the failing regression-guard test**

Append to `backend/crates/sim-server/tests/http.rs`:

```rust
#[test]
fn auth_backdoor_env_var_is_not_referenced_in_source() {
    // Regression guard: the Supabase verifier must never contain a runtime
    // env-var bypass that accepts arbitrary tokens. See
    // docs/superpowers/specs/2026-05-29-security-ci-guardrails-design.md
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/card_hand.rs"
    ))
    .expect("read card_hand.rs source");
    assert!(
        !src.contains("TEST_MODE_ACCEPT_ALL_JWTS"),
        "auth backdoor env var must not be referenced in card_hand.rs"
    );
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server auth_backdoor_env_var_is_not_referenced_in_source`
Expected: FAIL — the assertion trips because the string is still present.

- [ ] **Step 3: Delete the backdoor block**

In `backend/crates/sim-server/src/card_hand.rs`, change `validate` to:

```rust
    async fn validate(&self, token: &str) -> Result<Uuid, CardHandError> {
        if let Ok(user_id) = self.try_validate(token) {
            return Ok(user_id);
        }
        self.refresh().await?;
        self.try_validate(token)
    }
```

- [ ] **Step 4: Run the test and verify it passes**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS — the new guard passes and the existing 50 lib + 15 HTTP + 10 WebSocket tests still pass (none referenced the env var).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/card_hand.rs backend/crates/sim-server/tests/http.rs
git commit -m "fix(security): remove TEST_MODE_ACCEPT_ALL_JWTS auth backdoor

The Supabase JWT verifier accepted any UUID-parseable string as a valid
login when the env var was set. Nothing exercises the bypass (Rust tests
use AuthVerifier::LocalBearerUuid; the e2e render smoke never auths), so
it is deleted outright rather than gated. Adds a source-scan regression
guard.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Add `cors_allowed_origins` to `ServerConfig`

**Files:**
- Modify: `backend/crates/sim-server/src/config.rs`
- Modify: `backend/crates/sim-server/tests/http.rs:649` (struct literal gains the new field)
- Test: `backend/crates/sim-server/src/config.rs` (existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `config.rs`:

```rust
    #[test]
    fn config_parses_comma_separated_cors_origins() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
            (
                "CORS_ALLOWED_ORIGINS",
                "http://127.0.0.1:5173,https://app.example.com",
            ),
        ])
        .unwrap();

        assert_eq!(
            config.cors_allowed_origins,
            vec![
                "http://127.0.0.1:5173".to_string(),
                "https://app.example.com".to_string(),
            ]
        );
    }

    #[test]
    fn config_defaults_to_no_cors_origins_when_unset() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
        ])
        .unwrap();

        assert!(config.cors_allowed_origins.is_empty());
    }

    #[test]
    fn config_ignores_blank_cors_entries() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
            ("CORS_ALLOWED_ORIGINS", " http://a , ,http://b "),
        ])
        .unwrap();

        assert_eq!(
            config.cors_allowed_origins,
            vec!["http://a".to_string(), "http://b".to_string()]
        );
    }
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server config::tests`
Expected: FAIL — `cors_allowed_origins` field does not exist (compile error).

- [ ] **Step 3: Add the field and parsing**

In `config.rs`, add the field to the struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub database_url: String,
    pub supabase_url: String,
    pub cors_allowed_origins: Vec<String>,
}
```

In `from_pairs`, add a local accumulator and a match arm, and populate the result:

```rust
        let mut database_url = None;
        let mut supabase_url = None;
        let mut cors_allowed_origins = Vec::new();

        for (key, value) in pairs {
            match key.as_ref() {
                "DATABASE_URL" => database_url = Some(value.into()),
                "SUPABASE_URL" => supabase_url = Some(value.into()),
                "CORS_ALLOWED_ORIGINS" => {
                    cors_allowed_origins = value
                        .into()
                        .split(',')
                        .map(str::trim)
                        .filter(|origin| !origin.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                _ => {}
            }
        }

        Ok(Self {
            database_url: database_url.ok_or(ServerConfigError::MissingDatabaseUrl)?,
            supabase_url: supabase_url.ok_or(ServerConfigError::MissingSupabaseUrl)?,
            cors_allowed_origins,
        })
```

- [ ] **Step 4: Fix the struct-literal construction in the HTTP test**

`backend/crates/sim-server/tests/http.rs:649` builds `ServerConfig` with a struct literal, which will no longer compile without the new field. Update it:

```rust
    let config = ServerConfig {
        database_url,
        supabase_url: "http://dummy.local".to_string(),
        cors_allowed_origins: Vec::new(),
    };
```

(This is the only struct-literal construction of `ServerConfig` in the crate — all other call sites use `ServerConfig::from_pairs`/`from_env`.)

- [ ] **Step 5: Run the tests and verify they pass**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server config::tests`
Expected: PASS (all config tests, including the 3 pre-existing ones). The integration test crate also compiles now that the literal has the new field.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/config.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat(config): parse CORS_ALLOWED_ORIGINS into ServerConfig

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Replace permissive CORS with a fail-closed allow-list

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs` (`build_app_from_config`, `build_router_from_state`, the two infallible builders, imports)
- Modify: `.env.example`
- Test: `backend/crates/sim-server/src/app.rs` (add `#[cfg(test)] mod cors_tests`)

Context — current router tail in `build_router_from_state`:

```rust
        .with_state(state)
        .layer(CorsLayer::permissive())
```

`build_router_from_state(state)` has no access to config; three callers reach it: `build_app_from_config` (has `config`), `build_app_with_runtime_and_card_hands`, and `build_app` (via the former). We thread a prebuilt `CorsLayer` in.

- [ ] **Step 1: Write the failing CORS behavior test**

Add to `app.rs` (bottom of file):

```rust
#[cfg(test)]
mod cors_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn router_with_origins(origins: &[&str]) -> axum::Router {
        let owned: Vec<String> = origins.iter().map(|o| o.to_string()).collect();
        let cors = cors_layer(&owned).expect("valid origins");
        axum::Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .layer(cors)
    }

    #[tokio::test]
    async fn allowed_origin_is_reflected() {
        let app = router_with_origins(&["http://127.0.0.1:5173"]);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap().to_string()),
            Some("http://127.0.0.1:5173".to_string())
        );
    }

    #[tokio::test]
    async fn disallowed_origin_gets_no_cors_header() {
        let app = router_with_origins(&["http://127.0.0.1:5173"]);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "https://evil.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.headers().get("access-control-allow-origin").is_none());
    }

    #[tokio::test]
    async fn empty_allow_list_is_fail_closed() {
        let app = router_with_origins(&[]);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.headers().get("access-control-allow-origin").is_none());
    }
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server cors_tests`
Expected: FAIL — `cors_layer` does not exist (compile error).

- [ ] **Step 3: Add the `cors_layer` helper**

In `app.rs`, replace the import `use tower_http::cors::CorsLayer;` with:

```rust
use tower_http::cors::{AllowOrigin, CorsLayer};
```

Add this helper near `build_router_from_state`:

```rust
/// Build a fail-closed CORS layer from an explicit allow-list. An empty list
/// allows no cross-origin requests. Malformed origins are a startup error.
fn cors_layer(allowed_origins: &[String]) -> anyhow::Result<CorsLayer> {
    use axum::http::{HeaderValue, Method, header};

    let origins = allowed_origins
        .iter()
        .map(|origin| {
            origin
                .parse::<HeaderValue>()
                .map_err(|err| anyhow::anyhow!("invalid CORS origin {origin:?}: {err}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
}
```

- [ ] **Step 4: Thread the layer through the builders**

Change `build_router_from_state` to take a `CorsLayer` and use it instead of `permissive()`:

```rust
fn build_router_from_state(state: AppState, cors: CorsLayer) -> Router {
    state.spawn_snapshot_loop(SNAPSHOT_INTERVAL);

    Router::new()
        .route("/health", get(health))
        .route("/cards", get(cards))
        .route("/card-hand", get(card_hand).put(save_card_hand))
        .route("/world", get(world))
        .route("/base-world", get(base_world))
        .route("/chunks/{x}/{y}", get(chunk))
        .route("/commands", post(command))
        .route("/mobility", get(mobility))
        .route("/ws", get(websocket))
        .with_state(state)
        .layer(cors)
}
```

In `build_app_from_config`, build the CORS layer from config and pass it:

```rust
    let cors = cors_layer(&config.cors_allowed_origins)?;
    Ok(build_router_from_state(state, cors))
```

In `build_app_with_runtime_and_card_hands` (the infallible test/dev builder), pass a fail-closed layer:

```rust
    let state = AppState::new_with_card_hands(runtime, card_hands, auth);
    let cors = cors_layer(&[]).expect("empty origin list is always valid");
    build_router_from_state(state, cors)
```

- [ ] **Step 5: Run the full sim-server test suite**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS — the 3 new CORS tests pass; all existing HTTP/WebSocket tests still pass (they issue `oneshot` requests without an `Origin` header, so the allow-list does not affect them).

- [ ] **Step 6: Document the env var in `.env.example`**

Add under the backend section of `.env.example`:

```
# Comma-separated browser origins allowed to call the API (CORS). Unset =
# fail-closed (no cross-origin requests). Dev/e2e use http://127.0.0.1:5173.
CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173
```

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/app.rs .env.example
git commit -m "fix(security): replace permissive CORS with fail-closed allow-list

CorsLayer::permissive() allowed any origin. Origins now come from
CORS_ALLOWED_ORIGINS via ServerConfig; an empty list allows no
cross-origin requests. Malformed origins fail at startup.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Normalise Rust formatting

**Files:**
- Modify: whatever `cargo fmt` rewrites (e.g. `protocol/src/lib.rs`, `mobility/systems.rs`)

- [ ] **Step 1: Confirm fmt is currently dirty**

Run: `cargo fmt --manifest-path backend/Cargo.toml --all -- --check`
Expected: non-zero exit with a diff (this is the RED state).

- [ ] **Step 2: Apply formatting**

Run: `cargo fmt --manifest-path backend/Cargo.toml --all`

- [ ] **Step 3: Verify clean**

Run: `cargo fmt --manifest-path backend/Cargo.toml --all -- --check`
Expected: exit 0, no output.

- [ ] **Step 4: Verify nothing else broke**

Run: `cargo test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS (formatting-only change).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "style: cargo fmt --all

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Make `clippy --workspace --all-targets -D warnings` green

**Files:**
- Modify: `backend/crates/sim-core/examples/profile_lod_tick.rs:143`
- Modify: `backend/crates/sim-server/src/runtime.rs:106`

> **Parallel-agent check first:** run `git fetch origin && git log --oneline origin/main -8`. If `profile_lod_tick.rs` or `expected_base_world_car_count` were already fixed upstream, rebase onto `origin/main` and skip the corresponding step below.

Context — the broken example references a system deleted in the tram retirement:

```rust
    let mut s_board = {
        let mut s = Schedule::default();
        s.add_systems(boarding_alighting_system);   // <- deleted symbol (E0425)
        s
    };
```

And `runtime.rs:106` is only called from `#[cfg(test)]` code (lines 1820/1902/1945):

```rust
fn expected_base_world_car_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_car_routes(base_world).len()
}
```

- [ ] **Step 1: Confirm clippy is currently red**

Run: `cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
Expected: FAIL — `E0425 cannot find value 'boarding_alighting_system'` and `function 'expected_base_world_car_count' is never used`.

- [ ] **Step 2: Remove the dead schedule from the example**

In `profile_lod_tick.rs`, delete the `s_board` block (the four lines shown above) and remove any later use of `s_board` in the timing section. Search for remaining references:

Run: `rg -n "s_board|boarding_alighting" backend/crates/sim-core/examples/profile_lod_tick.rs`
Expected after edit: no matches.

- [ ] **Step 3: Gate the test-only function**

In `runtime.rs`, annotate the function so it is only compiled for tests:

```rust
#[cfg(test)]
fn expected_base_world_car_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_car_routes(base_world).len()
}
```

- [ ] **Step 4: Run clippy and fix anything it newly surfaces**

Run: `cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
Expected: clippy stops at the first error per crate, so additional warnings may now appear. Fix each with the minimal idiomatic change (no blanket `#[allow(...)]` unless the lint is genuinely a false positive — if so, scope the allow to the item and add a one-line comment why). Re-run until exit 0.

- [ ] **Step 5: Verify tests still pass**

Run: `cargo test --manifest-path backend/Cargo.toml --workspace`
Expected: PASS across protocol + sim-core + sim-server.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "fix: repair clippy/workspace targets after tram retirement

Drop the dead boarding_alighting_system schedule from the profile_lod_tick
example and gate the test-only expected_base_world_car_count behind
#[cfg(test)]. clippy --workspace --all-targets -D warnings is now clean.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Type-check the test suite and fix the genuine type errors

**Files:**
- Create: `tsconfig.typecheck.json`
- Modify: `package.json` (add `typecheck` script)
- Modify: `tests/backend/mobilityClient.test.ts` (`agentStateToProto` return type)
- Modify: `tests/e2e/render-smoke.spec.ts:185` (`clickableVehicle` null guard)

- [ ] **Step 1: Create the type-check project that includes tests**

Create `tsconfig.typecheck.json`:

```json
{
  "extends": "./tsconfig.json",
  "compilerOptions": {
    "noEmit": true,
    "types": ["node"]
  },
  "include": ["src", "tests", "scripts"]
}
```

- [ ] **Step 2: Run it to capture the RED state**

Run: `npx tsc -p tsconfig.typecheck.json`
Expected: FAIL with ~19 errors — ~12 `TS2591` "Cannot find name 'node:fs'/'process'" (resolved by the `"types": ["node"]` above once re-run), plus the genuine ones below. Re-run after Step 1 is saved; the Node-globals errors should be gone, leaving the real defects.

- [ ] **Step 3: Fix the loosely-typed `agentStateToProto`**

In `tests/backend/mobilityClient.test.ts`, the helper returns `{ state: { case: string; value: unknown } }`, which is not assignable to the protobuf `MessageInit<AgentMobility>` oneof. Remove the widening annotation and pin each discriminant with `as const`:

```ts
function agentStateToProto(state: MobilitySnapshotDto['agents'][number]['state']) {
  switch (state.type) {
    case 'walking':
      return { state: { case: 'walking' as const, value: { linkId: state.link_id, progress: state.progress } } };
    case 'waiting_at_stop':
      return { state: { case: 'waitingAtStop' as const, value: { stopId: state.stop_id } } };
    case 'in_vehicle':
      return { state: { case: 'inVehicle' as const, value: { vehicleId: state.vehicle_id, seatIndex: state.seat_index } } };
    case 'boarding':
      return { state: { case: 'boarding' as const, value: { vehicleId: state.vehicle_id, stopId: state.stop_id } } };
    case 'alighting':
      return { state: { case: 'alighting' as const, value: { vehicleId: state.vehicle_id, stopId: state.stop_id } } };
    case 'at_activity':
      return { state: { case: 'atActivity' as const, value: { activityId: state.activity_id } } };
    default: {
      const _exhaustive: never = state;
      throw new Error(`unhandled agent state ${JSON.stringify(_exhaustive)}`);
    }
  }
}
```

(The explicit return-type annotation lines — `): {\n  state: { case: string; value: unknown };\n}` — are removed; the `default` arm makes the switch provably exhaustive for tsc.)

- [ ] **Step 4: Fix the `clickableVehicle` null-safety error**

In `tests/e2e/render-smoke.spec.ts`, replace the truthiness expectation that does not narrow:

```ts
  const clickableVehicle = await visibleVehicle(page, { width: 409, height: 519 });
  expect(clickableVehicle).toBeTruthy();
```

with a guard that both asserts and narrows for the subsequent `.screen` / `.id` accesses:

```ts
  const clickableVehicle = await visibleVehicle(page, { width: 409, height: 519 });
  if (!clickableVehicle) throw new Error('expected a clickable vehicle in the viewport');
```

- [ ] **Step 5: Resolve any residual errors, then verify clean**

Run: `npx tsc -p tsconfig.typecheck.json`
Expected: exit 0. If any error remains (e.g. the `pollFn` callable error at the mock-interval site), apply the minimal type-correct fix — narrow the type or import the proper protobuf/Vitest type. **Do not** silence with `any` or a non-null `!` unless the value is provably non-null at that point.

- [ ] **Step 6: Add the npm script and confirm unit tests still pass**

Add to `package.json` `scripts`:

```json
    "typecheck": "tsc -p tsconfig.typecheck.json",
```

Run: `npm run typecheck && npm test`
Expected: typecheck exits 0; vitest passes.

- [ ] **Step 7: Commit**

```bash
git add tsconfig.typecheck.json package.json tests/backend/mobilityClient.test.ts tests/e2e/render-smoke.spec.ts
git commit -m "test: type-check tests/ and scripts/, fix genuine type defects

Adds tsconfig.typecheck.json covering src+tests+scripts with Node types,
and fixes the loosely-typed agentStateToProto mock and an unchecked
clickableVehicle access. Closes the test-typing gap noted in CLAUDE.md.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Tighten the CI gate

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `package.json` (drop `--passWithNoTests`)

Context — current Rust job runs only `cargo test ... -p sim-server`; the e2e job sets the now-deleted `TEST_MODE_ACCEPT_ALL_JWTS`.

- [ ] **Step 1: Drop `--passWithNoTests`**

In `package.json`, change:

```json
    "test": "vitest run --passWithNoTests",
```
to:
```json
    "test": "vitest run",
```

In `.github/workflows/ci.yml`, the retired-asset guard step (`npx vitest run tests/render/noRetiredAssets.test.ts --passWithNoTests`) — remove the `--passWithNoTests` flag.

- [ ] **Step 2: Add the TypeScript type-check step to the `frontend` job**

In `.github/workflows/ci.yml`, in the `frontend` job after "Generate protobuf TypeScript" and before "Run frontend tests":

```yaml
      - name: Type-check (src + tests + scripts)
        run: npm run typecheck
```

- [ ] **Step 3: Add fmt + clippy and widen tests in the `rust` job**

Replace the single "Run sim-server tests" step in the `rust` job with:

```yaml
      - name: Check Rust formatting
        run: cargo fmt --manifest-path backend/Cargo.toml --all -- --check

      - name: Run Clippy (workspace, all targets, deny warnings)
        run: cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings

      - name: Run workspace tests
        run: cargo test --manifest-path backend/Cargo.toml --workspace
```

- [ ] **Step 4: Update the e2e job env**

In `.github/workflows/ci.yml`, in the `e2e` job `env:` block, remove the line:

```yaml
      TEST_MODE_ACCEPT_ALL_JWTS: "1"
```

and add:

```yaml
      CORS_ALLOWED_ORIGINS: "http://127.0.0.1:5173"
```

- [ ] **Step 5: Verify the whole gate locally (clean tree)**

Run each and confirm exit 0:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path backend/Cargo.toml --workspace
npm run typecheck
npm test
npm run build && npx playwright test
```

Expected: all green. The Playwright smoke now runs against the fail-closed CORS server with `CORS_ALLOWED_ORIGINS=http://127.0.0.1:5173` (set this env var locally for the playwright run).

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml package.json
git commit -m "ci: gate on clippy, fmt, workspace tests, and tsc (incl tests)

Widens CI beyond 'cargo test -p sim-server': adds fmt --check, clippy
--workspace --all-targets -D warnings, cargo test --workspace, and tsc
over tests/. Drops --passWithNoTests. Removes the deleted
TEST_MODE_ACCEPT_ALL_JWTS env and sets CORS_ALLOWED_ORIGINS for the e2e
smoke.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification

- [ ] On a clean tree, the full local gate is green (Task 7 Step 5).
- [ ] `rg TEST_MODE_ACCEPT_ALL_JWTS` returns nothing.
- [ ] `rg "CorsLayer::permissive"` returns nothing.
- [ ] Push the branch and confirm the CI run is green before opening a PR (see `superpowers:finishing-a-development-branch`).
