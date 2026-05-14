# OpenTTD Diorama Zurich Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a composed OpenTTD-style Zurich screenshot scene with station, rail yard, quay, docks, industry props, fields, and compact blocks.

**Architecture:** Extend the existing city modules. `zurichTransport` owns rail yard geometry, `zurichPlacement` owns deterministic detail placement and blocking, and `main.ts` renders details using already imported OpenGFX2 sprites.

**Tech Stack:** TypeScript, Vitest, Vite, Playwright, Canvas 2D, OpenGFX/OpenGFX2 PNG sprites.

---

### Task 1: Add Failing Diorama Tests

**Files:**
- Modify: `tests/city/zurichTransport.test.ts`
- Modify: `tests/city/zurichPlacement.test.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1:** Assert rail tile count is high enough for a visible station/yard setpiece.
- [ ] **Step 2:** Assert placement exposes station, dock, industry, and field detail categories.
- [ ] **Step 3:** Assert runtime has at least 14 rail station tiles and detail counts.
- [ ] **Step 4:** Run focused tests and confirm they fail on the current implementation.

### Task 2: Build Rail Yard Geometry

**Files:**
- Modify: `src/city/zurichTransport.ts`

- [ ] **Step 1:** Add multiple parallel rail platform and yard paths around the main-station zone.
- [ ] **Step 2:** Keep existing road/rail overlap validation clean by letting roads skip non-crossing rail tiles.
- [ ] **Step 3:** Re-run transport tests.

### Task 3: Place Diorama Details

**Files:**
- Modify: `src/city/worldTypes.ts`
- Modify: `src/city/zurichPlacement.ts`

- [ ] **Step 1:** Extend `ZurichDetail.category` for station, dock, quay, field, and yard details.
- [ ] **Step 2:** Add deterministic setpiece detail placement before building placement.
- [ ] **Step 3:** Block setpiece detail tiles so buildings do not overwrite them.
- [ ] **Step 4:** Re-run placement tests.

### Task 4: Render Detail Layer

**Files:**
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1:** Load OpenGFX2 detail sprites from `public/opengfx2/all`.
- [ ] **Step 2:** Add a `detail` drawable and `drawDetail` switch for station, dock, ship, industry, field, park, and depot props.
- [ ] **Step 3:** Expand rail station placement to a visible station complex.
- [ ] **Step 4:** Re-run E2E smoke test and capture a screenshot.

### Task 5: Verify, Document, Push

**Files:**
- Modify: `progress.md`
- Create ignored screenshot: `artifacts/abutown-zurich-river-city-2026-05-14-diorama-v1.png`

- [ ] **Step 1:** Run `npm test`, `npm run build`, and `npm run test:e2e`.
- [ ] **Step 2:** Capture screenshot.
- [ ] **Step 3:** Update `progress.md`.
- [ ] **Step 4:** Commit and push without touching `.gitignore 2`.
