# God-File Splits — runtime.rs / app.rs (Reliability stream ③)

Date: 2026-05-30

## Status

Approved scope (stream ③ of the 3-stream reliability/refactor pass: ① startup
reliability [merged #42], ② world-drift hardening [merged #43], ③ god-file
splits). Own branch + PR (`plan/godfile-splits`, from `origin/main` 5676c35).

## Problem

Two sim-server files have grown into god-files that are hard to hold in context
and edit reliably:
- `runtime.rs` — 2334 lines (~993 production + ~1340 test).
- `app.rs` — 1927 lines (~1453 production + ~470 test).

## Goal

Split each into a **directory module** with cohesive, well-bounded submodules.
**Pure mechanical move — zero behavior change.** Public API (re-exported from
the module root) is unchanged so external callers (`main.rs`, tests, other
crates) keep compiling without edits.

## Architecture

### `runtime.rs` → `runtime/`
- `runtime/mod.rs`: `SimulationRuntime` struct + its `impl` blocks, `Default`,
  `refresh_flow_field_resources`, `default_base_world_path`, the consts
  (`BASE_WORLD_DEFAULT_PATH`, `TICK_PERIOD_MS`, `SEED_DENSITY`). Re-exports the
  submodule items it needs. Declares `mod base_world_expectations;` + `#[cfg(test)] mod tests;`.
- `runtime/base_world_expectations.rs`: `initial_mobility_snapshot_for_base_world`,
  `mobility_snapshot_matches_base_world`, `expected_base_world_car_routes`,
  `expected_base_world_driver_vehicles`, `expected_base_world_pedestrian_walks`,
  `ExpectedPedestrianWalk`, `polylines_match`, `expected_base_world_car_count`
  (runtime.rs:61-216). `pub(crate)`/`pub(super)` as needed for the items `mod.rs`
  calls; `use super::*` or explicit imports for their deps.
- `runtime/tests.rs`: the `#[cfg(test)] mod tests` body (runtime.rs:994-2334),
  with `use super::*;` (+ `use super::super::*` style as the compiler requires).

### `app.rs` → `app/`
- `app/mod.rs`: `AppState` + impl, the `build_app*`/`cors_layer`/`build_router_from_state`
  builders, the HTTP handlers (`health`/`world`/`mobility`/`base_world`/`cards`/
  `card_hand`/`chunk`/`command`/`websocket`), `ConnectionState` +
  `stream_world_deltas`, the tick loop (`tick_loop`/`tick_once`/`apply_mutation_owned`/
  `persist_snapshots_once`/`handle_client_message`/`send_server_message`),
  `build_read_view_from_runtime`, `ProtoBody`/`proto_response`, consts,
  `resolve_base_world_path`. Declares the submodules.
- `app/proto_convert.rs`: the DTO→proto helpers (app.rs:790-1007 +
  `chunk_delta_to_dto`/`chunk_snapshot_to_dto` if cleanly separable):
  `direction_to_proto`, `tile_kind_to_proto`, `chunk_state_to_proto`,
  `agent_dto_to_proto`, `vehicle_dto_to_proto`, `stop_dto_to_proto`,
  `world_summary_dto_to_proto`, `health_dto_to_proto`, `chunk_snapshot_dto_to_proto`,
  `mobility_snapshot_dto_to_proto`, `tile_pulse_dto_to_proto`, `world_event_dto_to_proto`.
  These are pure functions — the lowest-risk extraction. `pub(crate)`/`pub(super)`.
- `app/base_world_response.rs`: `BaseWorldResponse` + the nested
  `BaseWorld{Terrain,TerrainTile,Transport,Building,Decoration}Response` structs +
  `impl From<&BaseWorldBundle> for BaseWorldResponse` (app.rs:62-141). These are
  `pub` (returned by the `base_world` handler / serialized).
- `app/tests.rs`: the `#[cfg(test)] mod tests` body (app.rs:1454-1927).

### Module conversion mechanics
- Replace `pub mod app;` / `pub mod runtime;` in `lib.rs` with the directory form
  (Rust resolves `app/mod.rs` automatically — `lib.rs` is unchanged if it just
  says `pub mod app;`). Delete the old `app.rs`/`runtime.rs` after moving content
  into `<name>/mod.rs`.
- Visibility: items moved out but used by the root become `pub(super)` or
  `pub(crate)`; the root `mod.rs` re-`pub use`s anything that was `pub` and is part
  of the crate's API surface (so `sim_server::app::build_app_from_config`,
  `sim_server::runtime::SimulationRuntime`, `sim_server::app::BaseWorldResponse`,
  etc. resolve unchanged).

## Testing

Behavior is unchanged, so the existing suite is the spec:
- After each extraction, `cargo build -p sim-server` + the **full** `cargo test
  --workspace` stays green (the same tests, now in submodules).
- `clippy --workspace --all-targets -D warnings` clean (watch for newly-required
  `pub(crate)` / unused-import churn).
- `fmt --check` clean.
- No public-path change: `main.rs` and the `tests/` integration files compile
  **without edits** (proves the re-exports preserve the API).

## What this is NOT

- No behavior change, no new error handling, no API change. If a "while I'm here"
  improvement is tempting, resist it — this stream is purely structural.
- Not splitting the big `impl SimulationRuntime` block across files (an impl can
  legally span files but it adds churn for little clarity gain) — keep the impl in
  `runtime/mod.rs`. Only the cohesive free-function/struct groups move out.
- Not CHUNK_SIZE parametrization.

## Open questions (resolve in planning, against real code)

1. Exact visibility each moved item needs (`pub(super)` vs `pub(crate)`) — let the
   compiler drive it; start with `pub(crate)` for cross-module-used items.
2. Whether `chunk_delta_to_dto`/`chunk_snapshot_to_dto` belong in `proto_convert`
   or stay in `mod.rs` (they bridge DTO+domain) — keep in `mod.rs` if they pull in
   non-proto deps; only move the pure `*_to_proto` converters.
3. Order of extraction to keep every commit green (smallest/most-isolated first:
   proto_convert, then base_world_response/base_world_expectations, then tests,
   leaving `mod.rs` as the remainder).
