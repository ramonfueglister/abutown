# Mobility Population Integrity Plan

**Spec:** `docs/superpowers/specs/2026-05-30-mobility-population-integrity-design.md`
**Worktree:** `/Users/ramonfuglister/Coding/abutown/.worktrees/mobility-population-integrity`
**Branch:** `codex/mobility-population-integrity`
**Cargo:** `export CARGO_TARGET_DIR=/tmp/abutown-mobility-population-integrity-target`; every cargo command via `scripts/cargo-serial.sh`.

## Tasks

- [x] **Step 1: RED script smoke guard**
  - Add Vitest coverage that expected base-world agents are read from spawns and that runtime/persisted counts below expectation fail.
  - Run targeted Vitest and confirm the new tests fail.

- [x] **Step 2: GREEN script smoke guard**
  - Implement expected-agent parsing and assertion in `scripts/mobility-persistence-smoke-config.ts`.
  - Wire `scripts/smoke-mobility-persistence.mjs` to assert runtime and DB payload counts and print `expected_agents`.
  - Run targeted Vitest green.

- [x] **Step 3: RED backend runtime gate**
  - Add backend tests proving `/health` is unhealthy and persistence refuses to mark success when the mobility snapshot has fewer concrete agents than the base world expects.
  - Run targeted sim-server tests and confirm failure.

- [x] **Step 4: GREEN backend runtime gate**
  - Publish expected concrete agent count into `AppState`.
  - Degrade `/health` when the latest published mobility snapshot violates the contract.
  - Record a redacted mobility persistence failure and skip the mobility DB write for invalid snapshots.
  - Run targeted sim-server tests green.

- [x] **Step 5: Verification and integration**
  - `npm run test -- tests/scripts/mobilityPersistenceSmokeConfig.test.ts`
  - `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
  - `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
  - `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`
  - If credentials and ports are available, run `npm run smoke:mobility-persistence` against the local stack.

- [ ] **Step 6: Finish branch**
  - Commit intentionally, push to GitHub, open/merge PR, and clean up the isolated worktree.
