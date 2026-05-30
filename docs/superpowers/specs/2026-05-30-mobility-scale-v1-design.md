# Mobility Scale V1 Design

**Date:** 2026-05-30
**Status:** Approved by user via "lets go"
**Branch:** `codex/mobility-scale-v1`

## Goal

Move Abutopia from the one-agent isolation seed to the next roadmap checkpoint:
**300 real backend-authored walking agents** that are visible, persisted, and
health-gated from the server-authoritative mobility state.

## Requirements

- Agents are authored in the base-world spawn layer and seeded by the Rust
  backend. The frontend must not fabricate fallback/demo agents.
- The Supabase smoke must continue to derive `expected_agents` from
  `data/worlds/abutopia/layers/spawns.json`; after this slice it should report
  `expected_agents: 300` and `agents: 300`.
- The mobility population integrity gate remains active: `/health` is unhealthy
  and persistence refuses writes if the runtime has fewer concrete agents than
  the authored spawn count.
- Economy files and worktrees are out of scope.

## Roadmap Fit

This implements the visible-backend-mobility scale checkpoint from the
million-agent roadmap for pedestrians only. Road vehicles, viewport filtering,
per-chunk persistence, and the 1M-agent target remain later roadmap slices.

## Design

Abutopia keeps its current world geometry and single south-sidewalk pedestrian
corridor. The spawn count changes from `1` to `300` in both the generator and
generated base-world data. The existing seeder already supports
`agents_per_corridor > 1` and distributes initial walking progress evenly along
the corridor, so this slice does not need new movement logic.

Tests that previously asserted one Abutopia agent are updated to assert the new
base-world contract. The e2e render smoke still verifies backend-source
mobility, now with 300 agents.

## Non-Goals

- No fake frontend population.
- No road/building/world regeneration.
- No Economy code changes.
- No LOD/chunked persistence redesign in this slice.
