# God-File Splits (runtime.rs / app.rs) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax. PURELY MECHANICAL — move code, fix visibility, zero behavior change.

**Goal:** Split `sim-server`'s `runtime.rs` (2334 L) and `app.rs` (1927 L) into directory modules with cohesive submodules, no behavior/API change.

**Architecture:** Convert each file to `<name>/mod.rs` + extract cohesive groups into sibling files; re-export the public surface from the root so external callers compile unchanged.

**Tech Stack:** Rust modules, `git mv`, `pub(crate)`/`pub(super)` visibility.

**Spec:** `docs/superpowers/specs/2026-05-30-godfile-splits-design.md`

**Branch / isolation:** worktree `/Users/ramonfuglister/Coding/abutown-splits` on `plan/godfile-splits` (from `origin/main` 5676c35). `export CARGO_TARGET_DIR=/tmp/abutown-splits-target`. Every cargo via `scripts/cargo-serial.sh`; `fmt --check` + `clippy -p sim-server --all-targets -D warnings` + `test --workspace` green per task (behavior is unchanged, so the existing suite is the gate). `pgrep -f cargo` before each cargo.

## General mechanic (every extraction)
1. If the file isn't yet a directory module, `git mv backend/crates/sim-server/src/<name>.rs backend/crates/sim-server/src/<name>/mod.rs` (lib.rs already says `pub mod <name>;` — Rust resolves the dir form, NO lib.rs edit).
2. Create the sibling file; **move** (cut, don't copy) the group's items into it with `use super::*;` (or explicit `use`s) at the top.
3. In `mod.rs`: add `mod <sub>;` (or `pub(crate) mod`), and `pub use <sub>::*;` / explicit re-exports for items that were `pub` and are part of the crate's API or used by other modules.
4. Fix visibility: items the root still calls become `pub(crate)` or `pub(super)`. Let `cargo build -p sim-server` drive it (iterate on the errors).
5. Verify (build + clippy + test --workspace) green; commit.

---

## Task 1: `app.rs` → `app/` + extract `proto_convert` (lowest-risk: pure fns)

**Files:** `git mv app.rs app/mod.rs`; create `app/proto_convert.rs`.

- [ ] **Step 1:** `git mv backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/app/mod.rs`. RUN `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server` → still compiles (pure move).
- [ ] **Step 2:** Create `backend/crates/sim-server/src/app/proto_convert.rs`. **Move** these pure converters out of `mod.rs` into it (the contiguous block around app/mod.rs lines ~790-1007): `direction_to_proto`, `tile_kind_to_proto`, `chunk_state_to_proto`, `agent_dto_to_proto`, `vehicle_dto_to_proto`, `stop_dto_to_proto`, `world_summary_dto_to_proto`, `health_dto_to_proto`, `chunk_snapshot_dto_to_proto`, `mobility_snapshot_dto_to_proto`, `tile_pulse_dto_to_proto`, `world_event_dto_to_proto`. Add at the top of the new file the imports they need (grep their bodies: `use abutown_protocol as ...`? `use crate::ws as w;`? — copy the exact `use`/alias the originals relied on; `w::` is the `ws` proto alias). Mark each `pub(crate) fn` (the root + tick loop call them).
- [ ] **Step 3:** In `app/mod.rs` add `mod proto_convert;` and `use proto_convert::*;` (so existing unqualified call sites resolve). RUN build → fix any visibility/import errors the compiler reports (iterate). Do NOT move functions that pull in non-proto domain deps if they cause import sprawl — leave `chunk_delta_to_dto`/`chunk_snapshot_to_dto` in `mod.rs` (they bridge domain+proto).
- [ ] **Step 4:** Verify: `scripts/cargo-serial.sh build -p sim-server` · `clippy --manifest-path backend/Cargo.toml -p sim-server --all-targets -- -D warnings` · `test --manifest-path backend/Cargo.toml --workspace` (all green) · `fmt --all -- --check`.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "refactor(app): extract proto_convert module from app.rs\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"`

---

## Task 2: extract `app/base_world_response.rs`

**Files:** create `app/base_world_response.rs`.

- [ ] **Step 1:** Create `backend/crates/sim-server/src/app/base_world_response.rs`. **Move** `BaseWorldResponse`, `BaseWorldTerrainResponse`, `BaseWorldTerrainTileResponse`, `BaseWorldTransportResponse`, `BaseWorldBuildingResponse`, `BaseWorldDecorationResponse`, and `impl From<&BaseWorldBundle> for BaseWorldResponse` (app/mod.rs ~62-141) into it. These are `pub` (serialized + returned by the `base_world` handler). Add imports: `use serde::Serialize;`, `use sim_core::base_world::BaseWorldBundle;`, and whatever field types reference (`sim_core::base_world::*` — copy from the originals).
- [ ] **Step 2:** In `app/mod.rs`: `mod base_world_response;` + `pub use base_world_response::*;` (preserves `sim_server::app::BaseWorldResponse`). RUN build → fix imports.
- [ ] **Step 3:** Verify (build · clippy · test --workspace · fmt --check) green.
- [ ] **Step 4:** Commit `refactor(app): extract base_world_response DTOs from app.rs`.

---

## Task 3: extract `app/tests.rs`

**Files:** create `app/tests.rs`.

- [ ] **Step 1:** Move the entire `#[cfg(test)] mod tests { … }` body (app/mod.rs ~1454-end) into `backend/crates/sim-server/src/app/tests.rs` (the file content is the INNER body of the module). In `app/mod.rs` replace the inline module with `#[cfg(test)] mod tests;`. At the top of `tests.rs` add `use super::*;` (the tests referenced the parent's items unqualified). RUN `test --workspace` → fix any remaining path issues (some tests may need `use super::super::...` or explicit imports the compiler points out).
- [ ] **Step 2:** Verify (build · clippy --all-targets · test --workspace · fmt --check) green.
- [ ] **Step 3:** Commit `refactor(app): move app tests into app/tests.rs`.

---

## Task 4: `runtime.rs` → `runtime/` + extract `base_world_expectations`

**Files:** `git mv runtime.rs runtime/mod.rs`; create `runtime/base_world_expectations.rs`.

- [ ] **Step 1:** `git mv backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/runtime/mod.rs`. RUN build → compiles.
- [ ] **Step 2:** Create `runtime/base_world_expectations.rs`. **Move** (runtime/mod.rs ~61-216): `initial_mobility_snapshot_for_base_world`, `mobility_snapshot_matches_base_world`, `expected_base_world_car_routes`, `expected_base_world_driver_vehicles`, `ExpectedPedestrianWalk`, `expected_base_world_pedestrian_walks`, `polylines_match`, and the `#[cfg(test)] fn expected_base_world_car_count`. Add imports the bodies need (`use sim_core::base_world::BaseWorldBundle;`, `use crate::MobilityPersistSnapshot;`/the real path, `use sim_core::mobility::...`, `extract_from_world` — copy from runtime/mod.rs's `use`s). Mark used-by-root items `pub(crate)`.
- [ ] **Step 3:** In `runtime/mod.rs`: `mod base_world_expectations;` + `use base_world_expectations::*;`. RUN build → fix visibility/imports (iterate). Note `mobility_snapshot_matches_base_world` + `expected_base_world_car_count` are referenced by the `tests` module too — `pub(crate)` covers it.
- [ ] **Step 4:** Verify (build · clippy --all-targets · test --workspace · fmt --check) green.
- [ ] **Step 5:** Commit `refactor(runtime): extract base_world_expectations from runtime.rs`.

---

## Task 5: extract `runtime/tests.rs`

**Files:** create `runtime/tests.rs`.

- [ ] **Step 1:** Move the `#[cfg(test)] mod tests { … }` body (runtime/mod.rs ~994-end) into `runtime/tests.rs`; replace inline with `#[cfg(test)] mod tests;`. Also handle the `#[cfg(test)] impl SimulationRuntime { … }` block (runtime/mod.rs ~980-992) — keep it in `mod.rs` (it's a test-only impl on the parent type; leave it where it is, only the `mod tests` moves). At the top of `tests.rs`: `use super::*;`. RUN `test --workspace` → fix path issues the compiler reports.
- [ ] **Step 2:** Verify (build · clippy --all-targets · test --workspace · fmt --check) green.
- [ ] **Step 3:** Commit `refactor(runtime): move runtime tests into runtime/tests.rs`.

---

## Task 6: Final gate + PR
- [ ] `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
- [ ] `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
- [ ] `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
- [ ] `scripts/cargo-serial.sh build --manifest-path backend/Cargo.toml -p sim-server`
- [ ] Confirm `main.rs` + `tests/` compiled with NO edits (proves the public API is preserved).
- [ ] PR → CI green via `gh run watch <id> --exit-status` → merge → finishing-a-development-branch.

## Self-review note
Every task is a pure move + visibility fix; the unchanged `cargo test --workspace` is the behavior spec. If any task needs a logic change to compile, STOP — that means a hidden coupling; report it rather than changing behavior.
