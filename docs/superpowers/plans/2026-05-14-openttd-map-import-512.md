# OpenTTD Map Import 512 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Import a real OpenTTD scenario into the existing app as a flat `512 x 512` world without changing renderer mechanics or OpenGFX asset usage.

**Architecture:** A Node import library decodes OpenTTD savegame chunks, normalizes source tiles into compact world data, and writes a generated TypeScript module. A small runtime adapter converts generated compact data into the existing `ZurichWorld`, transport, and placement shapes.

**Tech Stack:** Node.js ESM scripts, Vitest, TypeScript, OpenTTD BaNaNaS content protocol, existing Canvas/OpenGFX renderer.

---

### Task 1: Import Normalizer

**Files:**
- Create: `scripts/openttdMapImportLib.mjs`
- Create: `tests/scripts/openttdMapImportLib.test.mjs`

- [x] Write tests for OpenTTD tile type normalization, `512 x 512` target sizing, road masks, bridge classification, house/tree/detail extraction, and void-to-grass handling.
- [x] Run `npm test -- tests/scripts/openttdMapImportLib.test.mjs` and verify the tests fail because the library does not exist.
- [x] Implement `normalizeOpenTtdMap`, `decodeOpenTtdSavegame`, `downloadBananasContent`, and `generateTypeScriptModule`.
- [x] Run the focused test and verify it passes.

### Task 2: Generated Runtime Adapter

**Files:**
- Create: `src/city/openTtdImportedWorld.ts`
- Generate: `src/city/openTtdHamburg.generated.ts`
- Modify: `src/main.ts`

- [x] Add an adapter that inflates generated terrain RLE and tuple arrays into the existing world, transport, and placement data structures.
- [x] Switch app boot data from procedural Zurich builders to the generated imported world data.
- [x] Keep camera, renderer, vehicle, building, OpenGFX, and interaction code behavior intact.

### Task 3: Verify Real Import

**Files:**
- Source artifact: OpenTTD BaNaNaS scenario `Hamburg 1.0.5`, content id `11910279`
- Generated: `src/city/openTtdHamburg.generated.ts`

- [x] Run the importer for `512 x 512`.
- [x] Run `npm test`.
- [x] Run `npm run build`.
- [x] Run browser/e2e smoke verification and inspect screenshot for black tiles, water, roads, buildings, trees, and bridges/details.
