# Codex brief: split `mobility/systems.rs` (parallel-safe refactor)

This brief is written to run **concurrently** with another agent that is doing
the security + CI hardening on branch `plan/security-ci-guardrails`. Stay inside
the boundaries below and we won't collide.

## Goal

`backend/crates/sim-core/src/mobility/systems.rs` is 3545 lines — ~16 ECS
systems + private helpers + ~1100 lines of tests in one file. Split it into
focused submodules. **Pure refactor: zero behavior change.** Same tests, same
public API.

## Hard constraints (read first)

1. **Build isolation — non-negotiable.** Use a separate target dir for *every*
   cargo command so you never share `backend/target/` with the other agent
   (shared target = build-lock stalls that look like minute-long hangs):
   ```
   export CARGO_TARGET_DIR=/tmp/abutown-codex-target
   ```
   Run cargo **serially** (never two cargo at once). If you stay in the repo's
   default target instead, route cargo through `scripts/cargo-serial.sh`.

2. **Branch:** start a fresh branch off the latest `main`
   (`git checkout main && git pull && git checkout -b codex/split-mobility-systems`).
   Do **not** work on `plan/security-ci-guardrails`.

3. **Do NOT touch these files — the other agent owns them right now:**
   - `backend/crates/sim-server/src/runtime.rs`, `app.rs`, `config.rs`, `card_hand.rs`
   - `backend/crates/sim-core/examples/profile_lod_tick.rs` (knowingly broken;
     references a deleted `boarding_alighting_system` — the other agent fixes it)
   - `.github/workflows/ci.yml`, `package.json`, `tsconfig*.json`
   - anything under `src/` (TypeScript) or `tests/` (TypeScript)

4. **Preserve the public path surface.** External code uses
   `sim_core::mobility::systems::<system_name>` (the plugin, the example). After
   the split, keep `pub use` re-exports in `mobility/systems/mod.rs` (or
   `mobility/mod.rs`) so every existing `mobility::systems::*` path still
   resolves unchanged. Do not rename systems.

5. **Verify with the scoped command only:**
   ```
   CARGO_TARGET_DIR=/tmp/abutown-codex-target cargo test --manifest-path backend/Cargo.toml -p sim-core
   ```
   Do **NOT** use `--workspace` or `--all-targets` — that pulls in the broken
   example (owned by the other agent) and the sim-server crate (its files are
   changing under you).

## Approach (TDD-flavoured, small reversible steps)

1. Read `systems.rs` and group the systems by concern. Expected groupings
   (confirm against the actual code): routing/route-assignment, walking,
   vehicles, LOD transitions, chunk/flow bookkeeping.
2. Create `backend/crates/sim-core/src/mobility/systems/` with `mod.rs` and one
   submodule per concern, e.g. `routing.rs`, `walking.rs`, `vehicles.rs`,
   `lod.rs`, `bookkeeping.rs`. Move tests next to the code they cover (or into a
   `systems/tests.rs`).
3. Move ONE concern at a time. After each move: `cargo test -p sim-core` green,
   then commit (`refactor(mobility): extract <concern> systems`). Small commits
   make the eventual rebase onto the security branch tractable.
4. Keep `pub use` re-exports so the public API is byte-for-byte compatible.
5. Run `cargo fmt --manifest-path backend/Cargo.toml --all` as the final step so
   the diff is formatting-clean.

## Merge expectation

Your branch and `plan/security-ci-guardrails` both touch `systems.rs` only via
formatting on the other side, so the only conflict at merge time is the global
`cargo fmt`; resolve by taking your refactored version and re-running `cargo
fmt`. Coordinate the merge order with the human.

## Definition of done

- `systems.rs` is replaced by a `systems/` module of focused files, none doing
  too much.
- `cargo test -p sim-core` passes with the same test count/results as before.
- `mobility::systems::*` public paths are unchanged (plugin + example still
  reference them).
- No files outside `backend/crates/sim-core/src/mobility/` changed (except the
  unavoidable `mobility/mod.rs` re-export line, if any).
