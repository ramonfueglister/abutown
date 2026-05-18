# Phase 7b — WS Multi-Client Scalability (Per-Chunk Broadcast Channels) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the per-client per-tick filter walk in the WS broadcast path by sharding the broadcast into per-chunk `tokio::broadcast` channels. Each WS handler holds receivers only for its subscribed chunks; tick-loop publishes once per Active|Hot chunk; the global `ServerMessageDto::MobilityDelta` wire variant is deleted in favour of `MobilityChunkDelta` + `MobilityChunkSnapshot`.

**Architecture:** New `AppState.chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDelta>>>`. `MobilityWorld::tick_mobility()` returns `HashMap<ChunkCoord, MobilityChunkDelta>` (uses new `PreviousAgentChunks` / `PreviousVehicleChunks` resources to compute `left_*`). WS handlers maintain `StreamMap<ChunkCoord, BroadcastStream<MobilityChunkDelta>>` for dynamic per-chunk receiver multiplexing. Subscribe creates the channel (if first) and sends one `MobilityChunkSnapshot`; unsubscribe drops the receiver and reaps the channel when subscriber count hits 0.

**Tech Stack:** `dashmap` (new dep), `tokio::sync::broadcast`, `tokio_stream::StreamMap` + `tokio_stream::wrappers::BroadcastStream`. Frontend: TypeScript types and per-chunk state merge.

---

## Spec

This plan implements `docs/superpowers/specs/2026-05-18-ws-multi-client-scalability-7b-design.md`. Re-read that spec before starting if any task is unclear.

## File Structure

**New files:**
- `scripts/smoke-7b.mjs` — playwright browser smoke for the new chunk-frame flow

**Modified files (backend):**
- `backend/crates/sim-server/Cargo.toml` — add `dashmap`, `tokio-stream` dependencies
- `backend/crates/protocol/src/lib.rs` — DTO additions + ServerMessageDto variants
- `backend/crates/sim-core/src/mobility/mod.rs` — `MobilityChunkDelta` type, `tick_mobility` signature, `build_chunk_snapshot`
- `backend/crates/sim-core/src/mobility/resources.rs` — `PreviousAgentChunks`, `PreviousVehicleChunks`
- `backend/crates/sim-core/src/mobility/dto.rs` — DTO conversion helpers
- `backend/crates/sim-server/src/app.rs` — `chunk_channels` field, subscribe + stream loop rewrite, tick fan-out
- `backend/crates/sim-server/src/runtime.rs` — delete old filtered/synthetic/global methods
- `backend/crates/sim-server/tests/websocket.rs` — adapt 3-client test to new protocol

**Modified files (frontend):**
- `src/backend/mobilityProtocol.ts` — new DTOs, parsers; delete old `MobilityDeltaDto` parsing
- `src/backend/mobilityState.ts` — `applyMobilityChunkDelta`, `applyMobilityChunkSnapshot`; delete old global `applyMobilityDelta`
- `tests/backend/mobilityProtocol.test.ts` — update parser tests
- `tests/backend/mobilityState.test.ts` — update apply tests

**Modified at the end:**
- `progress.md` — Phase 7b entry

---

## Task 1: Infrastructure additions — dependencies, resources, internal types, protocol DTOs

**Files:**
- Modify: `backend/crates/sim-server/Cargo.toml`
- Modify: `backend/crates/sim-core/src/mobility/resources.rs`
- Modify: `backend/crates/sim-core/src/mobility/mod.rs` (insert resources in `empty()`)
- Modify: `backend/crates/protocol/src/lib.rs`

This task is purely additive — no behavior changes, no signature changes. Sets up the types and resources later tasks consume.

- [ ] **Step 1: Add dependencies**

Edit `backend/crates/sim-server/Cargo.toml`. Add under `[dependencies]`:

```toml
dashmap = "6"
tokio-stream = { version = "0.1", features = ["sync"] }
```

The `sync` feature on `tokio-stream` enables `BroadcastStream`.

- [ ] **Step 2: Verify deps fetch + sim-server still builds**

Run:
```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server 2>&1 | tail -5
```

Expected: `Finished dev profile`. If dashmap version doesn't exist, pick the latest 6.x from crates.io and update the version string.

- [ ] **Step 3: Add `PreviousAgentChunks` + `PreviousVehicleChunks` resources**

In `backend/crates/sim-core/src/mobility/resources.rs`, append:

```rust
use crate::ids::{AgentId, VehicleId};

/// Per-agent record of the chunk that agent was in at the END of the
/// previous tick. Used by `tick_mobility` to compute `left_*` lists in the
/// new per-chunk delta — an agent whose previous chunk differs from its
/// current chunk is "leaving" the previous chunk and "arriving" in the
/// current chunk.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousAgentChunks(pub HashMap<AgentId, ChunkCoord>);

/// Mirror of `PreviousAgentChunks` for vehicles.
#[derive(Resource, Debug, Default, Clone)]
pub struct PreviousVehicleChunks(pub HashMap<VehicleId, ChunkCoord>);
```

- [ ] **Step 4: Insert the new resources in `MobilityWorld::empty()`**

In `backend/crates/sim-core/src/mobility/mod.rs`, find `MobilityWorld::empty()` and add (next to the other LOD resources):

```rust
world.insert_resource(PreviousAgentChunks::default());
world.insert_resource(PreviousVehicleChunks::default());
```

- [ ] **Step 5: Add `MobilityChunkDeltaDto` + `MobilityChunkSnapshotDto` to protocol**

