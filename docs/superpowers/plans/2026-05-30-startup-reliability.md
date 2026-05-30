# Startup Reliability (panic → Result) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A malformed/missing base world or bad input makes the server fail to start with a clear typed error — never `panic!`. Happy-path boot is byte-for-byte unchanged.

**Architecture:** Push data-integrity checks into `BaseWorldBundle::validate()` (already `Result`, already run by `load_from_dir`), so downstream code is provably safe; make the remaining startup `expect`s either propagate `Result` on the production path or be documented genuine invariants. Key realization: the production path (`main.rs` → `build_app_from_config`) already loads the world via `?`; the only production panic is a **redundant** second load inside `AppState::new_with_stores` (app.rs:190) — fixed by passing the already-loaded bundle in.

**Tech Stack:** Rust, `thiserror` (BaseWorldError), `anyhow` (server startup).

**Spec:** `docs/superpowers/specs/2026-05-30-startup-reliability-design.md`

**Branch / isolation:** worktree `/Users/ramonfuglister/Coding/abutown-reliability` on `plan/startup-reliability` (from `origin/main` 773b5f3). `export CARGO_TARGET_DIR=/tmp/abutown-reliability-target`. Every cargo via `scripts/cargo-serial.sh`; `cargo fmt --check` in every task verify.

## Grounding (verified on this branch)
- `BaseWorldBundle::load_from_dir` (base_world.rs:198) ends with `bundle.validate()?` — so anything added to `validate()` runs at load.
- `BaseWorldError` (base_world.rs:15) variants: `MissingManifest, Read, Parse, UnsupportedSchema, WorldIdMismatch, EmptyLayer(&'static str), OutOfBounds{x,y,width,height}`.
- `validate()` (base_world.rs:224) already bounds-checks roads/rails/paths/footprints/decorations but does **not** check spawn-group references or world-dimension i32 range.
- `chunk_coords()` (base_world.rs ~303) + `tiles_for_chunk()` (~334) use `i32::try_from(..).expect(..)` on chunk/local indices derived from `world_tiles.{width,height}`.
- `runtime.rs` `expected_base_world_car_routes` (113), `_driver_vehicles` (140), `_pedestrian_walks` (173) each `.unwrap_or_else(|| panic!("… references missing arterial/corridor …"))`; called only from `mobility_snapshot_matches_base_world` (`-> bool`, runtime.rs:68) and `expected_base_world_car_count` (222) + tests (2035, 2260).
- `app.rs`: production boot is `main.rs` → `build_app_from_config` (app.rs:394) which does `BaseWorldBundle::load_from_dir(...)?` then `AppState::new_with_stores(runtime, snapshot_store, mobility_snapshot_store, card_hands, auth)` (no bundle passed). `new_with_stores` (179) **re-loads** the bundle: `load_from_dir(resolve_base_world_path()).expect("base world bundle is required for app state")` (190). `new` (155) + `new_with_card_hands` (165) delegate to `new_with_stores`. `build_app_with_allowed_origins` (380, `-> anyhow::Result<Router>`) and `build_app` (374, `-> Router`) call `SimulationRuntime::new_from_base_world_dir(...).expect("…app startup")` (376/382). `resolve_base_world_path` (48) has `.nth(3).expect("sim-server crate lives under …")`. `cors_layer(&[]).expect("empty origin list is always valid")` (442).

---

## Task 1: `validate()` catches bad references + oversized dimensions

**Files:** Modify `backend/crates/sim-core/src/base_world.rs` (error enum ~14, `validate()` ~224, `chunk_coords`/`tiles_for_chunk` ~303/334). Test: inline `#[cfg(test)] mod tests` in the same file.

- [ ] **Step 1: Failing tests** (add to base_world.rs tests). Mirror how existing tests build a bundle — most load the real abutopia via a `load_from_dir(workspace_root().join("data/worlds/abutopia"))` helper; for the negative cases mutate a loaded bundle's `spawns`/`manifest` in memory then call `.validate()`:
```rust
    #[test]
    fn validate_rejects_car_group_with_missing_arterial() {
        let mut b = load_abutopia();
        b.spawns.car_groups.push(crate::base_world::CarGroup {
            id: "spawn:bad".into(),
            arterial_id: "arterial:does-not-exist".into(),
            cars_per_arterial: 1,
        });
        assert!(matches!(b.validate(), Err(BaseWorldError::MissingArterialRef { .. })));
    }

    #[test]
    fn validate_rejects_pedestrian_group_with_missing_corridor() {
        let mut b = load_abutopia();
        b.spawns.pedestrian_groups[0].corridor_id = "corridor:nope".into();
        assert!(matches!(b.validate(), Err(BaseWorldError::MissingCorridorRef { .. })));
    }

    #[test]
    fn validate_rejects_world_dimensions_that_overflow_i32() {
        let mut b = load_abutopia();
        b.manifest.world_tiles.width = u32::MAX;
        assert!(matches!(b.validate(), Err(BaseWorldError::WorldTooLarge { .. })));
    }

    #[test]
    fn validate_accepts_real_abutopia() {
        assert!(load_abutopia().validate().is_ok());
    }
```
Add a `fn load_abutopia() -> BaseWorldBundle` test helper if none exists: `BaseWorldBundle::load_from_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().join("data/worlds/abutopia")).expect("abutopia loads")`. (Confirm the exact `CarGroup`/`PedestrianGroup`/`SpawnLayer` field names against the structs in this file before writing — adjust the literals to match.)
RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core base_world` → the 3 negative tests FAIL (currently `validate()` ignores these), `validate_accepts_real_abutopia` passes.

- [ ] **Step 2: Add error variants** to `BaseWorldError` (base_world.rs:14), matching the existing `thiserror` style:
```rust
    #[error("base world spawn group {group} references missing arterial {arterial_id}")]
    MissingArterialRef { group: String, arterial_id: String },
    #[error("base world spawn group {group} references missing corridor {corridor_id}")]
    MissingCorridorRef { group: String, corridor_id: String },
    #[error("base world dimensions {width}x{height} are too large to index")]
    WorldTooLarge { width: u32, height: u32 },
