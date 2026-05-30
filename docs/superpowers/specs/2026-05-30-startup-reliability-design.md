# Startup Reliability — panic → Result (Reliability stream ①)

Date: 2026-05-30

## Status

Approved scope (part 1 of a 3-stream reliability/refactor pass: ① startup
reliability, ② world-drift hardening, ③ god-file splits). This is stream ①,
its own branch + PR (`plan/startup-reliability`, branched fresh from
`origin/main` 773b5f3).

## Goal

A malformed or missing base world, a bad config, or an unreadable data file must
make the server **fail to start with a clear, typed error** — never `panic!`.
Today several startup paths `panic!`/`.expect()` on bad input, which aborts the
process with a backtrace instead of a clean error. The server entry point
(`main.rs`) already returns `anyhow::Result<()>`, and `BaseWorldBundle::load_from_dir`
already returns `Result<_, BaseWorldError>` — this stream extends that discipline
through the remaining boot-path holes.

## Scope (verified against current main)

### In scope — the boot path
1. **`runtime.rs` base-world reference panics** — `expected_base_world_car_routes`,
   `expected_base_world_driver_vehicles`, `expected_base_world_pedestrian_walks`
   each `.unwrap_or_else(|| panic!("… references missing arterial/corridor …"))`
   when a spawn group points at an arterial/corridor that does not exist
   (runtime.rs:124/152/185). These fire during `SimulationRuntime` construction
   (called at runtime.rs:72/84/85).
2. **`app.rs` startup expects** — `.expect("base world bundle is required for app
   state")` (190), `.expect("base world bundle is required for app startup")`
   (376, 382), the crate-path `.expect(...)` (55), and the CORS
   `.expect("empty origin list is always valid")` (442).
3. **`base_world.rs` internal conversions** — `i32::try_from(..).expect(..)`
   (306/307/334/336) and the transport-point `.expect(...)` (573).

### Out of scope (deliberate)
- **Per-tick simulation panics / invariant guards** deep in `sim-core` mobility
  and routing. Those assert genuine invariants (e.g. "a routed agent must
  resolve through the graph"); if they fire it is a real bug to fix, not input to
  handle gracefully. Converting them to `Result` would be churn that hides bugs.
- `.unwrap()` inside `#[cfg(test)]` modules (tests may panic; that is fine).
- CHUNK_SIZE parametrization (the literal `32`) — separate concern, deferred.

## Architecture / approach

**Validation belongs with the data; the boot path only propagates.**

1. **Move reference-integrity into `BaseWorldBundle::validate()`** (base_world.rs,
   already `-> Result<(), BaseWorldError>` and already run at load). Add checks:
   every `spawns.car_group.arterial_id` resolves to a `transport.arterial_paths`
   entry; every `spawns.pedestrian_group.corridor_id` resolves to a
   `transport.pedestrian_corridors` entry. New `BaseWorldError` variants
   (`MissingArterialRef { group, arterial_id }`, `MissingCorridorRef { group,
   corridor_id }`). After this, a loaded+validated bundle is guaranteed
   referentially sound.
2. **`runtime.rs` `expected_base_world_*`**: because the bundle is validated
   before a runtime is built, the `position(...)` lookups can no longer be
   missing. Replace `.unwrap_or_else(|| panic!(...))` with a non-panicking path:
   either `expect_validated(...)` documented as "validated upstream" or, cleaner,
   skip groups whose reference is absent (defensive, no panic). Pick the
   non-panicking form so a future un-validated caller degrades instead of
   aborting. These helpers stay private and keep their current signatures
   (`-> HashMap<...>`), so callers (incl. tests at 2035/2260) are unchanged.
3. **`app.rs` startup expects** → typed errors. `build_state` /the app builder
   return `anyhow::Result<_>` (or a `StartupError`) with `.context(...)`; the
   crate-path and "base world required" expects become `?` with context. The
   CORS `.expect("empty origin list is always valid")` is a true invariant on a
   hardcoded empty slice — keep it but document why, OR replace with a direct
   construction that cannot fail. (Resolve in planning; do not weaken a real
   invariant into a silent fallback.)
4. **`base_world.rs` `i32::try_from(..).expect(..)`** → `map_err` into a new
   `BaseWorldError::CoordOutOfRange { x, y }` (world dimensions that overflow
   `i32` are bad data, not an unreachable case).

## Error handling

- Reuse the existing `BaseWorldError` enum (base_world.rs) for all data/coord
  errors; add the variants above. Keep `thiserror` style consistent with the
  existing variants.
- `app.rs` startup uses `anyhow` with `.context(...)` (matches `main.rs`).
- No new error appears in the per-tick hot path.

## Testing

- **Validation unit tests** (base_world.rs): a bundle whose car group references
  a missing arterial → `Err(MissingArterialRef)`; pedestrian group → missing
  corridor → `Err(MissingCorridorRef)`; a world too large for `i32` →
  `Err(CoordOutOfRange)`. The real abutopia bundle still `Ok`.
- **Runtime construction**: building a `SimulationRuntime` from a bundle with a
  dangling reference returns `Err` (propagated) instead of panicking — assert via
  `new_from_base_world(bundle).is_err()` / matching the error.
- **app.rs startup**: building app state without a base world returns `Err` with
  a clear message (not a panic). Existing http/websocket integration tests stay
  green (the happy path is unchanged).
- Full gate: `cargo test --workspace`, `clippy --workspace --all-targets -D
  warnings`, `fmt --check`, `cargo build -p sim-server`.

## What this is NOT

- Not a behavior change on the happy path — a valid abutopia world boots
  identically. Only malformed/missing input changes from panic → typed error.
- Not the god-file split (stream ③) and not the activity-geometry hardcoding
  (stream ②); those are separate branches.

## Open questions (resolve in planning, against real code)

1. The exact caller of `expected_base_world_*` (runtime.rs ~60-90) and whether
   it already sits inside a `Result`-returning fn — confirm the propagation path.
2. Whether `app.rs`'s state/app builders already return `Result` or need their
   signatures changed (and who calls them).
3. The CORS `.expect` at app.rs:442 — is the empty-origin construction a genuine
   infallible invariant (keep + document) or worth making total?