In `backend/crates/protocol/src/lib.rs`, find the existing `MobilityDeltaDto` definition. Below it, add:

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MobilityChunkDeltaDto {
    pub protocol_version: u32,
    pub world_id: WorldId,
    pub tick: u64,
    pub chunk: ChunkCoordDto,
    pub changed_agents: Vec<AgentRecordDto>,
    pub changed_vehicles: Vec<VehicleRecordDto>,
    pub left_agents: Vec<EntityId>,
    pub left_vehicles: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MobilityChunkSnapshotDto {
    pub protocol_version: u32,
    pub world_id: WorldId,
    pub tick: u64,
    pub chunk: ChunkCoordDto,
    pub agents: Vec<AgentRecordDto>,
    pub vehicles: Vec<VehicleRecordDto>,
}
```

Find `ServerMessageDto` enum (it has `Hello`, `TilePulseDelta`, `MobilityDelta`, etc.) and add two new variants WITH `#[serde(rename_all = "snake_case")]` already on the enum:

```rust
MobilityChunkDelta(MobilityChunkDeltaDto),
MobilityChunkSnapshot(MobilityChunkSnapshotDto),
```

Do NOT remove the existing `MobilityDelta(MobilityDeltaDto)` variant — Task 7 deletes it after the new path is wired.

- [ ] **Step 6: Verify protocol builds + add round-trip tests**

Add to the existing tests module in `backend/crates/protocol/src/lib.rs` (alongside other DTO round-trip tests):

```rust
#[test]
fn mobility_chunk_delta_round_trips() {
    let dto = MobilityChunkDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("test".into()),
        tick: 42,
        chunk: ChunkCoordDto { x: 1, y: 2 },
        changed_agents: vec![],
        changed_vehicles: vec![],
        left_agents: vec![EntityId("a1".into())],
        left_vehicles: vec![],
    };
    let json = serde_json::to_string(&dto).unwrap();
    let back: MobilityChunkDeltaDto = serde_json::from_str(&json).unwrap();
    assert_eq!(dto, back);
}

#[test]
fn mobility_chunk_snapshot_round_trips() {
    let dto = MobilityChunkSnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("test".into()),
        tick: 42,
        chunk: ChunkCoordDto { x: 0, y: 0 },
        agents: vec![],
        vehicles: vec![],
    };
    let json = serde_json::to_string(&dto).unwrap();
    let back: MobilityChunkSnapshotDto = serde_json::from_str(&json).unwrap();
    assert_eq!(dto, back);
}

#[test]
fn server_message_chunk_delta_variant_parses() {
    let dto = ServerMessageDto::MobilityChunkDelta(MobilityChunkDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("test".into()),
        tick: 1,
        chunk: ChunkCoordDto { x: 0, y: 0 },
        changed_agents: vec![],
        changed_vehicles: vec![],
        left_agents: vec![],
        left_vehicles: vec![],
    });
    let json = serde_json::to_string(&dto).unwrap();
    let back: ServerMessageDto = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, ServerMessageDto::MobilityChunkDelta(_)));
}
```

- [ ] **Step 7: Run protocol tests**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol 2>&1 | tail -5
```

Expected: all tests pass (existing 18 + 3 new = 21).

- [ ] **Step 8: Workspace check**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo build --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | tail -3
```

Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-server/Cargo.toml backend/Cargo.lock backend/crates/sim-core/src/mobility/resources.rs backend/crates/sim-core/src/mobility/mod.rs backend/crates/protocol/src/lib.rs
git commit -m "feat(ws): add dashmap dep, Previous*Chunks resources, ChunkDelta/Snapshot DTOs"
```

---

## Task 2: `MobilityChunkDelta` internal type + `build_chunk_snapshot` helper

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`

- [ ] **Step 1: Add the internal `MobilityChunkDelta` struct**

In `backend/crates/sim-core/src/mobility/mod.rs`, near the existing `MobilityDelta` struct definition, add:

```rust
/// The new per-chunk delta produced by `tick_mobility`. Mirrors
/// `MobilityChunkDeltaDto` shape but uses sim-core record types directly.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilityChunkDelta {
    pub chunk: crate::ids::ChunkCoord,
    pub changed_agents: Vec<AgentRecord>,
    pub changed_vehicles: Vec<VehicleRecord>,
    pub left_agents: Vec<crate::ids::AgentId>,
    pub left_vehicles: Vec<crate::ids::VehicleId>,
}

/// What `build_chunk_snapshot` returns: the current entities inside a chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct MobilityChunkSnapshot {
    pub chunk: crate::ids::ChunkCoord,
    pub agents: Vec<AgentRecord>,
    pub vehicles: Vec<VehicleRecord>,
}
```

- [ ] **Step 2: Write the failing test for `build_chunk_snapshot`**

In the existing test module of `backend/crates/sim-core/src/mobility/mod.rs`, append:

```rust
#[test]
fn build_chunk_snapshot_returns_only_entities_in_that_chunk() {
    use crate::ids::{AgentId, ChunkCoord, LinkId};

    let mut world = MobilityWorld::empty();

    // Two distinct chunks at chunk_size=32: chunk (0,0) covers world x,y in [0,32);
    // chunk (1,0) covers x in [32,64), y in [0,32).
    world.set_link_polyline(LinkId("l:a".into()), vec![(10.0, 10.0), (20.0, 10.0)]);
    world.set_link_polyline(LinkId("l:b".into()), vec![(40.0, 10.0), (50.0, 10.0)]);

    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("agent-a".into()),
        AgentMobilityState::Walking { link_id: LinkId("l:a".into()), progress: 0.0 },
        vec![PlanStage::Activity { activity_id: "act".into() }],
        0.0,
    ));
    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("agent-b".into()),
        AgentMobilityState::Walking { link_id: LinkId("l:b".into()), progress: 0.0 },
        vec![PlanStage::Activity { activity_id: "act".into() }],
        0.0,
    ));

    // Compute world positions by ticking once (compute_world_coord_system runs).
    world.tick_mobility();

    let snapshot = world.build_chunk_snapshot(ChunkCoord { x: 0, y: 0 });
    let agent_ids: Vec<String> = snapshot.agents.iter().map(|a| a.id.0.clone()).collect();
    assert_eq!(agent_ids, vec!["agent-a"], "snapshot returns only chunk(0,0) agents");
    assert!(snapshot.vehicles.is_empty());
    assert_eq!(snapshot.chunk, ChunkCoord { x: 0, y: 0 });
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core build_chunk_snapshot_returns_only_entities_in_that_chunk 2>&1 | tail -10
```

Expected: FAIL with `no method named 'build_chunk_snapshot' found`.

- [ ] **Step 4: Implement `build_chunk_snapshot`**

In `backend/crates/sim-core/src/mobility/mod.rs`, in the existing `impl MobilityWorld` block (find the one that defines `agent`, `vehicle`, `agents`, `vehicles` accessors), add:

```rust
/// Collect all agents + vehicles whose current `Position` falls inside
/// `chunk`. The new WS subscribe path sends this as a `MobilityChunkSnapshot`
/// frame so a client gets the current state of newly-subscribed chunks
/// without waiting for the next tick.
pub fn build_chunk_snapshot(
    &self,
    chunk: crate::ids::ChunkCoord,
) -> MobilityChunkSnapshot {
    let agents = self
        .agents()
        .into_iter()
        .filter(|record| {
            self.world_coord_for_agent(&record.id)
                .map(|(x, y)| crate::mobility::chunk_of(x, y, 32) == chunk)
                .unwrap_or(false)
        })
        .collect();
    let vehicles = self
        .vehicles()
        .into_iter()
        .filter(|record| {
            self.world_coord_for_vehicle(&record.id)
                .map(|(x, y)| crate::mobility::chunk_of(x, y, 32) == chunk)
                .unwrap_or(false)
        })
        .collect();
    MobilityChunkSnapshot { chunk, agents, vehicles }
}
```

If `world_coord_for_vehicle` doesn't exist (only `world_coord_for_agent` does), check the existing mod.rs for the equivalent vehicle accessor — likely uses `vehicle(&id).and_then(...)` pattern. If neither exists, query the Position component directly: `self.world.entity(*self.by_vehicle_id.get(&record.id).unwrap()).get::<Position>().map(|p| (p.x, p.y))`.

- [ ] **Step 5: Run test to verify it passes**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core build_chunk_snapshot_returns_only_entities_in_that_chunk 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/mobility/mod.rs
git commit -m "feat(mobility): MobilityChunkDelta/Snapshot types + build_chunk_snapshot helper"
```

---

## Task 3: `tick_mobility` returns per-chunk map + `Previous*Chunks` tracking

This is the centrepiece. `tick_mobility` changes return type, and all callers must be updated in the same commit (build would break otherwise). The sim-server side reconstructs the old `MobilityDelta` from the new map temporarily — Task 7 cleans that up.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility/mod.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-core/src/mobility/systems.rs` (only if `track_previous_chunks_system` lives here)

- [ ] **Step 1: Write the failing test for the new tick_mobility shape**

In `backend/crates/sim-core/src/mobility/mod.rs` test module, append:

```rust
#[test]
fn tick_mobility_returns_per_chunk_deltas_with_changed_and_left() {
    use crate::ids::{AgentId, ChunkCoord, LinkId};

    let mut world = MobilityWorld::empty();
    // A walkable polyline that crosses chunk boundary: chunk_size 32, so
    // x=30 → chunk 0, x=33 → chunk 1.
    world.set_link_polyline(LinkId("l".into()), vec![(30.0, 10.0), (40.0, 10.0)]);

    // Walk speed > 0 so progress advances and the agent crosses into chunk 1.
    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("walker".into()),
        AgentMobilityState::Walking { link_id: LinkId("l".into()), progress: 0.0 },
        vec![PlanStage::Activity { activity_id: "act".into() }],
        0.5,
    ));

    // First tick: agent enters world at chunk(0,0).
    let map1 = world.tick_mobility();
    assert!(map1.contains_key(&ChunkCoord { x: 0, y: 0 }));
    let delta1 = &map1[&ChunkCoord { x: 0, y: 0 }];
    assert!(!delta1.changed_agents.is_empty());
    assert!(delta1.left_agents.is_empty(), "first tick: no previous chunk to leave");

    // Tick enough times to cross into chunk(1,0).
    let mut crossed = false;
    for _ in 0..20 {
        let map = world.tick_mobility();
        if let Some(delta) = map.get(&ChunkCoord { x: 0, y: 0 })
            && !delta.left_agents.is_empty()
        {
            assert!(
                delta.left_agents.iter().any(|id| id.0 == "walker"),
                "walker shows up in chunk(0,0).left_agents when it crosses out"
            );
            assert!(
                map.get(&ChunkCoord { x: 1, y: 0 })
                    .map(|d| d.changed_agents.iter().any(|r| r.id.0 == "walker"))
                    .unwrap_or(false),
                "walker shows up in chunk(1,0).changed_agents when it crosses in"
            );
            crossed = true;
            break;
        }
    }
    assert!(crossed, "agent must cross chunk boundary within 20 ticks");
}

#[test]
fn tick_mobility_omits_unchanged_chunks() {
    use crate::ids::{AgentId, ChunkCoord, LinkId};
    let mut world = MobilityWorld::empty();
    world.set_link_polyline(LinkId("l".into()), vec![(10.0, 10.0), (20.0, 10.0)]);

    // walk_speed=0 → no progress change → no dirty agents → empty delta map.
    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("stationary".into()),
        AgentMobilityState::Walking { link_id: LinkId("l".into()), progress: 0.0 },
        vec![PlanStage::Activity { activity_id: "act".into() }],
        0.0,
    ));

    // First tick spawns the agent → it's "changed" because newly created.
    let _ = world.tick_mobility();

    // Second tick: no movement, no plan transitions.
    let map = world.tick_mobility();
    assert!(
        map.get(&ChunkCoord { x: 0, y: 0 })
            .map(|d| d.changed_agents.is_empty() && d.left_agents.is_empty())
            .unwrap_or(true),
        "chunk with no changes should either be absent or have empty changed/left lists"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail (compile error on return type)**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core tick_mobility_returns_per_chunk_deltas 2>&1 | tail -15
```

Expected: FAIL — `tick_mobility` still returns the old `MobilityDelta`, so `.contains_key` and the indexing don't compile.

- [ ] **Step 3: Refactor `tick_mobility` to return per-chunk map**

In `backend/crates/sim-core/src/mobility/mod.rs`, locate the existing `pub fn tick_mobility(&mut self) -> MobilityDelta`. Replace it with:

```rust
pub fn tick_mobility(&mut self) -> std::collections::HashMap<crate::ids::ChunkCoord, MobilityChunkDelta> {
    use std::collections::HashMap;

    // Run the schedule (LOD + Advance + Output + Bookkeeping).
    self.schedule.run(&mut self.world);

    // Sync LOD spawn/despawn into by_agent_id / by_vehicle_id (existing logic).
    self.sync_indexes_after_tick();

    // Collect dirty entities (existing DirtyAgents / DirtyVehicles).
    let dirty_agents: Vec<Entity> = self
        .world
        .resource_mut::<DirtyAgents>()
        .0
        .drain()
        .collect();
    let dirty_vehicles: Vec<Entity> = self
        .world
        .resource_mut::<DirtyVehicles>()
        .0
        .drain()
        .collect();

    // Build (current chunk → changed records) for agents.
    let mut changed_by_chunk_agents: HashMap<
        crate::ids::ChunkCoord,
        Vec<AgentRecord>,
    > = HashMap::new();
    let mut current_agent_chunks: HashMap<crate::ids::AgentId, crate::ids::ChunkCoord> =
        HashMap::new();
    for entity in &dirty_agents {
        if let Some(record) = self.agent_record_from_entity(*entity)
            && let Some((x, y)) = self.world_coord_for_agent(&record.id)
        {
            let chunk = crate::mobility::chunk_of(x, y, 32);
            current_agent_chunks.insert(record.id.clone(), chunk);
            changed_by_chunk_agents
                .entry(chunk)
                .or_default()
                .push(record);
        }
    }

    // Same for vehicles.
    let mut changed_by_chunk_vehicles: HashMap<
        crate::ids::ChunkCoord,
        Vec<VehicleRecord>,
    > = HashMap::new();
    let mut current_vehicle_chunks: HashMap<crate::ids::VehicleId, crate::ids::ChunkCoord> =
        HashMap::new();
    for entity in &dirty_vehicles {
        if let Some(record) = self.vehicle_record_from_entity(*entity)
            && let Some((x, y)) = self.world_coord_for_vehicle(&record.id)
        {
            let chunk = crate::mobility::chunk_of(x, y, 32);
            current_vehicle_chunks.insert(record.id.clone(), chunk);
            changed_by_chunk_vehicles
                .entry(chunk)
                .or_default()
                .push(record);
        }
    }

    // Compute left_* by comparing current chunk vs PreviousAgentChunks.
    let mut left_by_chunk_agents: HashMap<crate::ids::ChunkCoord, Vec<crate::ids::AgentId>> =
        HashMap::new();
    {
        let prev = self.world.resource::<PreviousAgentChunks>();
        for (id, current_chunk) in &current_agent_chunks {
            if let Some(prev_chunk) = prev.0.get(id)
                && prev_chunk != current_chunk
            {
                left_by_chunk_agents
                    .entry(*prev_chunk)
                    .or_default()
                    .push(id.clone());
            }
        }
    }
    let mut left_by_chunk_vehicles: HashMap<
        crate::ids::ChunkCoord,
        Vec<crate::ids::VehicleId>,
    > = HashMap::new();
    {
        let prev = self.world.resource::<PreviousVehicleChunks>();
        for (id, current_chunk) in &current_vehicle_chunks {
            if let Some(prev_chunk) = prev.0.get(id)
                && prev_chunk != current_chunk
            {
                left_by_chunk_vehicles
                    .entry(*prev_chunk)
                    .or_default()
                    .push(id.clone());
            }
        }
    }

    // Update PreviousAgentChunks + PreviousVehicleChunks for next tick.
    {
        let mut prev = self.world.resource_mut::<PreviousAgentChunks>();
        for (id, chunk) in &current_agent_chunks {
            prev.0.insert(id.clone(), *chunk);
        }
    }
    {
        let mut prev = self.world.resource_mut::<PreviousVehicleChunks>();
        for (id, chunk) in &current_vehicle_chunks {
            prev.0.insert(id.clone(), *chunk);
        }
    }

    // Assemble per-chunk delta map: union of all chunks that have either
    // a changed entity or a left entity.
    let mut out: HashMap<crate::ids::ChunkCoord, MobilityChunkDelta> = HashMap::new();
    for (chunk, agents) in changed_by_chunk_agents {
        out.entry(chunk).or_insert_with(|| MobilityChunkDelta {
            chunk,
            changed_agents: Vec::new(),
            changed_vehicles: Vec::new(),
            left_agents: Vec::new(),
            left_vehicles: Vec::new(),
        }).changed_agents = agents;
    }
    for (chunk, vehicles) in changed_by_chunk_vehicles {
        out.entry(chunk).or_insert_with(|| MobilityChunkDelta {
            chunk,
            changed_agents: Vec::new(),
            changed_vehicles: Vec::new(),
            left_agents: Vec::new(),
            left_vehicles: Vec::new(),
        }).changed_vehicles = vehicles;
    }
    for (chunk, ids) in left_by_chunk_agents {
        out.entry(chunk).or_insert_with(|| MobilityChunkDelta {
            chunk,
            changed_agents: Vec::new(),
            changed_vehicles: Vec::new(),
            left_agents: Vec::new(),
            left_vehicles: Vec::new(),
        }).left_agents = ids;
    }
    for (chunk, ids) in left_by_chunk_vehicles {
        out.entry(chunk).or_insert_with(|| MobilityChunkDelta {
            chunk,
            changed_agents: Vec::new(),
            changed_vehicles: Vec::new(),
            left_agents: Vec::new(),
            left_vehicles: Vec::new(),
        }).left_vehicles = ids;
    }

    out
}
```

If the existing function did NOT have `sync_indexes_after_tick` and instead inlined the index sync, preserve that — only change the data-shaping at the end. Read the existing function first to be sure.

- [ ] **Step 4: Update sim-server callers (temporary glue)**

In `backend/crates/sim-server/src/runtime.rs`, find `next_mobility_delta` (returns `MobilityDeltaDto`). It currently does:
```rust
let delta = self.mobility.tick_mobility();
build_mobility_delta_dto(&self.world_id, self.mobility.tick(), &self.mobility, &delta)
```

Change to reconstruct a `MobilityDelta` from the per-chunk map (temporary glue — Task 7 deletes this entire function):

```rust
pub fn next_mobility_delta(&mut self) -> MobilityDeltaDto {
    let per_chunk = self.mobility.tick_mobility();
    // Glue: flatten per-chunk map back into a global delta so the old
    // broadcast path keeps working until Task 7 deletes it.
    let mut changed_agents = Vec::new();
    let mut changed_vehicles = Vec::new();
    for delta in per_chunk.into_values() {
        changed_agents.extend(delta.changed_agents);
        changed_vehicles.extend(delta.changed_vehicles);
    }
    let delta = sim_core::mobility::MobilityDelta {
        changed_agents,
        changed_vehicles,
    };
    build_mobility_delta_dto(&self.world_id, self.mobility.tick(), &self.mobility, &delta)
}
```

The `MobilityDelta` struct (the old one) must still exist in sim-core for this glue to compile. If `tick_mobility` no longer returns it, the struct itself stays as a type used only by `build_mobility_delta_dto`. That's fine — Task 7 removes it.

- [ ] **Step 5: Update `next_mobility_delta_for_test` and any other callers**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
grep -rn "tick_mobility()" backend/crates/ 2>&1 | head -20
```

Adapt each call site to expect `HashMap<ChunkCoord, MobilityChunkDelta>`. The two most common patterns:
- Test code that asserts on `delta.changed_agents.len()` → change to iterate the map values and sum.
- Code paths in app.rs/runtime.rs that broadcast → use the glue from Step 4 OR collect into a flat list.

- [ ] **Step 6: Run the new tests + workspace**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core tick_mobility_returns_per_chunk_deltas tick_mobility_omits_unchanged_chunks build_chunk_snapshot 2>&1 | tail -10
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
```

Expected: new tests pass, workspace remains green.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat(mobility): tick_mobility returns per-chunk delta map with left_* tracking"
```

---

## Task 4: AppState `chunk_channels: Arc<DashMap<…>>` field

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

- [ ] **Step 1: Add the field to AppState**

In `backend/crates/sim-server/src/app.rs`, locate the `pub struct AppState` definition. Add:

```rust
use dashmap::DashMap;

pub struct AppState {
    runtime: Arc<RwLock<SimulationRuntime>>,
    chunk_channels: Arc<DashMap<sim_core::ids::ChunkCoord, broadcast::Sender<abutown_protocol::MobilityChunkDeltaDto>>>,
    snapshot_store: Arc<tokio::sync::Mutex<Box<dyn sim_core::persistence::ChunkSnapshotStore + Send + Sync>>>,
    mobility_snapshot_store: Arc<tokio::sync::Mutex<Box<dyn sim_core::persistence::MobilitySnapshotStore + Send + Sync>>>,
    deltas: broadcast::Sender<ServerMessageDto>, // KEEP — Task 7 deletes
    card_hands: CardHandStore,
    auth: AuthVerifier,
}
```

(Adjust field order/visibility to match existing.)

- [ ] **Step 2: Initialise in all AppState constructors**

Find `AppState::new`, `AppState::new_with_card_hands`, `AppState::new_with_stores`, etc. In each, add:

```rust
chunk_channels: Arc::new(DashMap::new()),
```

next to where the other Arc-wrapped fields are constructed.

- [ ] **Step 3: Add accessor**

In `impl AppState`:

```rust
pub(crate) fn chunk_channels(&self) -> Arc<DashMap<sim_core::ids::ChunkCoord, broadcast::Sender<abutown_protocol::MobilityChunkDeltaDto>>> {
    Arc::clone(&self.chunk_channels)
}
```

- [ ] **Step 4: Verify build**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 5: Workspace test**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -10
```

Expected: still green.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/app.rs
git commit -m "feat(ws): AppState.chunk_channels DashMap for per-chunk broadcast senders"
```

---

## Task 5: Subscribe path — create channel, store receiver, send snapshot

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

Phase 7a's `handle_client_message` already does `apply_subscription_diff` on subscribe/unsubscribe and emits a synthetic `MobilityDelta`. This task wires the NEW path: per added chunk, create channel + store receiver + send `MobilityChunkSnapshot`. Per removed chunk, drop receiver + reap channel if last.

- [ ] **Step 1: Extend ConnectionState with chunk_streams**

In `app.rs`, find `struct ConnectionState`. Change to:

```rust
use tokio_stream::StreamMap;
use tokio_stream::wrappers::BroadcastStream;

struct ConnectionState {
    subscription: std::collections::HashSet<sim_core::ids::ChunkCoord>,
    last_visible_agents: std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: std::collections::HashSet<abutown_protocol::EntityId>,
    chunk_streams: StreamMap<sim_core::ids::ChunkCoord, BroadcastStream<abutown_protocol::MobilityChunkDeltaDto>>,
}
```

Remove `#[derive(Default)]` from `ConnectionState` if present; replace with explicit default constructor:

```rust
impl ConnectionState {
    fn new() -> Self {
        Self {
            subscription: std::collections::HashSet::new(),
            last_visible_agents: std::collections::HashSet::new(),
            last_visible_vehicles: std::collections::HashSet::new(),
            chunk_streams: StreamMap::new(),
        }
    }
}
```

Replace any `ConnectionState::default()` with `ConnectionState::new()`.

- [ ] **Step 2: Helper to convert sim-core types to DTO**

Add a free function in `app.rs` (or use existing one in `dto.rs` if it exists):

```rust
fn chunk_snapshot_to_dto(
    snapshot: &sim_core::mobility::MobilityChunkSnapshot,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> abutown_protocol::MobilityChunkSnapshotDto {
    use sim_core::mobility::build_agent_record_dto;
    use sim_core::mobility::build_vehicle_record_dto;
    abutown_protocol::MobilityChunkSnapshotDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        chunk: abutown_protocol::ChunkCoordDto {
            x: snapshot.chunk.x,
            y: snapshot.chunk.y,
        },
        agents: snapshot.agents.iter().map(build_agent_record_dto).collect(),
        vehicles: snapshot.vehicles.iter().map(build_vehicle_record_dto).collect(),
    }
}
```

If `build_agent_record_dto` / `build_vehicle_record_dto` don't exist, look in `backend/crates/sim-core/src/mobility/dto.rs` for similar helpers — they almost certainly do under different names. Adapt.

- [ ] **Step 3: Refactor `handle_client_message` subscribe path**

Find the subscribe branch in `handle_client_message`. Replace the existing logic with:

```rust
ClientMessageDto::ChunkSubscribe(payload) => {
    let added: Vec<sim_core::ids::ChunkCoord> = payload
        .coords
        .iter()
        .map(sim_core::ids::ChunkCoord::from)
        .filter(|c| connection.subscription.insert(*c))
        .collect();

    // For each newly-added chunk:
    //  1. Increment ChunkSubscribers (runtime write-lock briefly)
    //  2. Create or look up channel in chunk_channels (DashMap)
    //  3. Subscribe to the channel and stash receiver in chunk_streams
    //  4. Build & send a MobilityChunkSnapshot for this chunk
    if !added.is_empty() {
        let chunk_channels = state.chunk_channels();
        let mut runtime = state.runtime().write().await;
        runtime.apply_subscription_diff(&added, std::iter::empty());
        let world_id = runtime.world_id_for_persist().clone();
        let tick = runtime.mobility_tick();
        for coord in &added {
            let sender = chunk_channels
                .entry(*coord)
                .or_insert_with(|| broadcast::channel(8).0)
                .clone();
            let receiver = sender.subscribe();
            connection.chunk_streams.insert(*coord, BroadcastStream::new(receiver));
            let snapshot = runtime.mobility_for_persist().build_chunk_snapshot(*coord);
            let dto = chunk_snapshot_to_dto(&snapshot, &world_id, tick);
            let _ = send_server_message(
                &mut /* socket */ , // see below
                ServerMessageDto::MobilityChunkSnapshot(dto),
            ).await;
        }
    }
    return None; // no synthetic delta anymore — snapshots replace it
}
```

PROBLEM: `handle_client_message` doesn't currently have access to `socket` — it returns an Option that the caller (stream_world_deltas) sends. The new path sends multiple frames per subscribe. Restructure: pass `&mut WebSocket` into `handle_client_message`, OR return `Vec<ServerMessageDto>` for the caller to send.

The simpler refactor: change return type to `Vec<ServerMessageDto>`:

```rust
async fn handle_client_message(
    state: &AppState,
    message: &ClientMessageDto,
    connection: &mut ConnectionState,
) -> Vec<ServerMessageDto> {
    // ... accumulate messages and return them all
}
```

And the caller (in `stream_world_deltas`) loops over the returned Vec and `send_server_message`s each.

Update the subscribe path inside this Vec-returning function:

```rust
let mut out: Vec<ServerMessageDto> = Vec::new();
// ... in the subscribe branch, push ServerMessageDto::MobilityChunkSnapshot(dto) instead of sending
```

For unsubscribe:

```rust
ClientMessageDto::ChunkUnsubscribe(payload) => {
    let removed: Vec<sim_core::ids::ChunkCoord> = payload
        .coords
        .iter()
        .map(sim_core::ids::ChunkCoord::from)
        .filter(|c| connection.subscription.remove(c))
        .collect();
    if !removed.is_empty() {
        let chunk_channels = state.chunk_channels();
        let mut runtime = state.runtime().write().await;
        runtime.apply_subscription_diff(std::iter::empty(), &removed);
        for coord in &removed {
            connection.chunk_streams.remove(coord);
            // Aggressive reap: if no other client is subscribed to this chunk,
            // drop the channel so the tick fan-out has one less map entry.
            if runtime.chunk_subscriber_count(*coord) == 0 {
                chunk_channels.remove(coord);
            }
        }
    }
}
```

You'll need to expose `chunk_subscriber_count(coord) -> u8` on `SimulationRuntime` (reads `ChunkSubscribers` resource). Simple addition.

- [ ] **Step 4: Write a TDD test for the subscribe-emits-snapshot behavior**

Add to `backend/crates/sim-server/tests/websocket.rs`:

```rust
#[tokio::test]
async fn chunk_subscribe_emits_chunk_snapshot_frame() {
    let runtime = SimulationRuntime::new();
    let app = build_app_with_runtime(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let url = format!("ws://{}/ws", addr);
    let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    // Consume hello.
    let _ = client.next().await.unwrap().unwrap();

    send_chunk_subscribe(&mut client, &[ChunkCoordDto { x: 4, y: 4 }]).await;

    // Expect the next non-pulse message to be a MobilityChunkSnapshot for (4,4).
    let mut got_snapshot = false;
    for _ in 0..10 {
        let msg = client.next().await.unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg
            && let Ok(ServerMessageDto::MobilityChunkSnapshot(snap)) = serde_json::from_str(&text)
        {
            assert_eq!(snap.chunk.x, 4);
            assert_eq!(snap.chunk.y, 4);
            got_snapshot = true;
            break;
        }
    }
    assert!(got_snapshot, "subscribe should emit a MobilityChunkSnapshot for the new chunk");
}
```

- [ ] **Step 5: Run the test (should pass)**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server --test websocket chunk_subscribe_emits_chunk_snapshot_frame 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 6: Workspace check**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -10
```

Expected: green.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat(ws): subscribe creates per-chunk channel + emits MobilityChunkSnapshot"
```

---

## Task 6: Per-handler StreamMap select! loop forwards per-chunk deltas

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

- [ ] **Step 1: Add the chunk_streams arm to stream_world_deltas select!**

Find `stream_world_deltas`. Inside the main `loop { tokio::select! { … } }`, add a new arm BEFORE the existing `deltas.recv()` arm (so chunk-specific path has priority):

```rust
Some((chunk, item)) = connection.chunk_streams.next(), if !connection.chunk_streams.is_empty() => {
    use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
    match item {
        Ok(delta) => {
            if send_server_message(
                &mut socket,
                ServerMessageDto::MobilityChunkDelta(delta),
            ).await.is_err() {
                break;
            }
        }
        Err(BroadcastStreamRecvError::Lagged(_)) => {
            // Recovery: re-send a fresh snapshot for this chunk.
            let snap = {
                let runtime = state.runtime().read().await;
                let snapshot = runtime.mobility_for_persist().build_chunk_snapshot(chunk);
                let world_id = runtime.world_id_for_persist().clone();
                let tick = runtime.mobility_tick();
                chunk_snapshot_to_dto(&snapshot, &world_id, tick)
            };
            if send_server_message(
                &mut socket,
                ServerMessageDto::MobilityChunkSnapshot(snap),
            ).await.is_err() {
                break;
            }
        }
    }
}
```

You'll need `use futures::StreamExt;` (or `tokio_stream::StreamExt;`) for `.next()` on `StreamMap`. Check which is the right import — tokio-stream version.

- [ ] **Step 2: Write integration test**

In `tests/websocket.rs`, append:

```rust
#[tokio::test]
async fn subscribed_chunk_receives_mobility_chunk_delta_each_tick() {
    let runtime = SimulationRuntime::new();
    let app = build_app_with_runtime(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let url = format!("ws://{}/ws", addr);
    let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let _ = client.next().await.unwrap().unwrap(); // hello

    // Subscribe to a chunk that the seeded SimulationRuntime has agents in.
    // Reuse the helper from the existing 3-client test.
    send_chunk_subscribe(&mut client, &[ChunkCoordDto { x: 4, y: 4 }]).await;

    let mut snapshot_seen = false;
    let mut delta_seen = false;
    for _ in 0..30 {
        let msg = client.next().await.unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            if let Ok(ServerMessageDto::MobilityChunkSnapshot(_)) = serde_json::from_str(&text) {
                snapshot_seen = true;
            }
            if let Ok(ServerMessageDto::MobilityChunkDelta(delta)) = serde_json::from_str(&text) {
                assert_eq!(delta.chunk.x, 4);
                assert_eq!(delta.chunk.y, 4);
                delta_seen = true;
                break;
            }
        }
    }
    assert!(snapshot_seen, "snapshot should arrive on subscribe");
    assert!(delta_seen, "per-tick delta should arrive within 30 messages");
}
```

(This test passes ONLY after Task 8's tick fan-out. Mark it as `#[ignore]` for this task and unignore in Task 8.)

Actually flip — skip the test for now (don't write it yet) and add it in Task 8 once fan-out is wired.

- [ ] **Step 3: Verify build**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server 2>&1 | tail -5
```

Expected: clean (the chunk_streams arm compiles even with no incoming traffic).

- [ ] **Step 4: Workspace test**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -10
```

Expected: all green (no behavior tests added yet, just plumbing).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/app.rs
git commit -m "feat(ws): stream_world_deltas reads per-chunk receivers via StreamMap"
```

---

## Task 7: Tick-loop fan-out into chunk_channels + integration test

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [ ] **Step 1: Add `tick_and_fan_out` to AppState (or a free function)**

In `app.rs`, add:

```rust
async fn tick_and_fan_out(state: &AppState) {
    // Phase 1: tick the world under brief write-lock, return per-chunk deltas.
    let (per_chunk, world_id, tick) = {
        let mut runtime = state.runtime().write().await;
        let per_chunk = runtime.tick_world_mobility();
        let world_id = runtime.world_id_for_persist().clone();
        let tick = runtime.mobility_tick();
        (per_chunk, world_id, tick)
    };

    // Phase 2: publish per-chunk deltas into broadcast channels. No runtime lock.
    let chunk_channels = state.chunk_channels();
    for (chunk, delta) in per_chunk {
        let Some(sender) = chunk_channels.get(&chunk).map(|e| e.clone()) else {
            continue;
        };
        let dto = chunk_delta_to_dto(&delta, &world_id, tick);
        let _ = sender.send(dto); // best-effort; ignore if no receivers
    }
}
```

Where `chunk_delta_to_dto` mirrors `chunk_snapshot_to_dto` (Task 5):

```rust
fn chunk_delta_to_dto(
    delta: &sim_core::mobility::MobilityChunkDelta,
    world_id: &abutown_protocol::WorldId,
    tick: u64,
) -> abutown_protocol::MobilityChunkDeltaDto {
    abutown_protocol::MobilityChunkDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        chunk: abutown_protocol::ChunkCoordDto { x: delta.chunk.x, y: delta.chunk.y },
        changed_agents: delta.changed_agents.iter().map(build_agent_record_dto).collect(),
        changed_vehicles: delta.changed_vehicles.iter().map(build_vehicle_record_dto).collect(),
        left_agents: delta.left_agents.iter().map(|id| abutown_protocol::EntityId(id.0.clone())).collect(),
        left_vehicles: delta.left_vehicles.iter().map(|id| abutown_protocol::EntityId(id.0.clone())).collect(),
    }
}
```

`tick_world_mobility` is a new method on `SimulationRuntime`:

```rust
pub fn tick_world_mobility(&mut self) -> std::collections::HashMap<
    sim_core::ids::ChunkCoord,
    sim_core::mobility::MobilityChunkDelta,
> {
    self.mobility.tick_mobility()
}
```

- [ ] **Step 2: Rewrite `spawn_delta_loop` to use `tick_and_fan_out`**

Find `spawn_delta_loop`. Currently it builds `ServerMessageDto::MobilityDelta` and broadcasts to the global `deltas` channel. Add a parallel new behavior:

```rust
fn spawn_delta_loop(&self, tick_interval: Duration) {
    let state = self.clone();
    let deltas = self.deltas.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            // New per-chunk fan-out:
            tick_and_fan_out(&state).await;
            // KEEP old broadcast for one task longer (Task 8 deletes it):
            let messages = {
                let runtime = state.runtime();
                let mut runtime = runtime.write().await;
                runtime.next_server_messages_legacy()
            };
            for message in messages {
                let _ = deltas.send(message);
            }
        }
    });
}
```

Note: `tick_world_mobility` calls `tick_mobility` once. If `next_server_messages_legacy` ALSO calls tick_mobility (it does — that's the bug), we'd tick twice per interval. Refactor `next_server_messages_legacy` to NOT call tick_mobility (it just builds DTOs from current state). For the legacy path during transition, just call `next_pulse()` (returns `TilePulseDelta`).

Replace `next_server_messages` with `next_pulse_only` for the legacy bit:

```rust
let messages = {
    let runtime = state.runtime().read().await;
    vec![runtime.next_pulse()]
};
```

Where `next_pulse` is on SimulationRuntime, returns one `TilePulseDelta` per call. If it requires `&mut self` (it does — increments pulse counter), use write lock.

- [ ] **Step 3: Write the integration test (was deferred from Task 6)**

In `tests/websocket.rs`, append the test from Task 6 Step 2 (the `subscribed_chunk_receives_mobility_chunk_delta_each_tick` test).

- [ ] **Step 4: Run new test**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server --test websocket subscribed_chunk_receives_mobility_chunk_delta_each_tick 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 5: Workspace test**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -10
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat(ws): tick_and_fan_out publishes per-chunk deltas to chunk_channels"
```