```

- [ ] **Step 3: Add checks to `validate()`** (base_world.rs:224, before the final `Ok(())`):
```rust
        // World dimensions must fit i32 (chunk/tile index conversions rely on it).
        if i32::try_from(self.manifest.world_tiles.width).is_err()
            || i32::try_from(self.manifest.world_tiles.height).is_err()
        {
            return Err(BaseWorldError::WorldTooLarge {
                width: self.manifest.world_tiles.width,
                height: self.manifest.world_tiles.height,
            });
        }
        // Spawn-group references must resolve.
        for group in &self.spawns.car_groups {
            if !self.transport.arterial_paths.iter().any(|p| p.id == group.arterial_id) {
                return Err(BaseWorldError::MissingArterialRef {
                    group: group.id.clone(),
                    arterial_id: group.arterial_id.clone(),
                });
            }
        }
        for group in &self.spawns.pedestrian_groups {
            if !self.transport.pedestrian_corridors.iter().any(|c| c.id == group.corridor_id) {
                return Err(BaseWorldError::MissingCorridorRef {
                    group: group.id.clone(),
                    corridor_id: group.corridor_id.clone(),
                });
            }
        }
```
(Match the real field names for car/pedestrian groups — `arterial_id`/`corridor_id`/`id` per `expected_base_world_*` in runtime.rs.)

- [ ] **Step 4: Document the now-safe `expect`s** in `chunk_coords()` and `tiles_for_chunk()`: change each `.expect("chunk x fits i32")` / `.expect("local x fits i32")` message to `.expect("world dims fit i32 — enforced by validate()")` so the invariant's source is explicit. (No signature change — `validate()` guarantees it.)

- [ ] **Step 5: Verify** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core base_world` (all green) · `clippy -p sim-core --all-targets -- -D warnings` · `fmt --all -- --check`.

- [ ] **Step 6: Commit**
```
git add -A && git commit -m "fix(base-world): validate() rejects dangling spawn refs + oversized dims

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `runtime.rs` reference lookups degrade instead of panic

**Files:** Modify `backend/crates/sim-server/src/runtime.rs` (`expected_base_world_car_routes` 113, `_driver_vehicles` 140, `_pedestrian_walks` 173). Test: inline runtime.rs tests (`mod tests` at 1003).

Because `load_from_dir` now validates references, a bundle reaching these helpers is sound and the lookups cannot miss. Replace the `panic!` with a defensive skip so an un-validated caller degrades (drops the group) instead of aborting the process.

- [ ] **Step 1: Failing test** (runtime.rs tests) — a hand-built bundle (bypassing `load_from_dir`, so unvalidated) with a dangling car-group ref must NOT panic; the expected map simply omits it:
```rust
    #[test]
    fn expected_car_routes_skips_dangling_arterial_without_panicking() {
        let mut b = super::tests_support::abutopia_bundle(); // or load_from_dir helper used elsewhere
        b.spawns.car_groups.push(CarGroup {
            id: "spawn:bad".into(),
            arterial_id: "arterial:missing".into(),
            cars_per_arterial: 3,
        });
        // Must not panic; the dangling group contributes nothing.
        let routes = super::expected_base_world_car_routes(&b);
        assert!(!routes.keys().any(|k| k.contains("bad")));
    }
```
(Use whatever bundle-construction the existing runtime.rs tests use — grep `expected_base_world_car_count` test sites at 2035/2260 for the pattern. `expected_base_world_car_routes` is module-private, so the test lives in the same crate's `mod tests`.)
RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server expected_car_routes_skips` → FAIL (currently panics).

- [ ] **Step 2: Replace the panics with skips** in all three helpers. For each `.position(...).unwrap_or_else(|| panic!(...))`, use:
```rust
            let Some(arterial_index) = base_world
                .transport
                .arterial_paths
                .iter()
                .position(|path| path.id == group.arterial_id)
            else {
                continue; // unreachable after validate(); skip defensively rather than abort
            };
```
and the corridor equivalent in `_pedestrian_walks` (`continue` the pedestrian-group loop).

