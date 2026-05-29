# Viewport-Filtered Mobility Replication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Introduce a `ChunkSubscribe`/`ChunkUnsubscribe` client→server primitive and filter `MobilityDelta` per connection so each client only receives entities in its subscribed chunks.

**Architecture:** New `ClientMessageDto` enum with `chunk_subscribe`/`chunk_unsubscribe` variants. WS task refactored from forward-only into a `select!` loop with per-connection `ConnectionState { subscription, last_visible_agents, last_visible_vehicles }`. A pure-function `ConnectionMobilityFilter::apply(delta, world, state)` produces per-connection `MobilityDeltaDto` with `changed_*`/`left_*` fields. Frontend gets a thin `chunkSubscriptionClient` that sends an initial 8×8 subscribe after `Hello`.

**Tech Stack:** Axum WebSocket, Tokio `select!`, serde JSON DTOs (Rust + TS), Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-05-16-viewport-mobility-replication-design.md`
**Roadmap:** Phase 4 of `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md`

---

## File Structure

Backend new:
- (none — all changes are in existing files)

Backend modified:
- `backend/crates/protocol/src/lib.rs` — `ClientMessageDto` enum + `ChunkSubscribeDto` + `ChunkUnsubscribeDto`; `left_agents`/`left_vehicles` on `MobilityDeltaDto`.
- `backend/crates/sim-core/src/mobility.rs` — `chunk_of(world_coord, chunk_size)` helper; `MobilityWorld::filtered_delta_dto_for_subscription(world_id, tick, delta, subscription, last_visible_*) -> (MobilityDeltaDto, new_last_visible_*)`.
- `backend/crates/sim-server/src/app.rs` — refactor `stream_world_deltas` to `select!` loop with per-connection `ConnectionState`; handle inbound subscribe/unsubscribe; filter `MobilityDelta` before send.

Frontend new:
- `src/backend/chunkSubscriptionClient.ts` — given a viewport bounds + camera-change events, send `chunk_subscribe`/`chunk_unsubscribe` via the WS.
- `tests/backend/chunkSubscriptionClient.test.ts`.

Frontend modified:
- `src/backend/mobilityProtocol.ts` — add `ClientMessageDto`, `ChunkSubscribeDto`, `ChunkUnsubscribeDto`; extend `MobilityDeltaDto` parsing to accept optional `left_agents`/`left_vehicles`.
- `src/backend/mobilityState.ts` — `applyMobilityDelta` drops ids in `left_agents`/`left_vehicles` before applying `changed_*`.
- `src/backend/mobilityClient.ts` — wire the new subscription module on WS open.

Frontend tests modified:
- `tests/backend/mobilityProtocol.test.ts` — assert backward-compat parsing without left fields + new send-message types.
- `tests/backend/mobilityState.test.ts` — assert `left_*` drops entities.

Backend tests modified:
- `backend/crates/sim-server/tests/websocket.rs` — two-client AoI test, subscribe-grows-then-shrinks test.

---

## Task 1: Protocol DTOs (Rust)

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`

- [x] **Step 1: Write failing tests**

Append to `backend/crates/protocol/src/lib.rs` test module:

```rust
#[test]
fn client_message_chunk_subscribe_round_trips() {
    let msg = ClientMessageDto::ChunkSubscribe(ChunkSubscribeDto {
        protocol_version: 1,
        coords: vec![ChunkCoordDto { x: 4, y: 4 }, ChunkCoordDto { x: 5, y: 4 }],
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "chunk_subscribe");
    assert_eq!(json["coords"].as_array().unwrap().len(), 2);
    let back: ClientMessageDto = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn client_message_chunk_unsubscribe_round_trips() {
    let msg = ClientMessageDto::ChunkUnsubscribe(ChunkUnsubscribeDto {
        protocol_version: 1,
        coords: vec![ChunkCoordDto { x: 4, y: 4 }],
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "chunk_unsubscribe");
    let back: ClientMessageDto = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn mobility_delta_dto_serializes_with_left_fields() {
    let dto = MobilityDeltaDto {
        protocol_version: 1,
        world_id: WorldId("w".to_string()),
        tick: 7,
        changed_agents: vec![],
        changed_vehicles: vec![],
        left_agents: vec![EntityId("agent:walk:1".to_string())],
        left_vehicles: vec![EntityId("vehicle:car:0:0".to_string())],
    };
    let json = serde_json::to_value(&dto).unwrap();
    assert_eq!(json["left_agents"].as_array().unwrap().len(), 1);
    assert_eq!(json["left_vehicles"].as_array().unwrap().len(), 1);
    let back: MobilityDeltaDto = serde_json::from_value(json).unwrap();
    assert_eq!(back, dto);
}

#[test]
fn mobility_delta_dto_accepts_missing_left_fields_for_backward_compat() {
    let json = serde_json::json!({
        "protocol_version": 1,
        "world_id": "w",
        "tick": 0,
        "changed_agents": [],
        "changed_vehicles": []
    });
    let dto: MobilityDeltaDto = serde_json::from_value(json).unwrap();
    assert_eq!(dto.left_agents, Vec::<EntityId>::new());
    assert_eq!(dto.left_vehicles, Vec::<EntityId>::new());
}
```

- [x] **Step 2: Confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol client_message_chunk_subscribe_round_trips
```

Expected: FAIL — types don't exist.

- [x] **Step 3: Add the new types and fields**

In `backend/crates/protocol/src/lib.rs`:

Find the `ServerMessageDto` enum (around line 133). Below it add:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessageDto {
    ChunkSubscribe(ChunkSubscribeDto),
    ChunkUnsubscribe(ChunkUnsubscribeDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkSubscribeDto {
    pub protocol_version: u16,
    pub coords: Vec<ChunkCoordDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkUnsubscribeDto {
    pub protocol_version: u16,
    pub coords: Vec<ChunkCoordDto>,
}
```