---

## Task 8: Delete old paths (MobilityDelta variant, filtered/synthetic methods, global broadcast)

This is the cleanup task that removes the "old crap" per the spec's state-of-the-art mandate.

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-core/src/mobility/dto.rs` (if `build_filtered_mobility_delta_dto` lives there)
- Modify: `backend/crates/sim-core/src/mobility/mod.rs` (delete unused `MobilityDelta` struct)
- Modify: `backend/crates/sim-server/tests/websocket.rs` (remove tests for deleted variants)

- [ ] **Step 1: Delete from runtime.rs**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
grep -n "filtered_mobility_delta_from_dto\|synthetic_mobility_delta_for_subscription\|next_mobility_delta\|next_server_messages" backend/crates/sim-server/src/runtime.rs
```

Delete each method:
- `filtered_mobility_delta_from_dto(&self, …) -> MobilityDeltaDto`
- `synthetic_mobility_delta_for_subscription(&self, …) -> MobilityDeltaDto`
- `next_mobility_delta(&mut self) -> MobilityDeltaDto`
- `next_server_messages(&mut self) -> Vec<ServerMessageDto>` (or whatever legacy variant exists from Task 7)

Keep: `next_pulse(&mut self) -> ServerMessageDto` (the tile-pulse path is independent).

- [ ] **Step 2: Delete from app.rs**