- [ ] **Step 3: Verify** — the new test passes; `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server` green; `clippy -p sim-server --all-targets -- -D warnings`; `fmt --check`.

- [ ] **Step 4: Commit**
```
git add -A && git commit -m "fix(runtime): base-world reference lookups skip instead of panic

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `app.rs` startup propagates errors; no redundant load

**Files:** Modify `backend/crates/sim-server/src/app.rs` (`AppState::new_with_stores` 179 + `new`/`new_with_card_hands` 155/165, `build_app_with_allowed_origins` 380, `build_app_from_config` 394, `resolve_base_world_path` 48, CORS at 442). Test: `backend/crates/sim-server/tests/http.rs` (or inline) for the happy path; existing tests must stay green.

**Approach:** Eliminate the redundant in-`AppState` re-load by passing the already-loaded bundle in. The production path (`build_app_from_config`) already has it via `?`. Test conveniences (`new`/`new_with_card_hands`) load it themselves (documented). Genuine invariants (crate-path, empty-CORS) stay as documented `expect`s.

- [ ] **Step 1: Failing test** — building the app from a config pointed at a missing world returns `Err`, not a panic. Add to http.rs (set `ABUTOWN_BASE_WORLD_PATH` to a nonexistent dir for the duration):
```rust
    #[tokio::test]
    async fn build_app_from_config_errors_on_missing_base_world() {
        // SAFETY: test sets a process env var; run serially if needed.
        unsafe { std::env::set_var("ABUTOWN_BASE_WORLD_PATH", "/nonexistent/abutopia-xyz"); }
        let cfg = test_config(); // existing helper; DB url etc.
        let result = sim_server::app::build_app_from_config(&cfg).await;
        unsafe { std::env::remove_var("ABUTOWN_BASE_WORLD_PATH"); }
        assert!(result.is_err(), "missing base world must be a clean Err, not a panic");
    }
```
(Adapt to the existing http.rs config/test helpers — confirm `test_config`/DB handling; if env-var mutation is unsafe in parallel, gate with the crate's existing serial-test pattern.)
RUN: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server build_app_from_config_errors` → may already pass if the early `load_from_dir(...)?` (app.rs:395) catches it before `new_with_stores`. If it PASSES already, keep it as a regression guard and note the early `?` is the reason; still do Steps 2-3 to remove the redundant panic-on-reload.

- [ ] **Step 2: Pass the bundle into `new_with_stores`** — change its signature to accept the already-loaded bundle, removing the internal re-load + `.expect()` (app.rs:190):
```rust
    pub fn new_with_stores(
        runtime: SimulationRuntime,
        base_world: &BaseWorldBundle,
        snapshot_store: Box<dyn ChunkSnapshotStore + Send + Sync>,
        mobility_snapshot_store: Box<dyn MobilitySnapshotStore + Send + Sync>,
        card_hands: CardHandStore,
        auth: AuthVerifier,
    ) -> Self {
        // … unchanged …
        let base_world_response = Arc::new(BaseWorldResponse::from(base_world));
        // … use base_world_response where `Arc::new(BaseWorldResponse::from(&base_world))` was …
    }
```
Remove the `let base_world = BaseWorldBundle::load_from_dir(...).expect(...)` line.

- [ ] **Step 3: Update callers.**
  - `build_app_from_config` (394): it already has `base_world` from `load_from_dir(...)?`; pass `&base_world` into `new_with_stores`.
  - `new` (155) + `new_with_card_hands` (165): load the bundle themselves and pass it, keeping a documented test-convenience `expect`:
    ```rust
        let base_world = BaseWorldBundle::load_from_dir(resolve_base_world_path())
            .expect("base world bundle present (test/dev convenience; production uses build_app_from_config)");
        Self::new_with_stores(runtime, &base_world, /* … */)
    ```
  - `build_app_with_allowed_origins` (380, already `-> Result`): replace `SimulationRuntime::new_from_base_world_dir(...).expect("…app startup")` (382) with `?`.
  - `build_app` (374, `-> Router`, dev/test): keep `.expect(...)` but append a doc comment `// dev/test entry; production uses build_app_from_config which propagates errors`.
  - `resolve_base_world_path` (48) `.nth(3).expect(...)` and `cors_layer(&[]).expect("empty origin list is always valid")` (442): leave as-is, add a one-line comment each noting they are infallible invariants (fixed crate layout / hardcoded empty slice).

- [ ] **Step 4: Verify** — `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server` (all green, incl. existing http/websocket tests) · `clippy -p sim-server --all-targets -- -D warnings` · `fmt --check` · `build -p sim-server`.

- [ ] **Step 5: Commit**
```
git add -A && git commit -m "fix(app): startup propagates base-world errors; drop redundant reload

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Final gate + PR
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
- [ ] `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server`
- [ ] PR → confirm CI green with `gh run watch <id> --exit-status` → merge → `superpowers:finishing-a-development-branch`.

## Deferred (not this stream)
- World-drift hardening / data-driven activity geometry (stream ②).
- god-file splits of runtime.rs + app.rs (stream ③).
- CHUNK_SIZE parametrization.
