# Grass Footway Walking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let backend walking agents move on grass as well as authored sidewalks, while keeping roads, buildings, and water forbidden.

**Architecture:** Extend base-world mobility seeding to emit additional `Footway` links across adjacent grass tiles and short sidewalk-to-grass connectors. Keep the existing `Walking { link_id, progress }` wire/state shape and add deterministic next-footway selection for simple wandering agents at the end of a footway.

**Tech Stack:** Rust (`sim-core`, `sim-server`, Bevy ECS), existing routing graph, `scripts/cargo-serial.sh` for every Cargo command.

---

## File Structure

- Modify `backend/crates/sim-core/src/mobility/seed.rs`: add base-world grass-footway seeding and tests.
- Modify `backend/crates/sim-core/src/mobility/systems/routing.rs`: make simple `Activity` walkers choose a connected next footway.
- Modify `backend/crates/sim-core/src/mobility/systems/route_execution_tests.rs`: add a focused wandering test.
- Modify `backend/crates/sim-server/src/runtime.rs`: use base-world grass footways when constructing the runtime graph and add runtime assertions.
- Add/update docs under `docs/superpowers/specs/` and `docs/superpowers/plans/`.

## Task 1: Seed Grass Footways From Base World

- [x] Add failing `sim-core` tests in `mobility/seed.rs` proving grass links are emitted, sidewalks remain, and road/building/water tiles are excluded.
- [x] Run focused tests and confirm they fail because grass links are not generated yet.
- [x] Add `seeded_walks_from_base_world(&BaseWorldBundle) -> Vec<SeededWalk>`.
- [x] Use deterministic ids: existing sidewalks keep `link:walk:corridor:N`; grass links use `link:walk:grass:X:Y:dir`; sidewalk connectors use `link:walk:connector:corridor:N:end:M`.
- [x] Run focused tests and confirm they pass.

## Task 2: Use Grass Footways In Base-World Runtime

- [x] Add failing runtime assertions that Abutopia has grass footway edges and still has sidewalk footway edges.
- [x] Replace base-world runtime graph setup from `seeded_walks_from_network(...)` to `seeded_walks_from_base_world(...)`.
- [x] Keep network-only seed paths unchanged because they do not have terrain.
- [x] Run focused `sim-server` runtime tests and confirm they pass.

## Task 3: Deterministic Wander Across Footways

- [x] Add a failing system test for a completed `Activity` walker choosing a connected next `Footway`.
- [x] Implement deterministic next-footway selection in `route_assignment_system`.
- [x] Mark the agent dirty when the `link_id` changes.
- [x] Run focused mobility system tests and confirm they pass.

## Task 4: Verification

- [x] Run `cargo fmt` through `scripts/cargo-serial.sh`.
- [x] Run focused `sim-core` and `sim-server` tests.
- [x] Build `e2e_server` and restart the local backend/frontend if needed.
- [x] Browser-smoke `http://127.0.0.1:5173/` because the feature changes backend-to-rendered mobility behavior.