- Remove `deltas: broadcast::Sender<ServerMessageDto>` field from `AppState`.
- Remove `subscribe_deltas`, the `deltas.recv()` arm in `stream_world_deltas`, the legacy half of `spawn_delta_loop` (only `tick_and_fan_out` remains).
- Remove `broadcast::channel(DELTA_BROADCAST_CAPACITY)` and the constant.

`stream_world_deltas` should now have only:
- hello on connect
- inbound (handle_client_message → loop over returned Vec<ServerMessageDto> and send)
- chunk_streams arm
- cleanup on close

- [ ] **Step 3: Delete from protocol**

In `backend/crates/protocol/src/lib.rs`:
- Delete the `MobilityDelta(MobilityDeltaDto)` variant from `ServerMessageDto`.
- Delete the `MobilityDeltaDto` struct itself.

- [ ] **Step 4: Delete from sim-core**

In `backend/crates/sim-core/src/mobility/mod.rs`, the `MobilityDelta` struct (the OLD one with global `changed_agents` / `changed_vehicles`) is now unused.

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
grep -rn "MobilityDelta\b" backend/crates/ 2>&1 | grep -v "MobilityChunkDelta\|MobilityDeltaDto"
```

Delete `pub struct MobilityDelta { ... }` from sim-core. Any remaining `build_mobility_delta_dto` / `build_filtered_mobility_delta_dto` helpers in `dto.rs` that took `&MobilityDelta` go too.

- [ ] **Step 5: Adapt tests**

Some existing tests may use the deleted methods. Grep:

```bash
grep -rn "MobilityDelta\|filtered_mobility_delta_from_dto\|synthetic_mobility_delta_for_subscription\|next_mobility_delta\|next_server_messages" backend/crates/sim-server/tests/ backend/crates/sim-core/tests/ 2>&1 | head -20
```

For each match, either delete the test (if it tested behavior that no longer exists) or rewrite it for the new per-chunk shape. Specifically the Phase 7a `two_clients_with_different_subscriptions_see_different_entities` and `three_clients_with_disjoint_subscriptions_see_only_their_chunks` need adaptation — they currently expect `MobilityDelta` frames. Change them to expect `MobilityChunkSnapshot` then `MobilityChunkDelta`.

- [ ] **Step 6: Workspace check**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo build --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | tail -5
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: clean build, all tests green, clippy clean.

- [ ] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs backend/crates/protocol/src/lib.rs backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-core/src/mobility/dto.rs backend/crates/sim-server/tests/
git commit -m "refactor(ws): delete MobilityDelta variant + filtered/synthetic/global broadcast path"
```

