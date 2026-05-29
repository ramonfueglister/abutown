# City Composition Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Clean up the graphics demo so streets, rails, and buildings respect separate map layers and look less like random overlapping grids.

**Architecture:** Keep the current single-canvas TypeScript runtime, but make generation order explicit: terrain -> rail reservation -> road network -> buildings. Road generation must avoid rail tiles except future explicit crossing tiles; district streets should be sparse arterials and frontage lanes rather than repeated square loops.

**Tech Stack:** Vite, TypeScript, Canvas 2D, OpenGFX2 assets, browser screenshot verification on `http://127.0.0.1:5175/`.

---

### Task 1: Rail Reservation

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`

- [x] **Step 1: Build rail paths before roads**

Move `railPaths` before `roads` and add a `railReserved` set derived from every rail coordinate.

- [x] **Step 2: Make road placement respect rail reservation**

Update `addRoadPoint` and `buildRoadNetwork.addPath` so normal road tiles are not inserted onto rail-reserved coordinates.

- [x] **Step 3: Verify**

Run `npm run build`; expected exit code `0`.

### Task 2: District Street Cleanup

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`

- [x] **Step 1: Replace square grids**

Replace the current full 3-by-3 district grid generator with sparse district streets: one main axis, one cross axis, and a few deterministic short frontage spurs.

- [x] **Step 2: Keep protected outside roads**

Keep the north/south/east/west edge roads and existing arterial roads protected from pruning.

- [x] **Step 3: Verify**

Run `npm run build`; expected exit code `0`, then reload `http://127.0.0.1:5175/` and inspect screenshot.

### Task 3: Building Frontage Tightening

**Files:**
- Modify: `/Users/ramonfuglister/Desktop/Coding/abutown/src/main.ts`

- [x] **Step 1: Require safer frontage**

Keep buildings off road/rail/water tiles and prefer the visible street side so sprites do not cover the front road.

- [x] **Step 2: Add diagnostic state**

Extend `render_game_to_text` with invariant counts for road/rail overlap and invalid buildings.

- [x] **Step 3: Verify**

Run `npm run build`; expected exit code `0`, reload browser, check console logs and screenshot.