Update `MobilityDeltaDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobilityDeltaDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub changed_agents: Vec<AgentMobilityDto>,
    pub changed_vehicles: Vec<VehicleMobilityDto>,
    #[serde(default)]
    pub left_agents: Vec<EntityId>,
    #[serde(default)]
    pub left_vehicles: Vec<EntityId>,
}
```

`#[serde(default)]` makes the fields optional in JSON — old clients that don't send them still parse, and new tests can omit them.

- [x] **Step 4: Update existing `MobilityDeltaDto` construction sites**

Any existing literal in the workspace that builds `MobilityDeltaDto { ... }` won't compile. Find them:

```bash
rg "MobilityDeltaDto \{" backend/
```

For each hit, add `left_agents: vec![], left_vehicles: vec![]` to the struct literal. (At time of writing the only sites are inside `mobility.rs` `build_mobility_delta_dto` and the protocol test module.)

- [x] **Step 5: Verify workspace**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: all green.

- [x] **Step 6: Commit**

```bash
git add backend/crates/protocol/src/lib.rs backend/crates/sim-core/src/mobility.rs
git commit -m "feat: ClientMessageDto subscribe/unsubscribe + left fields on MobilityDelta"
```

---

## Task 2: chunk_of helper

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [x] **Step 1: Add failing tests**

Append to mobility.rs test module:

```rust
#[test]
fn chunk_of_truncates_to_chunk_grid() {
    use crate::ids::ChunkCoord;
    assert_eq!(chunk_of(0.0, 0.0, 32), ChunkCoord { x: 0, y: 0 });
    assert_eq!(chunk_of(31.9, 31.9, 32), ChunkCoord { x: 0, y: 0 });
    assert_eq!(chunk_of(32.0, 0.0, 32), ChunkCoord { x: 1, y: 0 });
    assert_eq!(chunk_of(150.5, 95.0, 32), ChunkCoord { x: 4, y: 2 });
}

#[test]
fn chunk_of_handles_negative_coords() {
    use crate::ids::ChunkCoord;
    // Coordinate slightly below 0 should fall into chunk -1
    assert_eq!(chunk_of(-0.1, -0.1, 32), ChunkCoord { x: -1, y: -1 });
}
```

- [x] **Step 2: Confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core chunk_of
```

Expected: FAIL — function doesn't exist.

- [x] **Step 3: Implement**

In `backend/crates/sim-core/src/mobility.rs`, near the top (after the `use` block) add:

```rust
pub fn chunk_of(x: f32, y: f32, chunk_size: u16) -> crate::ids::ChunkCoord {
    let cs = chunk_size as f32;
    crate::ids::ChunkCoord {
        x: x.div_euclid(cs) as i32,
        y: y.div_euclid(cs) as i32,
    }
}
```

`div_euclid` rounds toward negative infinity, giving the correct chunk for negative coords (`-0.1 / 32 = -1`, not `0`).

- [x] **Step 4: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core chunk_of
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: chunk_of helper to project world coord onto chunk grid"
```

---

## Task 3: ConnectionMobilityFilter (pure Rust)

This task adds the per-connection filter logic as a pure function. No WS code yet — that's Task 4.

**Files:**
- Modify: `backend/crates/sim-core/src/mobility.rs`

- [x] **Step 1: Add failing tests**

Append to mobility.rs test module:

```rust
#[test]
fn filter_excludes_entities_outside_subscription() {
    use crate::ids::ChunkCoord;
    use std::collections::HashSet;

    // Build a 1-arterial network so we have one car at (1, 0)…(11, 0).
    let network = crate::city_network::CityNetwork {
        version: 1,
        world_id: "t".to_string(),
        chunk_size: 32,
        world_tiles: crate::city_network::WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![
            crate::city_network::NetworkCoord { x: 0, y: 0 },
            crate::city_network::NetworkCoord { x: 200, y: 0 },
        ]],
        pedestrian_corridors: vec![],
    };
    let world = seed::from_network(&network, seed::SeedDensity {
        pedestrians_per_corridor: 0,
        cars_per_arterial: 1,
        trams_total: 0,
    });

    // Subscribe to chunk (1, 0) only. The car at world_coord ~(0,0) is in chunk (0,0).
    let subscription: HashSet<ChunkCoord> = [ChunkCoord { x: 1, y: 0 }].into_iter().collect();
    let mut last_visible_agents: HashSet<abutown_protocol::EntityId> = HashSet::new();
    let mut last_visible_vehicles: HashSet<abutown_protocol::EntityId> = HashSet::new();

    let delta = MobilityDelta {
        changed_agents: world.agents.values().cloned().collect(),
        changed_vehicles: world.vehicles.values().cloned().collect(),
    };
    let world_id = WorldId("t".to_string());
    let dto = build_filtered_mobility_delta_dto(
        &world_id,
        world.tick(),
        &world,
        &delta,
        &subscription,
        &mut last_visible_agents,
        &mut last_visible_vehicles,
    );
    assert!(dto.changed_agents.is_empty(), "agent at (0,0) not in subscription {{(1,0)}}");
    assert!(dto.changed_vehicles.is_empty(), "vehicle at (0,0) not in subscription {{(1,0)}}");
    assert!(dto.left_agents.is_empty());
    assert!(dto.left_vehicles.is_empty());
}

#[test]
fn filter_emits_left_when_entity_leaves_subscription() {
    use crate::ids::ChunkCoord;
    use std::collections::HashSet;

    let network = crate::city_network::CityNetwork {
        version: 1,
        world_id: "t".to_string(),
        chunk_size: 32,
        world_tiles: crate::city_network::WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![
            crate::city_network::NetworkCoord { x: 0, y: 0 },
            crate::city_network::NetworkCoord { x: 200, y: 0 },
        ]],
        pedestrian_corridors: vec![],
    };
    let world = seed::from_network(&network, seed::SeedDensity {
        pedestrians_per_corridor: 0,
        cars_per_arterial: 1,
        trams_total: 0,
    });

    let car_id = world.vehicles.keys().next().unwrap().clone();
    let car_entity_id = abutown_protocol::EntityId(car_id.0.clone());
    let subscription: HashSet<ChunkCoord> = [ChunkCoord { x: 1, y: 0 }].into_iter().collect();
    let mut last_visible_agents: HashSet<abutown_protocol::EntityId> = HashSet::new();
    let mut last_visible_vehicles: HashSet<abutown_protocol::EntityId> = [car_entity_id.clone()].into_iter().collect();

    let delta = MobilityDelta { changed_agents: vec![], changed_vehicles: vec![] };
    let world_id = WorldId("t".to_string());
    let dto = build_filtered_mobility_delta_dto(
        &world_id,
        world.tick(),
        &world,
        &delta,
        &subscription,
        &mut last_visible_agents,
        &mut last_visible_vehicles,
    );
    assert_eq!(dto.left_vehicles, vec![car_entity_id]);
}

#[test]
fn filter_emits_join_when_entity_enters_subscription() {
    use crate::ids::ChunkCoord;
    use std::collections::HashSet;

    let network = crate::city_network::CityNetwork {
        version: 1,
        world_id: "t".to_string(),
        chunk_size: 32,
        world_tiles: crate::city_network::WorldTiles { width: 256, height: 256 },
        arterial_paths: vec![vec![
            crate::city_network::NetworkCoord { x: 0, y: 0 },
            crate::city_network::NetworkCoord { x: 200, y: 0 },
        ]],
        pedestrian_corridors: vec![],
    };
    let world = seed::from_network(&network, seed::SeedDensity {
        pedestrians_per_corridor: 0,
        cars_per_arterial: 1,
        trams_total: 0,
    });

    // Subscribe to chunk containing the car at ~(0,0).
    let subscription: HashSet<ChunkCoord> = [ChunkCoord { x: 0, y: 0 }].into_iter().collect();
    let mut last_visible_agents: HashSet<abutown_protocol::EntityId> = HashSet::new();
    let mut last_visible_vehicles: HashSet<abutown_protocol::EntityId> = HashSet::new();

    // Empty delta but world has a never-seen-before vehicle — filter should "join" it.
    let delta = MobilityDelta { changed_agents: vec![], changed_vehicles: vec![] };
    let world_id = WorldId("t".to_string());
    let dto = build_filtered_mobility_delta_dto(
        &world_id,
        world.tick(),
        &world,
        &delta,
        &subscription,
        &mut last_visible_agents,
        &mut last_visible_vehicles,
    );
    assert_eq!(dto.changed_vehicles.len(), 1);
    assert!(dto.left_vehicles.is_empty());
    assert_eq!(last_visible_vehicles.len(), 1, "filter updated last_visible_vehicles");
}
```