---

## Task 9: Frontend protocol parsing + state apply functions

**Files:**
- Modify: `src/backend/mobilityProtocol.ts`
- Modify: `src/backend/mobilityState.ts`
- Modify: `tests/backend/mobilityProtocol.test.ts`
- Modify: `tests/backend/mobilityState.test.ts`

- [ ] **Step 1: Add new DTO types + parsers**

In `src/backend/mobilityProtocol.ts`, find the existing `ServerMessageDto` union. Add:

```ts
export type MobilityChunkDeltaDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  chunk: ChunkCoordDto;
  changed_agents: AgentRecordDto[];
  changed_vehicles: VehicleRecordDto[];
  left_agents: EntityId[];
  left_vehicles: EntityId[];
};

export type MobilityChunkSnapshotDto = {
  protocol_version: number;
  world_id: string;
  tick: number;
  chunk: ChunkCoordDto;
  agents: AgentRecordDto[];
  vehicles: VehicleRecordDto[];
};
```

Extend `ServerMessageDto`:

```ts
export type ServerMessageDto =
  | { type: 'hello'; ... }
  | { type: 'tile_pulse'; ... }
  | { type: 'mobility_chunk_delta'; ...inline MobilityChunkDeltaDto fields }
  | { type: 'mobility_chunk_snapshot'; ...inline MobilityChunkSnapshotDto fields };
```

