# Backend-Required Mobility Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Require the Rust backend before frontend runtime startup and source mobility diagnostics from backend `/mobility` plus `/ws`.

**Architecture:** Add a small backend gate for `/health`, extend the mobility client with backend-default URL resolution and required initial snapshot loading, then wire both into `src/main.ts` before canvas boot. Keep existing local pedestrians and road vehicles as manifested selectable entities, but report backend mobility as the authoritative mobility source.

**Tech Stack:** TypeScript, Vite, Vitest, Playwright, Rust Axum backend on `127.0.0.1:8080`.

---

## File Structure

- Create `src/backend/backendGate.ts`: backend URL resolution, health DTO validation, and `requireBackend()`.
- Create `tests/backend/backendGate.test.ts`: unit coverage for healthy backend, transport failure, HTTP failure, invalid payload, and URL override.
- Modify `src/backend/mobilityClient.ts`: default to backend `8080`, export `resolveMobilityBackendBaseUrl()`, and export `requireMobilitySnapshot()`.
- Create `tests/backend/mobilityClient.test.ts`: unit coverage for required initial mobility snapshot and default backend URL.
- Modify `src/main.ts`: gate startup on backend health, require mobility snapshot before canvas boot, start websocket bridge after initial snapshot, render fatal backend-required status on failure, and update diagnostics.
- Modify `tests/e2e/render-smoke.spec.ts`: assert backend-sourced mobility diagnostics while preserving local manifested entities and selection.
- Modify `playwright.config.ts`: start backend and frontend preview as required E2E servers.
- Create `scripts/run-dev-stack.mjs`: one command for local backend plus Vite dev/preview.
- Modify `package.json`: add `dev:stack` and `preview:stack` scripts.

## Tasks

### Task 1: Backend Gate

**Files:**
- Create: `src/backend/backendGate.ts`
- Create: `tests/backend/backendGate.test.ts`

- [ ] Write failing tests for default URL, explicit URL override, valid health response, missing fetch, HTTP failure, and invalid payload.
- [ ] Run `npm test -- tests/backend/backendGate.test.ts`; expected failure because `backendGate.ts` does not exist.
- [ ] Implement `resolveBackendBaseUrl()`, `isBackendHealthDto()`, `requireBackend()`, and `backendErrorMessage()`.
- [ ] Run `npm test -- tests/backend/backendGate.test.ts`; expected pass.
- [ ] Commit with `git commit -m "feat: add backend startup gate"`.

### Task 2: Required Mobility Snapshot

**Files:**
- Modify: `src/backend/mobilityClient.ts`
- Create: `tests/backend/mobilityClient.test.ts`

- [ ] Write failing tests for default backend URL, override URL, successful `requireMobilitySnapshot()`, HTTP failure, and invalid payload failure.
- [ ] Run `npm test -- tests/backend/mobilityClient.test.ts`; expected failure because the exported API is missing.
- [ ] Implement `resolveMobilityBackendBaseUrl()` and `requireMobilitySnapshot()` using existing mobility protocol guards and reducers.
- [ ] Run `npm test -- tests/backend/mobilityClient.test.ts tests/backend/mobilityState.test.ts`; expected pass.
- [ ] Commit with `git commit -m "feat: require backend mobility snapshot"`.

### Task 3: Main Runtime Wiring

**Files:**
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] Update E2E expectations to require `city.backend.required === true`, `city.backend.status.ok === true`, `city.mobility.source === "backend"`, and `city.mobility.status === "connected"`.
- [ ] Run `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`; expected failure before main wiring.
- [ ] In `src/main.ts`, call `requireBackend()` before `mountCardHandView()` and `boot()`, pass the backend base URL to card-hand and mobility, require an initial mobility snapshot before setting `data-ready`, and start `connectMobilityBackend()` after the snapshot succeeds.
- [ ] Add a fatal `renderBackendRequired()` path that does not set `data-ready="true"` and does not start animation.
- [ ] Update `render_game_to_text()` mobility diagnostics to use backend state and include separate local manifested counts.
- [ ] Run `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`; expected pass with backend running.
- [ ] Commit with `git commit -m "feat: gate runtime on backend mobility"`.

### Task 4: Required Dev/Test Stack

**Files:**
- Modify: `playwright.config.ts`
- Create: `scripts/run-dev-stack.mjs`
- Modify: `package.json`

- [ ] Update Playwright config to start `cargo run --manifest-path backend/Cargo.toml -p sim-server` at `http://127.0.0.1:8080/health` and frontend preview at `http://127.0.0.1:5173`.
- [ ] Add `scripts/run-dev-stack.mjs` to run backend plus either Vite dev or Vite preview and forward shutdown signals.
- [ ] Add `dev:stack` and `preview:stack` scripts to `package.json`.
- [ ] Run `npm run build` and `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`; expected pass.
- [ ] Commit with `git commit -m "chore: require backend in dev stack"`.

### Task 5: Full Verification And Publish

**Files:**
- All files changed in this plan.

- [ ] Run `npm test`; expected all Vitest files pass.
- [ ] Run `npm run build`; expected TypeScript and Vite build pass.
- [ ] Run `npm run test:e2e -- tests/e2e/render-smoke.spec.ts`; expected pass.
- [ ] Run `cargo test --manifest-path backend/Cargo.toml --workspace`; expected pass.
- [ ] Run `git diff --check`; expected no output.
- [ ] Push branch, create PR to `codex/zurich-river-city-world`, merge it, fast-forward the main worktree, verify `http://127.0.0.1:5175/`, then remove the worktree and delete local/remote feature branches.