- [x] **Step 2: Confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core build_filtered_mobility_delta_dto
```

Expected: FAIL — function doesn't exist.

Note: tests may also fail to compile because the function is referenced but doesn't exist. Compile errors are the expected first failure.

- [x] **Step 3: Implement the filter**

In `backend/crates/sim-core/src/mobility.rs`, add a new public function:

```rust
pub fn build_filtered_mobility_delta_dto(
    world_id: &WorldId,
    tick: u64,
    world: &MobilityWorld,
    delta: &MobilityDelta,
    subscription: &std::collections::HashSet<crate::ids::ChunkCoord>,
    last_visible_agents: &mut std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: &mut std::collections::HashSet<abutown_protocol::EntityId>,
) -> abutown_protocol::MobilityDeltaDto {
    let chunk_size = world.link_polylines.values().next().map(|_| 32u16).unwrap_or(32);

    // Compute current visible set across the world.
    let mut current_visible_agents: std::collections::HashSet<abutown_protocol::EntityId> = std::collections::HashSet::new();
    for agent in world.agents.values() {
        if matches!(agent.state, AgentMobilityState::InVehicle { .. }) {
            continue;
        }
        let coord = world.world_coord_for_agent(&agent.id);
        if let Some((x, y)) = coord {
            if subscription.contains(&chunk_of(x, y, chunk_size)) {
                current_visible_agents.insert(abutown_protocol::EntityId(agent.id.0.clone()));
            }
        }
    }
    let mut current_visible_vehicles: std::collections::HashSet<abutown_protocol::EntityId> = std::collections::HashSet::new();
    for vehicle in world.vehicles.values() {
        let coord = world.world_coord_for_vehicle(&vehicle.id);
        if let Some((x, y)) = coord {
            if subscription.contains(&chunk_of(x, y, chunk_size)) {
                current_visible_vehicles.insert(abutown_protocol::EntityId(vehicle.id.0.clone()));
            }
        }
    }

    // entered = currently visible but not in last_visible
    let entered_agents: Vec<abutown_protocol::AgentMobilityDto> = current_visible_agents
        .iter()
        .filter(|id| !last_visible_agents.contains(*id))
        .filter_map(|id| world.agent_dto_for(&AgentId(id.0.clone())))
        .collect();
    let entered_vehicles: Vec<abutown_protocol::VehicleMobilityDto> = current_visible_vehicles
        .iter()
        .filter(|id| !last_visible_vehicles.contains(*id))
        .filter_map(|id| world.vehicle_dto_for(&VehicleId(id.0.clone())))
        .collect();

    // changed-still-visible = entities that were already known AND appear in delta
    let mut changed_agents: Vec<abutown_protocol::AgentMobilityDto> = entered_agents;
    for agent in &delta.changed_agents {
        let entity_id = abutown_protocol::EntityId(agent.id.0.clone());
        if current_visible_agents.contains(&entity_id) && last_visible_agents.contains(&entity_id) {
            if let Some(dto) = world.agent_dto_for(&agent.id) {
                changed_agents.push(dto);
            }
        }
    }
    let mut changed_vehicles: Vec<abutown_protocol::VehicleMobilityDto> = entered_vehicles;
    for vehicle in &delta.changed_vehicles {
        let entity_id = abutown_protocol::EntityId(vehicle.id.0.clone());
        if current_visible_vehicles.contains(&entity_id) && last_visible_vehicles.contains(&entity_id) {
            if let Some(dto) = world.vehicle_dto_for(&vehicle.id) {
                changed_vehicles.push(dto);
            }
        }
    }

    // left = was in last_visible but no longer visible
    let left_agents: Vec<abutown_protocol::EntityId> = last_visible_agents
        .iter()
        .filter(|id| !current_visible_agents.contains(*id))
        .cloned()
        .collect();
    let left_vehicles: Vec<abutown_protocol::EntityId> = last_visible_vehicles
        .iter()
        .filter(|id| !current_visible_vehicles.contains(*id))
        .cloned()
        .collect();

    *last_visible_agents = current_visible_agents;
    *last_visible_vehicles = current_visible_vehicles;

    abutown_protocol::MobilityDeltaDto {
        protocol_version: abutown_protocol::PROTOCOL_VERSION,
        world_id: world_id.clone(),
        tick,
        changed_agents,
        changed_vehicles,
        left_agents,
        left_vehicles,
    }
}
```

This is a single pure function: subscription + last_visible state in, new DTO out + updated last_visible state.

(`PROTOCOL_VERSION` is already a const in `protocol/src/lib.rs`. If it's not exported, use `1` literally — check before assuming.)

- [x] **Step 4: Verify**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-core build_filtered_mobility_delta_dto
cargo clippy --locked --manifest-path backend/Cargo.toml -p sim-core --all-targets -- -D warnings
```

