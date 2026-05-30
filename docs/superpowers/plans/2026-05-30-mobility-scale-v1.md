# Mobility Scale V1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Seed, render, persist, and smoke-check 300 real backend-authored Abutopia walking agents.

**Architecture:** Keep Abutopia geometry unchanged. Raise the base-world spawn count in the generator and generated data, and update backend/e2e expectations to treat 300 concrete agents as the authored contract.

**Tech Stack:** Rust 2024, Bevy ECS, Axum, Supabase/Postgres, TypeScript/Vite/Playwright, Vitest.

---

### Task 1: RED Agent Count Contract

**Files:**
- Modify: `backend/crates/sim-core/tests/abutopia_bundle.rs`
- Modify: `backend/crates/sim-server/src/runtime/tests.rs`
- Modify: `backend/crates/sim-server/tests/http.rs`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [x] **Step 1: Update tests to expect 300 Abutopia agents**
  - In bundle tests, assert `agents_per_corridor == 300`.
  - In runtime tests, assert `agents.len() == 300`, `agent:walk:0` exists, and `agent:walk:299` exists.
  - In HTTP tests, assert `/mobility` returns 300 agents.
  - In e2e smoke, assert backend mobility diagnostics and rendered agent count are 300.

- [x] **Step 2: Run targeted tests and confirm RED**
  - `export CARGO_TARGET_DIR=/tmp/abutown-mobility-scale-v1-target`
  - `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --test abutopia_bundle`
  - `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server runtime_seeds_backend_pedestrian_from_base_world mobility_snapshot_is_available`
  - `npm run test -- tests/e2e/render-smoke.spec.ts`
  - Expected: tests fail because current spawns still author one agent.

### Task 2: GREEN Spawn Scale

**Files:**
- Modify: `scripts/generate-abutopia-world.mjs`
- Modify: `data/worlds/abutopia/layers/spawns.json`
- Modify: `backend/README.md`

- [x] **Step 1: Raise the authored spawn count**
  - Add a named `pedestrianAgentsPerCorridor = 300` constant in the generator.
  - Use it for `agents_per_corridor`.
  - Update generated `spawns.json` to `300`.
  - Update `backend/README.md` from one walking agent to 300 walking agents.

- [x] **Step 2: Run targeted tests and confirm GREEN**
  - Same commands as Task 1.
  - Expected: targeted Rust and frontend tests pass.

### Task 3: Full Verification

**Files:** no production changes unless verification finds a defect.

- [x] **Step 1: Rust gates**
  - `scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check`
  - `scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings`
  - `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace`

- [x] **Step 2: Frontend gates**
  - `npm install` if `node_modules` is missing in the worktree.
  - `npm run typecheck`
  - `npm run test`
  - `npm run build`

- [x] **Step 3: Supabase/live smoke**
  - Start the stack from this worktree with the ignored root `.env`.
  - Run `npm run smoke:mobility-persistence`.
  - Expected JSON includes `"expected_agents":300` and `"agents":300`.

### Task 4: Finish

**Files:** plan doc only if checklist status needs syncing.

- [x] **Step 1: Mark this plan complete**
- [x] **Step 2: Commit, push, open PR, wait for CI, merge to `main`, delete remote branch, remove worktree**