(Match the serde `rename_all = "snake_case"` discriminator format from the backend.)

Add parsers `isMobilityChunkDeltaDto`, `isMobilityChunkSnapshotDto`, and extend `parseServerMessage` to recognise the new variants.

DELETE the old `MobilityDeltaDto` type and its parser branch.

- [ ] **Step 2: Update protocol tests**

In `tests/backend/mobilityProtocol.test.ts`, delete tests for the old `MobilityDelta`. Add:

```ts
it('parses a MobilityChunkDelta server message', () => {
  const raw = {
    type: 'mobility_chunk_delta',
    protocol_version: 1,
    world_id: 'abutown-main',
    tick: 5,
    chunk: { x: 4, y: 4 },
    changed_agents: [],
    changed_vehicles: [],
    left_agents: [],
    left_vehicles: [],
  };
  const msg = parseServerMessage(raw);
  expect(msg?.type).toBe('mobility_chunk_delta');
});

it('parses a MobilityChunkSnapshot server message', () => {
  const raw = {
    type: 'mobility_chunk_snapshot',
    protocol_version: 1,
    world_id: 'abutown-main',
    tick: 5,
    chunk: { x: 4, y: 4 },
    agents: [],
    vehicles: [],
  };
  const msg = parseServerMessage(raw);
  expect(msg?.type).toBe('mobility_chunk_snapshot');
});
```