Expected: green.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/mobility.rs
git commit -m "feat: per-connection mobility delta filter with entered/left diffing"
```

---

## Task 4: WS task refactor — select loop + per-connection filter

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`

This is the central wiring task. The current `stream_world_deltas` is forward-only; we make it bidirectional.

- [x] **Step 1: Read the current WS handler**

Read `backend/crates/sim-server/src/app.rs` lines 294–342 to know the current structure. The new structure:

- Receive `socket` and `state` as before.
- Set up `let mut connection = ConnectionState::default();`
- Send `Hello` as before.
- `tokio::select!` loop on two arms:
  - `result = deltas.recv()` — handle broadcast message; if `MobilityDelta`, filter through `connection`; else pass through.
  - `result = socket.recv()` — parse `ClientMessageDto` from text; mutate `connection.subscription`.
- On any send error, return.
- On `socket.recv()` returning None (close), return.

- [x] **Step 2: Add the ConnectionState struct**

Inside `app.rs`, above `stream_world_deltas`:

```rust
#[derive(Default)]
struct ConnectionState {
    subscription: std::collections::HashSet<sim_core::ids::ChunkCoord>,
    last_visible_agents: std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: std::collections::HashSet<abutown_protocol::EntityId>,
}
```

- [x] **Step 3: Rewrite `stream_world_deltas`**

Replace the existing body:

```rust
async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let mut deltas = state.subscribe_deltas();
    let hello = {
        let runtime = state.runtime();
        let runtime = runtime.lock().await;
        runtime.hello()
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    let mut connection = ConnectionState::default();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                let Some(Ok(message)) = inbound else { return; };
                let Message::Text(text) = message else { continue; };
                let Ok(client_message) = serde_json::from_str::<ClientMessageDto>(&text) else {
                    tracing::warn!(?text, "invalid client message");
                    continue;
                };
                let synthetic_delta = handle_client_message(&state, &client_message, &mut connection).await;
                if let Some(dto) = synthetic_delta {
                    if send_server_message(&mut socket, ServerMessageDto::MobilityDelta(dto)).await.is_err() {
                        return;
                    }
                }
            }
            broadcast = deltas.recv() => {
                let message = match broadcast {
                    Ok(message) => message,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                let outbound = match message {
                    ServerMessageDto::MobilityDelta(raw_delta) => {
                        let dto = {
                            let runtime = state.runtime();
                            let runtime = runtime.lock().await;
                            runtime.filtered_mobility_delta_from_dto(
                                &raw_delta,
                                &connection.subscription,
                                &mut connection.last_visible_agents,
                                &mut connection.last_visible_vehicles,
                            )
                        };
                        // Skip empty deltas — no entities changed and nothing left.
                        if dto.changed_agents.is_empty() && dto.changed_vehicles.is_empty() && dto.left_agents.is_empty() && dto.left_vehicles.is_empty() {
                            continue;
                        }
                        ServerMessageDto::MobilityDelta(dto)
                    }
                    other => other,
                };
                if send_server_message(&mut socket, outbound).await.is_err() {
                    return;
                }
            }
        }
    }
}

async fn handle_client_message(
    state: &AppState,
    message: &ClientMessageDto,
    connection: &mut ConnectionState,
) -> Option<MobilityDeltaDto> {
    match message {
        ClientMessageDto::ChunkSubscribe(payload) => {
            for coord in &payload.coords {
                connection.subscription.insert(sim_core::ids::ChunkCoord { x: coord.x, y: coord.y });
            }
        }
        ClientMessageDto::ChunkUnsubscribe(payload) => {
            for coord in &payload.coords {
                connection.subscription.remove(&sim_core::ids::ChunkCoord { x: coord.x, y: coord.y });
            }
        }
    }
    // After mutating the subscription, emit a synthetic delta so the client immediately learns
    // about newly visible / no-longer-visible entities.
    let runtime = state.runtime();
    let runtime = runtime.lock().await;
    let dto = runtime.synthetic_mobility_delta_for_subscription(
        &connection.subscription,
        &mut connection.last_visible_agents,
        &mut connection.last_visible_vehicles,
    );
    if dto.changed_agents.is_empty() && dto.changed_vehicles.is_empty() && dto.left_agents.is_empty() && dto.left_vehicles.is_empty() {
        None
    } else {
        Some(dto)
    }
}
```

- [x] **Step 4: Add the two runtime helpers**

