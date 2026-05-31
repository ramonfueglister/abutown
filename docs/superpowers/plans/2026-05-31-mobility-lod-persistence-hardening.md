# Mobility LOD Persistence Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent LOD unsubscribe/cooldown from removing Abutopia's 300 concrete base-world agents from health-gated mobility persistence.

**Architecture:** Add a server-owned pinned-active chunk resource. The core LOD reclassifier treats pinned chunks as active simulation interest without changing browser subscriber counts. Runtime startup fills the pin set from base-world mobility paths.

**Tech Stack:** Rust 2024, Bevy ECS, Axum runtime tests, existing `scripts/cargo-serial.sh` verification.

---

## Task 1: RED Runtime Contract

**Files:**
- Modify: `backend/crates/sim-server/src/runtime/tests.rs`

- [x] **Step 1: Add a failing runtime test**

Add a test that loads Abutopia, subscribes to the first walking agent chunk,
ticks until active, unsubscribes, ticks past the LOD cooldown, and asserts
`mobility_persist_snapshot().agents.len() == expected_base_world_agent_count`.

- [x] **Step 2: Run RED**

Run:

```bash
CARGO_TARGET_DIR=/tmp/abutown-mobility-lod-persistence-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server runtime_keeps_base_world_agents_concrete_after_viewport_unsubscribe
```

Expected: FAIL on current main because the agents demote into `flow_cells`.

## Task 2: GREEN Active Pins

**Files:**
- Modify: `backend/crates/sim-core/src/world/resources.rs`
- Modify: `backend/crates/sim-core/src/world/plugin.rs`
- Modify: `backend/crates/sim-core/src/world/systems.rs`
- Modify: `backend/crates/sim-server/src/runtime/mod.rs`
- Modify: `backend/crates/sim-server/src/runtime/tests.rs`

- [x] **Step 1: Add `PinnedActiveChunks`**

Create a core resource storing `HashSet<ChunkCoord>`, installed by
`CorePlugin`.

- [x] **Step 2: Teach the LOD classifier about pins**

Read `PinnedActiveChunks` in `reclassify_chunk_lod_system`. If a chunk is
pinned, classify it as at least `Active` while leaving `ChunkSubscriberCount`
unchanged.

- [x] **Step 3: Pin base-world mobility path chunks at runtime startup**

After applying the mobility snapshot in runtime startup/hydration, compute the
chunks touched by referenced pedestrian corridors and car arterial paths and
insert them into `PinnedActiveChunks`.

- [x] **Step 4: Run GREEN**

Run the RED command again and then:

```bash
CARGO_TARGET_DIR=/tmp/abutown-mobility-lod-persistence-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core world::systems
CARGO_TARGET_DIR=/tmp/abutown-mobility-lod-persistence-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server runtime_keeps_base_world_agents_concrete_after_viewport_unsubscribe persist_snapshots_once_rejects_mobility_snapshots_below_base_world_agents
```

Expected: PASS. The integrity guard remains active for invalid snapshots.

## Task 3: Verification And Finish

**Files:**
- Modify: `docs/superpowers/specs/2026-05-31-mobility-lod-persistence-hardening-design.md`
- Modify: `docs/superpowers/plans/2026-05-31-mobility-lod-persistence-hardening.md`

- [x] **Step 1: Run gates**

```bash
CARGO_TARGET_DIR=/tmp/abutown-mobility-lod-persistence-target scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
CARGO_TARGET_DIR=/tmp/abutown-mobility-lod-persistence-target scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/abutown-mobility-lod-persistence-target scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml --workspace
npm run typecheck
npm run test
npm run build
```

- [x] **Step 2: Commit, PR, merge, push, cleanup**

Merge only after CI is green, then delete the branch/worktree.