- [ ] **Step 3: Add `applyMobilityChunkSnapshot` and `applyMobilityChunkDelta`**

In `src/backend/mobilityState.ts`:

```ts
export function applyMobilityChunkSnapshot(
  state: MobilityOverlayState,
  msg: MobilityChunkSnapshotDto,
  now: number,
): MobilityOverlayState {
  // Replace all entities that BELONG TO this chunk with the snapshot's contents.
  // Other chunks' entities are untouched.
  const chunkKey = `${msg.chunk.x},${msg.chunk.y}`;
  const nextAgents = new Map(state.agents);
  // Remove existing entries in this chunk.
  for (const [id, entry] of nextAgents) {
    if (chunkKeyOf(entry.current) === chunkKey) nextAgents.delete(id);
  }
  // Insert snapshot's entries.
  for (const dto of msg.agents) {
    nextAgents.set(dto.id, makeMobilityAgentEntry(dto, now));
  }
  const nextVehicles = /* same pattern */;
  return { ...state, agents: nextAgents, vehicles: nextVehicles, lastUpdatedAt: now };
}

export function applyMobilityChunkDelta(
  state: MobilityOverlayState,
  msg: MobilityChunkDeltaDto,
  now: number,
): MobilityOverlayState {
  const nextAgents = new Map(state.agents);
  for (const dto of msg.changed_agents) {
    nextAgents.set(dto.id, makeMobilityAgentEntry(dto, now));
  }
  for (const id of msg.left_agents) {
    nextAgents.delete(id);
  }
  const nextVehicles = /* same pattern */;
  return { ...state, agents: nextAgents, vehicles: nextVehicles, lastUpdatedAt: now };
}
```

(Adapt to the exact existing state shape — read mobilityState.ts to see how it stores agent entries with prev/current interpolation. The interpolation-buffer structure has to be preserved for the existing render-time smoothing to keep working.)

Update `applyServerMessage` to route the new variants:

```ts
case 'mobility_chunk_snapshot':
  return applyMobilityChunkSnapshot(state, msg, now);
case 'mobility_chunk_delta':
  return applyMobilityChunkDelta(state, msg, now);
```

DELETE the `mobility_delta` case and the `applyMobilityDelta` function.

- [ ] **Step 4: Update state tests**

In `tests/backend/mobilityState.test.ts`, delete `applyMobilityDelta` tests. Add tests for the two new functions:

```ts
it('applyMobilityChunkSnapshot replaces entities for that chunk only', () => {
  // ... seed state with agents in chunks A and B
  // ... apply snapshot for A with different agents
  // ... assert A's agents replaced, B's untouched
});

it('applyMobilityChunkDelta removes left_agents and merges changed_agents', () => {
  // ...
});
```

- [ ] **Step 5: Run frontend tests**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
npx vitest run tests/backend/mobilityProtocol.test.ts tests/backend/mobilityState.test.ts 2>&1 | tail -8
npx tsc --noEmit 2>&1 | head -5
```

Expected: all green, tsc clean.

- [ ] **Step 6: Commit**

```bash
git add src/backend/mobilityProtocol.ts src/backend/mobilityState.ts tests/backend/mobilityProtocol.test.ts tests/backend/mobilityState.test.ts
git commit -m "feat(ws): frontend learns MobilityChunkDelta/Snapshot, drops MobilityDelta"
```

---

## Task 10: Browser smoke (CLAUDE.md mandate for frontend wire changes)

**Files:**
- Create: `scripts/smoke-7b.mjs`

- [ ] **Step 1: Write the smoke script**

Copy `scripts/smoke-7a.mjs` to `scripts/smoke-7b.mjs`, then adapt the message-classification logic:

```js
function summarise(receivedFrames) {
  const out = {
    mobility_chunk_snapshot: { count: 0, by_chunk: {} },
    mobility_chunk_delta: { count: 0, by_chunk: {} },
    tile_pulse: 0,
    hello: 0,
    other: { count: 0, samples: [] },
  };
  for (const f of receivedFrames) {
    let parsed;
    try { parsed = JSON.parse(f); } catch { continue; }
    switch (parsed.type) {
      case 'mobility_chunk_snapshot': {
        out.mobility_chunk_snapshot.count += 1;
        const key = `${parsed.chunk.x},${parsed.chunk.y}`;
        out.mobility_chunk_snapshot.by_chunk[key] = (out.mobility_chunk_snapshot.by_chunk[key] ?? 0) + 1;
        break;
      }
      case 'mobility_chunk_delta': {
        out.mobility_chunk_delta.count += 1;
        const key = `${parsed.chunk.x},${parsed.chunk.y}`;
        out.mobility_chunk_delta.by_chunk[key] = (out.mobility_chunk_delta.by_chunk[key] ?? 0) + 1;
        break;
      }
      case 'tile_pulse': out.tile_pulse += 1; break;
      case 'hello': out.hello += 1; break;
      default:
        out.other.count += 1;
        if (out.other.samples.length < 3) out.other.samples.push(parsed);
    }
  }
  return out;
}
```

The rest of the script (open browser, pan, zoom) is identical to smoke-7a. Add an assertion section at the end:

```js
// Sanity checks.
const checks = {
  got_chunk_snapshots_on_subscribe: summary.received_breakdown.mobility_chunk_snapshot.count > 0,
  got_no_global_mobility_delta: !receivedFrames.some(f => {
    try { return JSON.parse(f).type === 'mobility_delta'; } catch { return false; }
  }),
  pan_added_new_chunks: Object.keys(summary.received_breakdown.mobility_chunk_snapshot.by_chunk).length > 5,
};
console.log('\nSANITY CHECKS:', checks);
const allPass = Object.values(checks).every(Boolean);
process.exit(allPass ? 0 : 1);
```

- [ ] **Step 2: Start dev stack**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
pkill -f run-dev-stack 2>/dev/null; pkill -f sim-server 2>/dev/null; pkill -f "vite --host" 2>/dev/null
sleep 2
nohup npm run dev:stack > /tmp/abutown-stack.log 2>&1 & disown
until curl -sf http://127.0.0.1:8080/health > /dev/null; do sleep 3; done
echo BACKEND_UP
```