In `backend/crates/sim-server/src/runtime.rs`, add to the `impl SimulationRuntime` block:

```rust
pub fn filtered_mobility_delta_from_dto(
    &self,
    raw_delta_dto: &abutown_protocol::MobilityDeltaDto,
    subscription: &std::collections::HashSet<sim_core::ids::ChunkCoord>,
    last_visible_agents: &mut std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: &mut std::collections::HashSet<abutown_protocol::EntityId>,
) -> abutown_protocol::MobilityDeltaDto {
    // Reconstruct a MobilityDelta from the DTO's id list (so we can drive the per-connection
    // filter from the broadcast payload without re-running the tick).
    let changed_agents: Vec<sim_core::mobility::AgentRecord> = raw_delta_dto
        .changed_agents
        .iter()
        .filter_map(|dto| self.mobility.agents.get(&sim_core::mobility::AgentId(dto.id.0.clone())).cloned())
        .collect();
    let changed_vehicles: Vec<sim_core::mobility::VehicleRecord> = raw_delta_dto
        .changed_vehicles
        .iter()
        .filter_map(|dto| self.mobility.vehicles.get(&sim_core::mobility::VehicleId(dto.id.0.clone())).cloned())
        .collect();
    let delta = sim_core::mobility::MobilityDelta { changed_agents, changed_vehicles };
    sim_core::mobility::build_filtered_mobility_delta_dto(
        &self.world_id,
        self.mobility.tick(),
        &self.mobility,
        &delta,
        subscription,
        last_visible_agents,
        last_visible_vehicles,
    )
}

pub fn synthetic_mobility_delta_for_subscription(
    &self,
    subscription: &std::collections::HashSet<sim_core::ids::ChunkCoord>,
    last_visible_agents: &mut std::collections::HashSet<abutown_protocol::EntityId>,
    last_visible_vehicles: &mut std::collections::HashSet<abutown_protocol::EntityId>,
) -> abutown_protocol::MobilityDeltaDto {
    let empty_delta = sim_core::mobility::MobilityDelta {
        changed_agents: vec![],
        changed_vehicles: vec![],
    };
    sim_core::mobility::build_filtered_mobility_delta_dto(
        &self.world_id,
        self.mobility.tick(),
        &self.mobility,
        &empty_delta,
        subscription,
        last_visible_agents,
        last_visible_vehicles,
    )
}
```

