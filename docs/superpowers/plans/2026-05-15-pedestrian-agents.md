# Pedestrian Agents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Remove the hard-coded demo mobility-agent mapping and make the already-rendered pedestrians the primary local agents, including click selection.

**Architecture:** Add a small local-agent boundary in `src/render/pedestrianAgents.ts` that derives stable agent records from runtime pedestrians without duplicating rendering. `src/main.ts` owns selection state, hit testing, debug serialization, and visual selection rings around the existing pedestrian sprite.

**Tech Stack:** TypeScript, Vitest, Vite canvas app, Playwright smoke test.

---

### Task 1: Local Pedestrian Agent Projection

**Files:**
- Create: `src/render/pedestrianAgents.ts`
- Test: `tests/render/pedestrianAgents.test.ts`

- [x] Write failing tests for stable ids, position projection, status `walking`, and nearest-hit selection.
- [x] Run targeted test and verify it fails because the module is missing.
- [x] Implement `buildPedestrianAgents` and `findNearestPedestrianAgent`.
- [x] Run targeted test and verify it passes.

### Task 2: Remove Demo-Agent Map From App Surface

**Files:**
- Modify: `src/main.ts`
- Modify: `src/backend/mobilityState.ts`
- Delete: `src/render/mobilityOverlay.ts`
- Delete: `tests/render/mobilityOverlay.test.ts`
- Modify: `tests/backend/mobilityState.test.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [x] Write failing expectations that `render_game_to_text().city.localAgents` exposes pedestrian agents and no demo mobility agent is rendered.
- [x] Remove frontend demo marker projection and overlay drawing.
- [x] Keep backend mobility protocol parsing and connection state, but remove hard-coded demo coordinate mapping from frontend state.
- [x] Run targeted backend/render tests.

### Task 3: Click Selection

**Files:**
- Modify: `src/main.ts`
- Test: `tests/render/pedestrianAgents.test.ts`

- [x] Write failing tests for hit radius behavior.
- [x] Add canvas click handler that selects nearest local pedestrian agent.
- [x] Draw a restrained selection ring around the selected pedestrian only.
- [x] Expose `selectedAgentId` and selected agent metadata in `render_game_to_text()`.