- [ ] **Step 3: Run smoke**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
node scripts/smoke-7b.mjs 2>&1 | tail -30
```

Expected: SANITY CHECKS all true. Process exits 0.

If anything fails (e.g., no `mobility_chunk_snapshot` received), instrument as we did for the 7a bugfix — that's exactly the kind of bug the smoke is designed to catch.

- [ ] **Step 4: Stop stack**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
pkill -f run-dev-stack 2>/dev/null; pkill -f sim-server 2>/dev/null; pkill -f "vite --host" 2>/dev/null
sleep 1
pgrep -fl "run-dev-stack|sim-server|vite --host" || echo all-stopped
```

- [ ] **Step 5: Commit**

```bash
git add scripts/smoke-7b.mjs
git commit -m "test(ws): browser smoke 7b verifies chunk-snapshot/delta flow + no global delta"
```

---

## Task 11: Final quality gate + progress + push

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Run all gates**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
cargo fmt --all --manifest-path backend/Cargo.toml
cargo test --locked --manifest-path backend/Cargo.toml --workspace 2>&1 | grep -E "test result|FAILED" | tail -15
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings 2>&1 | tail -5
npx vitest run 2>&1 | tail -8
npx tsc --noEmit
npm run build 2>&1 | tail -10
```

Expected:
- cargo test: all green
- cargo clippy: clean
- vitest: all green
- tsc: clean
- npm run build: succeeds

Fix any failure before continuing.

- [ ] **Step 2: Add progress.md entry**

Get timestamp:
```bash
date -u +%Y-%m-%dT%H:%M:%S.000Z
```

Insert at top of the reverse-chronological tail (immediately after the latest entry, before older ones — match the layout of recent entries):

```
<TIMESTAMP> - Phase 7b WS multi-client scalability: replaced the global filtered-delta broadcast with per-chunk `tokio::broadcast` channels. `MobilityWorld::tick_mobility` now returns `HashMap<ChunkCoord, MobilityChunkDelta>`, computing `left_*` via new `PreviousAgentChunks`/`PreviousVehicleChunks` resources that track each entity's chunk at end-of-tick. New `AppState.chunk_channels: Arc<DashMap<ChunkCoord, broadcast::Sender<MobilityChunkDeltaDto>>>` holds one channel per Active|Hot chunk (created on first subscribe, reaped on last unsubscribe). Each WS handler maintains a `tokio_stream::StreamMap<ChunkCoord, BroadcastStream<…>>` for dynamic per-chunk multiplexing; `tokio::select!` reads it alongside `socket.recv`. Subscribe emits one `ServerMessageDto::MobilityChunkSnapshot` per added chunk (replaces the Phase-7a synthetic delta); ticks emit per-chunk `MobilityChunkDelta` only for chunks with changes. The lagged-receiver recovery path re-snapshots the affected chunk. The old `ServerMessageDto::MobilityDelta` variant, `MobilityDeltaDto`, `filtered_mobility_delta_from_dto`, `synthetic_mobility_delta_for_subscription`, `next_mobility_delta`, `next_server_messages`, and the global `tokio::broadcast::Sender<ServerMessageDto>` in `AppState` are all gone — state-of-the-art wire protocol, no per-client filter step. Per-client per-tick CPU now scales with `N_subscribed_chunks`, not `N_world_entities`. Phase 7b of the WS scalability arc; Phase 7c (Arc-snapshot lock-free reads) is the next architectural step.
```

- [ ] **Step 3: Commit + push**

```bash
cd /Users/ramonfuglister/Desktop/Coding/abutown
git add progress.md
git commit -m "chore: phase 7b quality gate + progress note"
git push origin main
```

---

## Self-Review

**1. Spec coverage:**

| Spec requirement | Task |
|---|---|
| New AppState `chunk_channels: Arc<DashMap<…>>` | Task 4 |
| Per-chunk `tokio::broadcast` channel | Task 4 + Task 5 |
| `MobilityWorld::tick_mobility` returns per-chunk map | Task 3 |
| `PreviousAgentChunks` / `PreviousVehicleChunks` for `left_*` | Task 1 + Task 3 |
| Channel lifecycle: aggressive reap on last unsubscribe | Task 5 |
| Per-handler StreamMap | Task 5 + Task 6 |
| `MobilityChunkDelta` + `MobilityChunkSnapshot` DTOs | Task 1 |
| Subscribe emits `MobilityChunkSnapshot` | Task 5 |
| Tick fan-out to chunk_channels | Task 7 |
| Lagged-receiver recovery: re-send snapshot | Task 6 |
| Delete old `MobilityDelta`, `filtered_*`, `synthetic_*`, global broadcast | Task 8 |
| Frontend: new variant handling, delete old | Task 9 |
| 3-client integration test adapted | Task 8 (test adaptation) |
| Browser smoke for new chunk-frame flow | Task 10 |
| Final gate + progress | Task 11 |

All covered.

**2. Placeholder scan:** No "TBD" / "implement later". Task 2 Step 4 has a fallback ("if neither exists, query Position component directly") with the actual code shown — concrete escape hatch, not a TODO. Task 5 Step 3 has an "if these helpers don't exist, look in dto.rs" — same pattern, escape hatch with concrete fallback.

**3. Type consistency:**
- `MobilityChunkDelta`, `MobilityChunkDeltaDto` consistently named across Tasks 1, 3, 5, 6, 7, 8, 9.
- `MobilityChunkSnapshot`, `MobilityChunkSnapshotDto` likewise.
- `chunk_channels` field name consistent across Tasks 4, 5, 6, 7.
- `chunk_streams` (the per-handler StreamMap) consistent in Tasks 5 + 6.
- `tick_world_mobility` (Task 7) is the only addition to SimulationRuntime's API; `tick_mobility` on MobilityWorld is the world-level method.

**Order rationale:** Pure additive infra first (Tasks 1+2), then the breaking tick_mobility signature change (Task 3, all callers updated in same commit), then the new channel infra alongside the old (Tasks 4–7), then a single big delete commit (Task 8). Frontend (Task 9) follows server because it's protocol-driven. Browser smoke (Task 10) and gate (Task 11) close out.

**Scope check:** 11 tasks, ~11 commits. Each task is bite-sized and produces a working green-tests commit. The big-delete commit (Task 8) is the riskiest — touches multiple files but only deletes code, doesn't change behavior.

**Risks acknowledged in spec, addressed in plan:**
- DashMap vs RwLock<HashMap>: Task 4 uses DashMap per spec.
- Lagged-receiver recovery: Task 6 implements the re-snapshot path.
- left_* semantics across chunks: Task 3 builds the prev-vs-current comparison and produces the correct per-chunk left lists.
- Browser-smoke per CLAUDE.md: Task 10 is mandatory (frontend wire change).