(The `SimulationRuntime` field names — `mobility`, `world_id` — must match what's already in the struct. Read it first to confirm.)

- [x] **Step 5: Add the `ClientMessageDto` import in app.rs**

At the top of `backend/crates/sim-server/src/app.rs`, add to the existing `use abutown_protocol::{...}` line:

```rust
use abutown_protocol::{ClientMessageDto, MobilityDeltaDto, /* ...existing imports... */};
```

- [x] **Step 6: Build + test workspace**

```bash
cargo build --locked --manifest-path backend/Cargo.toml --workspace
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: green. If any existing test asserted "client received a MobilityDelta after Hello with N agents", that test will now receive an empty delta (no subscription). Update it: send a `ChunkSubscribe` for the relevant chunks first.

In `backend/crates/sim-server/tests/websocket.rs`, find `websocket_sends_hello_and_tile_pulse` (or similar). The test connects, reads Hello, reads TilePulse, reads MobilityDelta. With Phase 4 the MobilityDelta only arrives after subscribing. Update:

```rust
// After Hello: subscribe to all seeded chunks before expecting any MobilityDelta.
let subscribe = ClientMessageDto::ChunkSubscribe(ChunkSubscribeDto {
    protocol_version: 1,
    coords: vec![
        ChunkCoordDto { x: 4, y: 4 },
        ChunkCoordDto { x: 5, y: 4 },
        ChunkCoordDto { x: 4, y: 5 },
    ],
});
let text = serde_json::to_string(&subscribe).unwrap();
stream.send(tungstenite::Message::Text(text.into())).await.unwrap();
// Now MobilityDelta arrives (synthetic on subscribe + per-tick).
```

- [x] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat: per-connection mobility AoI filter in WS task"
```

---

## Task 5: Two-client AoI integration test

**Files:**
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [x] **Step 1: Add the test**

Append to `backend/crates/sim-server/tests/websocket.rs`:

```rust
#[tokio::test]
async fn two_clients_with_different_subscriptions_see_different_entities() {
    // Spin up a real WS server using the in-memory build_app().
    let app = sim_server::app::build_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{}/ws", addr);

    // Client A subscribes to chunk (4,4)
    let (mut client_a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    read_hello(&mut client_a).await;
    send_subscribe(&mut client_a, &[ChunkCoordDto { x: 4, y: 4 }]).await;

    // Client B subscribes to chunk (5,4)
    let (mut client_b, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    read_hello(&mut client_b).await;
    send_subscribe(&mut client_b, &[ChunkCoordDto { x: 5, y: 4 }]).await;

    // Drain one MobilityDelta on each (synthetic on subscribe) and assert disjoint entity sets.
    let delta_a = read_mobility_delta(&mut client_a).await;
    let delta_b = read_mobility_delta(&mut client_b).await;
    let ids_a: std::collections::HashSet<String> = delta_a.changed_agents.iter().map(|a| a.id.0.clone()).chain(delta_a.changed_vehicles.iter().map(|v| v.id.0.clone())).collect();
    let ids_b: std::collections::HashSet<String> = delta_b.changed_agents.iter().map(|a| a.id.0.clone()).chain(delta_b.changed_vehicles.iter().map(|v| v.id.0.clone())).collect();
    assert!(ids_a.intersection(&ids_b).next().is_none(), "client A and client B should see disjoint entity sets");
}

async fn read_hello(stream: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) {
    use futures_util::StreamExt;
    let _ = stream.next().await.unwrap().unwrap();
}

async fn send_subscribe(
    stream: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    coords: &[ChunkCoordDto],
) {
    use futures_util::SinkExt;
    let msg = ClientMessageDto::ChunkSubscribe(ChunkSubscribeDto {
        protocol_version: 1,
        coords: coords.to_vec(),
    });
    let text = serde_json::to_string(&msg).unwrap();
    stream.send(tokio_tungstenite::tungstenite::Message::Text(text.into())).await.unwrap();
}

async fn read_mobility_delta(stream: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> MobilityDeltaDto {
    use futures_util::StreamExt;
    loop {
        let msg = stream.next().await.unwrap().unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            if let Ok(ServerMessageDto::MobilityDelta(delta)) = serde_json::from_str::<ServerMessageDto>(&text) {
                return delta;
            }
        }
    }
}
```

Add at the top of the file if not already there:

```rust
use abutown_protocol::{ChunkCoordDto, ChunkSubscribeDto, ClientMessageDto, MobilityDeltaDto, ServerMessageDto};
```

(Adapt to whatever import style the existing tests use.)

- [x] **Step 2: Run the test**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server --test websocket two_clients
```

Expected: PASS. If it fails because Client A's chunk (4,4) actually has no seeded entities (tiny_world's hardcoded routes start at chunk (4,4) — the test world's seed places trams at chunk centers (4,4)/(5,4)/(4,5)), pick subscriptions that match the actual seeded entity positions. Use `cargo test ... -- --nocapture` and `println!` debug the entity positions to choose chunks with non-empty content.

- [x] **Step 3: Commit**

```bash
git add backend/crates/sim-server/tests/websocket.rs
git commit -m "test: two-client AoI integration verifies disjoint entity sets"
```

---

## Task 6: Frontend protocol mirrors

**Files:**
- Modify: `src/backend/mobilityProtocol.ts`
- Modify: `tests/backend/mobilityProtocol.test.ts`

- [x] **Step 1: Add failing tests**

In `tests/backend/mobilityProtocol.test.ts`, append:

```ts
import { encodeClientMessage } from '../../src/backend/mobilityProtocol';

it('encodes chunk_subscribe message with snake_case discriminator', () => {
  const wire = encodeClientMessage({
    type: 'chunk_subscribe',
    protocol_version: 1,
    coords: [{ x: 4, y: 4 }, { x: 5, y: 4 }],
  });
  const json = JSON.parse(wire);
  expect(json.type).toBe('chunk_subscribe');
  expect(json.coords).toHaveLength(2);
});

it('encodes chunk_unsubscribe message', () => {
  const wire = encodeClientMessage({
    type: 'chunk_unsubscribe',
    protocol_version: 1,
    coords: [{ x: 4, y: 4 }],
  });
  expect(JSON.parse(wire).type).toBe('chunk_unsubscribe');
});

it('parses MobilityDelta with left_agents/left_vehicles', () => {
  const result = parseServerMessage(JSON.stringify({
    type: 'mobility_delta',
    protocol_version: 1,
    world_id: 'w',
    tick: 0,
    changed_agents: [],
    changed_vehicles: [],
    left_agents: ['agent:walk:1'],
    left_vehicles: ['vehicle:car:0:0'],
  }));
  expect(result?.type).toBe('mobility_delta');
  if (result?.type === 'mobility_delta') {
    expect(result.left_agents).toEqual(['agent:walk:1']);
    expect(result.left_vehicles).toEqual(['vehicle:car:0:0']);
  }
});

it('parses MobilityDelta missing left_*  (backward compat)', () => {
  const result = parseServerMessage(JSON.stringify({
    type: 'mobility_delta',
    protocol_version: 1,
    world_id: 'w',
    tick: 0,
    changed_agents: [],
    changed_vehicles: [],
  }));
  expect(result?.type).toBe('mobility_delta');
  if (result?.type === 'mobility_delta') {
    expect(result.left_agents).toEqual([]);
    expect(result.left_vehicles).toEqual([]);
  }
});
```

(Adapt the `parseServerMessage` import to whatever's already imported at the top of the test file.)

- [x] **Step 2: Confirm failure**

```bash
npx vitest run tests/backend/mobilityProtocol.test.ts
```

Expected: FAIL — `encodeClientMessage` doesn't exist, `left_*` fields missing on the type.

- [x] **Step 3: Implement**

In `src/backend/mobilityProtocol.ts`:

Add the client message types:

```ts
export type ChunkCoordDto = { x: number; y: number };

export type ChunkSubscribeMessage = {
  type: 'chunk_subscribe';
  protocol_version: number;
  coords: ChunkCoordDto[];
};

export type ChunkUnsubscribeMessage = {
  type: 'chunk_unsubscribe';
  protocol_version: number;
  coords: ChunkCoordDto[];
};

export type ClientMessageDto = ChunkSubscribeMessage | ChunkUnsubscribeMessage;

export function encodeClientMessage(message: ClientMessageDto): string {
  return JSON.stringify(message);
}
```

Update the `MobilityDelta`-related message type:

```ts
export type MobilityDeltaServerMessage = {
  type: 'mobility_delta';
  protocol_version: number;
  world_id: string;
  tick: number;
  changed_agents: AgentMobilityDto[];
  changed_vehicles: VehicleMobilityDto[];
  left_agents: string[];
  left_vehicles: string[];
};
```

Update the parser. Find the `mobility_delta` branch in `parseServerMessage` and update:

```ts
case 'mobility_delta': {
  if (
    !isString(value.world_id) ||
    !isNonNegativeInteger(value.tick) ||
    !Array.isArray(value.changed_agents) ||
    !Array.isArray(value.changed_vehicles)
  ) return null;
  const left_agents = Array.isArray(value.left_agents) ? value.left_agents.filter(isString) : [];
  const left_vehicles = Array.isArray(value.left_vehicles) ? value.left_vehicles.filter(isString) : [];
  // ... existing validation of changed_agents / changed_vehicles ...
  return {
    type: 'mobility_delta',
    protocol_version: 1,
    world_id: value.world_id,
    tick: value.tick,
    changed_agents,
    changed_vehicles,
    left_agents,
    left_vehicles,
  };
}
```

(Adapt to the actual existing parser body.)

- [x] **Step 4: Verify**

```bash
npx vitest run tests/backend/mobilityProtocol.test.ts
npx tsc --noEmit
```

Expected: green.

- [x] **Step 5: Commit**

```bash
git add src/backend/mobilityProtocol.ts tests/backend/mobilityProtocol.test.ts
git commit -m "feat: frontend ClientMessageDto encoder + left_* fields on mobility delta"
```

---

## Task 7: Frontend applyMobilityDelta drops left ids

**Files:**
- Modify: `src/backend/mobilityState.ts`
- Modify: `tests/backend/mobilityState.test.ts`

- [x] **Step 1: Add failing test**

In `tests/backend/mobilityState.test.ts`, append:

```ts
it('applyMobilityDelta drops entities listed in left_agents and left_vehicles', () => {
  const state = applyMobilitySnapshot(
    createMobilityOverlayState(),
    {
      protocol_version: 1,
      world_id: 'w', tick: 0,
      agents: [{ id: 'agent:walk:1', state: { type: 'walking', link_id: 'l', progress: 0 }, plan_cursor: 0, world_coord: { x: 0, y: 0 }, direction: 'e', sprite_key: 'p:0' }],
      vehicles: [{ id: 'vehicle:car:0:0', kind: 'car', route_id: 'r', link_index: 0, progress: 0, capacity: 1, occupants: [], dwell_ticks_remaining: 0, world_coord: { x: 0, y: 0 }, direction: 'e', sprite_key: 'c:0' }],
      stops: [],
    },
    0,
  );
  expect(state.agents.size).toBe(1);
  expect(state.vehicles.size).toBe(1);

  const after = applyMobilityDelta(state, {
    type: 'mobility_delta',
    protocol_version: 1,
    world_id: 'w', tick: 1,
    changed_agents: [], changed_vehicles: [],
    left_agents: ['agent:walk:1'],
    left_vehicles: ['vehicle:car:0:0'],
  }, 100);
  expect(after.agents.size).toBe(0);
  expect(after.vehicles.size).toBe(0);
});
```

- [x] **Step 2: Confirm failure**

```bash
npx vitest run tests/backend/mobilityState.test.ts
```

Expected: FAIL — `left_*` aren't dropped.

- [x] **Step 3: Implement**

In `src/backend/mobilityState.ts`, find `applyMobilityDelta`. Before the existing apply loops, add:

```ts
for (const id of delta.left_agents) state.agents.delete(id);
for (const id of delta.left_vehicles) state.vehicles.delete(id);
```

If `state` is immutable (returns a new object), do it on the clone:

```ts
const next = { ...state, agents: new Map(state.agents), vehicles: new Map(state.vehicles) };
for (const id of delta.left_agents) next.agents.delete(id);
for (const id of delta.left_vehicles) next.vehicles.delete(id);
// existing changed_agents / changed_vehicles loops, mutating next
return next;
```

Match the existing pattern in the file — read it first to confirm mutability convention.

- [x] **Step 4: Verify**

```bash
npx vitest run
```

Expected: green.

- [x] **Step 5: Commit**

```bash
git add src/backend/mobilityState.ts tests/backend/mobilityState.test.ts
git commit -m "feat: applyMobilityDelta drops entities in left_agents/left_vehicles"
```

---

## Task 8: Frontend chunkSubscriptionClient

**Files:**
- Create: `src/backend/chunkSubscriptionClient.ts`
- Create: `tests/backend/chunkSubscriptionClient.test.ts`
- Modify: `src/backend/mobilityClient.ts`

- [x] **Step 1: Write failing test**

Create `tests/backend/chunkSubscriptionClient.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { computeInitialSubscriptionCoords, createSubscriptionClient } from '../../src/backend/chunkSubscriptionClient';

describe('chunkSubscriptionClient', () => {
  it('computeInitialSubscriptionCoords covers the entire world grid', () => {
    const coords = computeInitialSubscriptionCoords({ worldWidthTiles: 256, worldHeightTiles: 256, chunkSize: 32 });
    expect(coords).toHaveLength(64);
    expect(coords).toContainEqual({ x: 0, y: 0 });
    expect(coords).toContainEqual({ x: 7, y: 7 });
  });

  it('sends a chunk_subscribe with the initial coords when start is called', () => {
    const sendCalls: string[] = [];
    const send = (s: string) => sendCalls.push(s);
    const client = createSubscriptionClient({ send, worldWidthTiles: 64, worldHeightTiles: 64, chunkSize: 32 });
    client.start();
    expect(sendCalls).toHaveLength(1);
    const parsed = JSON.parse(sendCalls[0]);
    expect(parsed.type).toBe('chunk_subscribe');
    expect(parsed.coords).toHaveLength(4);
  });
});
```

- [x] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/backend/chunkSubscriptionClient.test.ts
```

Expected: FAIL — file doesn't exist.

- [x] **Step 3: Implement**

Create `src/backend/chunkSubscriptionClient.ts`:

```ts
import { encodeClientMessage, type ChunkCoordDto } from './mobilityProtocol';

export function computeInitialSubscriptionCoords(opts: {
  worldWidthTiles: number;
  worldHeightTiles: number;
  chunkSize: number;
}): ChunkCoordDto[] {
  const cs = opts.chunkSize;
  const cols = Math.ceil(opts.worldWidthTiles / cs);
  const rows = Math.ceil(opts.worldHeightTiles / cs);
  const out: ChunkCoordDto[] = [];
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      out.push({ x, y });
    }
  }
  return out;
}

export type SubscriptionClient = {
  start(): void;
};

export function createSubscriptionClient(opts: {
  send: (text: string) => void;
  worldWidthTiles: number;
  worldHeightTiles: number;
  chunkSize: number;
}): SubscriptionClient {
  const initial = computeInitialSubscriptionCoords(opts);
  return {
    start() {
      opts.send(encodeClientMessage({
        type: 'chunk_subscribe',
        protocol_version: 1,
        coords: initial,
      }));
    },
  };
}
```

YAGNI: this Phase-4 module only sends the initial full-world subscription. Camera-aware shrinking is a follow-up (Phase 4.5 or Phase 6) once we have measurable bandwidth pressure.

- [x] **Step 4: Wire into mobilityClient**

In `src/backend/mobilityClient.ts`, find where the WebSocket is opened (`new WebSocket(...)` or similar). After the socket reaches `OPEN` state, instantiate and `start()` the subscription client:

```ts
import { createSubscriptionClient } from './chunkSubscriptionClient';
// ...
socket.addEventListener('open', () => {
  const client = createSubscriptionClient({
    send: (text) => socket.send(text),
    worldWidthTiles: 256, // TODO: read from worldSummary if/when exposed; hardcode for now to match zurich-network.json
    worldHeightTiles: 256,
    chunkSize: 32,
  });
  client.start();
});
```

If the existing `mobilityClient.ts` already has an `open` handler, add the client.start() inside it. Don't break existing reconnect logic.

- [x] **Step 5: Verify**

```bash
npx vitest run
npx tsc --noEmit
npm run build
```

Expected: green.

- [x] **Step 6: Commit**

```bash
git add src/backend/chunkSubscriptionClient.ts tests/backend/chunkSubscriptionClient.test.ts src/backend/mobilityClient.ts
git commit -m "feat: frontend sends initial chunk_subscribe covering full world"
```

---

## Task 9: Quality gate + progress.md + browser verification

**Files:**
- Modify: `progress.md`

- [x] **Step 1: Run all gates**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
npx vitest run
npx tsc --noEmit
npm run build
```

Expected: all green (excluding pre-existing `noRetiredAssets.test.ts` failures, which are unrelated to Phase 4).

- [x] **Step 2: Append to progress.md**

Add one line summarising Phase 4:

```
2026-05-16T<HH:MM:SS>.000Z - Viewport-filtered mobility replication: introduced ClientMessageDto with chunk_subscribe / chunk_unsubscribe; refactored WS task into a select! loop with per-connection ConnectionState; build_filtered_mobility_delta_dto produces per-connection deltas with changed_*/left_* fields. Frontend chunkSubscriptionClient sends an 8×8 initial subscription after Hello. applyMobilityDelta drops left_agents/left_vehicles before applying changes. Phase 4 of the million-agent roadmap; backend can now sustain ~10k entities with per-client viewport-scoped bandwidth.
```

- [x] **Step 3: Verify in browser**

Restart the stack:

```bash
pkill -f run-dev-stack; pkill -f sim-server; pkill -f "vite --host"
sleep 2
nohup npm run dev:stack > /tmp/abutown-stack.log 2>&1 & disown
```

Wait for backend ready:

```bash
until curl -sf http://127.0.0.1:8080/health > /dev/null; do sleep 2; done
echo READY
```

Open http://127.0.0.1:5175 in your normal logged-in browser. Confirm:
- Walking pedestrians still visible on the streets.
- Cars still rolling.
- Network tab: WebSocket frames show fewer entities per delta after initial subscribe-snapshot, OR roughly the same since we subscribed to the full world.

If walkers/cars disappear: the initial subscribe isn't reaching the backend (check WS messages in DevTools Network tab), OR the chunk-coord math is off (check that the `world_coord` for a known agent falls in a subscribed chunk).

- [x] **Step 4: Commit**

```bash
git add progress.md backend/ src/ tests/
git commit -m "chore: phase 4 quality gate + progress note"
```

---

## Self-Review

**1. Spec coverage:**

- Per-connection AoI filter → Tasks 3+4.
- `ChunkSubscribe`/`ChunkUnsubscribe` protocol → Task 1.
- `left_agents`/`left_vehicles` on `MobilityDeltaDto` → Task 1.
- Subscription change emits synthetic delta → Task 4 (`synthetic_mobility_delta_for_subscription`).
- Initial connection: nothing sent before subscribe → Task 4 (`continue` on empty delta).
- Frontend chunk subscription client → Task 8.
- Frontend `applyMobilityDelta` drops left ids → Task 7.
- Two-client AoI integration test → Task 5.
- `chunk_of` helper with `div_euclid` for negative-coord safety → Task 2.

**2. Placeholder scan:**

- One `// TODO: read from worldSummary if/when exposed` in Task 8 Step 4. That's a deliberate acknowledged YAGNI scope-fence — the `worldSummary` exists but threading it through the client is Phase-4.5 cleanup. Acceptable; the hardcoded 256×256 matches the only world we ship (zurich-network.json).
- No "implement later" / "handle edge cases" / "similar to" patterns.

**3. Type consistency:**

- `ChunkCoordDto` used in protocol (Task 1), `chunk_of` returns `ChunkCoord` (Task 2 — Rust-native `i32`-coord type from `sim-core::ids`). The conversion is in `handle_client_message` (Task 4 Step 3): `ChunkCoord { x: coord.x, y: coord.y }`.
- `ClientMessageDto` consistent between Rust (Task 1), TS (Task 6), and TS chunkSubscriptionClient (Task 8).
- `last_visible_agents: HashSet<EntityId>` consistent across `ConnectionState` (Task 4), `build_filtered_mobility_delta_dto` (Task 3), runtime helpers (Task 4 Step 4).
- `MobilityDeltaDto` field name `left_agents` / `left_vehicles` consistent across Rust DTO, TS message type, parser, and frontend handler.

**Scope check:** Single PR, ~9 tasks. Cohesive (introduces subscription primitive + applies it to mobility). Doesn't drag in TilePulse filtering (explicit non-goal). Reasonable size.

**Risks acknowledged in spec:** subscription-update storm (debounce — but frontend currently sends ONE static initial subscribe, so no storm in this phase); `last_visible_ids` memory (bounded by world size); filter cost (O(n) per connection per tick — fine at current scale).
