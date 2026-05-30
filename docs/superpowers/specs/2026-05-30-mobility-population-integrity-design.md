# Mobility Population Integrity Design

**Date:** 2026-05-30
**Status:** Approved direction from user
**Branch:** `codex/mobility-population-integrity`

## Problem

Abutopia is the current canonical world and its design spec requires one backend-authored pedestrian walking through the world. The backend can currently report healthy mobility persistence while the concrete mobility snapshot contains zero agents. That is not roadmap-correct: it hides a broken server-authoritative population behind a successful persistence liveness check.

## Goal

Make the authored base-world concrete population a runtime contract:

- Abutopia's expected concrete agents come from `data/worlds/abutopia/layers/spawns.json`.
- `/health` is not OK when the published mobility snapshot has fewer concrete agents than the base world expects.
- Mobility persistence never writes a snapshot that violates the base-world concrete population contract.
- The Supabase smoke fails when runtime or persisted mobility has fewer expected agents.
- Frontend remains backend-authoritative; no fake or fallback agents.

## Non-Goals

- Do not touch economy-system plans, code, or worktrees.
- Do not implement the full 1M-agent roadmap in this slice.
- Do not replace the current `mobility_snapshots` storage model with chunked persistence in this slice.
- Do not change frontend rendering to fabricate missing agents.

## Roadmap Fit

This is a corrective gate before scaling work. The 1M roadmap still points toward chunked persistence and LOD-aware storage, but the current Abutopia slice must first prove that a tiny authored population cannot silently disappear while health and persistence claim success.

## Acceptance

- Unit tests cover expected base-world agent count and population-integrity failures.
- Script tests cover the smoke guard for expected concrete agents.
- `smoke:mobility-persistence` reports `expected_agents` and fails on zero-agent runtime/payload.
- Backend verification runs through the repo's serial cargo wrapper.
