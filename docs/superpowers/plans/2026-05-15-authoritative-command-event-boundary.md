# Authoritative Command Event Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This checklist is historical; `progress.md` and later plans are authoritative for current implementation status.

**Goal:** Add the first Rust-authoritative mutation path: a client command is validated, applied to hot chunk state, stored as an in-memory append-only event, and broadcast over `/ws`.

**Architecture:** Keep browser input indirect and server-authoritative. Add JSON command DTOs to `abutown-protocol`, an in-memory event store in `sim-core`, command validation/application in `sim-server`, and `POST /commands` as the first testable ingress. Supabase/Postgres is intentionally not part of this slice; the event boundary is shaped so a durable adapter can replace or mirror the in-memory store later.

**Tech Stack:** Rust 2024, Axum, Tokio broadcast, serde JSON DTOs, existing dense chunk storage, existing snapshot loop.

---

## Scope

This plan implements one small mutation command: set one tile's kind in one already-loaded chunk.

It does not implement authentication, player accounts, command idempotency, command permissions, Supabase/Postgres, chunk loading, chunk unload, recovery from restart, frontend command UI, bidirectional WebSocket commands, or full interest management.

## Current Baseline

Relevant implemented backend surface:

- `GET /health`, `GET /world`, `GET /chunks/{x}/{y}`, `GET /mobility`
- `GET /ws` streaming `hello`, `tile_pulse`, and `mobility_delta`
- `Chunk::set_tile_kind()` tracks dirty tiles and increments chunk-local tile versions.
- `ChunkRegistry::write_snapshots()` writes loaded chunks and clears dirty flags.
- `SimulationRuntime` owns `ChunkRegistry`, `MobilityWorld`, and `InMemoryChunkSnapshotStore`.

Relevant docs:

- `docs/superpowers/specs/2026-05-14-abutown-simulation-architecture-v2-design.md` requires validated indirect actions, append-only mutation events, dirty chunk snapshots, and Rust authority.
- `docs/superpowers/plans/2026-05-14-backend-persistence-snapshot-loop.md` explicitly did not implement player-driven mutation APIs.
- `docs/superpowers/plans/2026-05-14-agent-mobility-foundation.md` explicitly did not implement player commands.

## File Structure

- Modify `backend/crates/protocol/src/lib.rs`
  - Add command DTOs.
  - Add command response DTOs.
  - Add append-only world event DTOs.
  - Add `ServerMessageDto::WorldEvent`.
- Create `backend/crates/sim-core/src/events.rs`
  - Add `InMemoryWorldEventStore` helpers.
- Modify `backend/crates/sim-core/src/lib.rs`
  - Export `events`.
- Modify `backend/crates/sim-core/src/tile.rs`
  - Add `From<TileKindDto> for TileKind`.
- Create `backend/crates/sim-server/src/commands.rs`
  - Add command application result and rejection types.
- Modify `backend/crates/sim-server/src/lib.rs`
  - Export `commands` internally.
- Modify `backend/crates/sim-server/src/chunk_registry.rs`
  - Add `set_tile_kind()` for loaded chunks.
- Modify `backend/crates/sim-server/src/runtime.rs`
  - Own an `InMemoryWorldEventStore`.
  - Apply commands through runtime validation.
  - Build event IDs and append events.
- Modify `backend/crates/sim-server/src/app.rs`
  - Add `POST /commands`.
  - Broadcast accepted world events to `/ws` subscribers.
- Modify `backend/crates/sim-server/tests/http.rs`
  - Add accepted and rejected command tests.
- Modify `backend/crates/sim-server/tests/websocket.rs`
  - Add command broadcast test.
- Modify `backend/README.md`
  - Document the command/event boundary and targeted commands.

---

### Task 1: Add Protocol DTOs For Commands And Events

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`

- [x] **Step 1: Add failing protocol serialization tests**

Append these tests inside the existing `#[cfg(test)] mod tests` block in `backend/crates/protocol/src/lib.rs`:

```rust
    #[test]
    fn client_set_tile_kind_command_serializes_with_type_tag() {
        let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            command_id: "command:test:1".to_string(),
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 11,
            kind: TileKindDto::Water,
        });

        let json = serde_json::to_string(&command).expect("command serializes");

        assert_eq!(
            json,
            r#"{"type":"set_tile_kind","protocol_version":1,"world_id":"abutown-main","command_id":"command:test:1","coord":{"x":4,"y":4},"local_index":11,"kind":"water"}"#
        );
    }

    #[test]
    fn accepted_command_response_serializes_event() {
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event:1".to_string(),
            command_id: "command:test:1".to_string(),
            world_id: WorldId("abutown-main".to_string()),
            tick: 0,
            version: 1,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 11,
            kind: TileKindDto::Water,
        });
        let response = CommandResponseDto::Accepted(CommandAcceptedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            command_id: "command:test:1".to_string(),
            event,
        });

        let json = serde_json::to_value(&response).expect("accepted response serializes");

        assert_eq!(json["status"], "accepted");
        assert_eq!(json["event"]["type"], "tile_kind_set");
        assert_eq!(json["event"]["event_id"], "event:1");
        assert_eq!(json["event"]["kind"], "water");
    }

    #[test]
    fn rejected_command_response_serializes_reason() {
        let response = CommandResponseDto::Rejected(CommandRejectedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: Some(WorldId("abutown-main".to_string())),
            command_id: Some("command:test:2".to_string()),
            code: "chunk_not_loaded".to_string(),
            message: "chunk 9:9 is not loaded".to_string(),
        });

        let json = serde_json::to_value(&response).expect("rejected response serializes");

        assert_eq!(json["status"], "rejected");
        assert_eq!(json["world_id"], "abutown-main");
        assert_eq!(json["command_id"], "command:test:2");
        assert_eq!(json["code"], "chunk_not_loaded");
    }

    #[test]
    fn websocket_world_event_serializes_with_outer_type_tag() {
        let message = ServerMessageDto::WorldEvent {
            event: WorldEventDto::TileKindSet(TileKindSetEventDto {
                protocol_version: PROTOCOL_VERSION,
                event_id: "event:2".to_string(),
                command_id: "command:test:3".to_string(),
                world_id: WorldId("abutown-main".to_string()),
                tick: 4,
                version: 8,
                coord: ChunkCoordDto { x: 5, y: 4 },
                local_index: 23,
                kind: TileKindDto::Road,
            }),
        };

        let json = serde_json::to_value(&message).expect("world event message serializes");

        assert_eq!(json["type"], "world_event");
        assert_eq!(json["event"]["type"], "tile_kind_set");
        assert_eq!(json["event"]["version"], 8);
        assert_eq!(json["event"]["coord"]["x"], 5);
    }
```

- [x] **Step 2: Run the failing protocol tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol command_
```

Expected: FAIL because the command, response, and event DTOs do not exist.

- [x] **Step 3: Add protocol DTOs**

In `backend/crates/protocol/src/lib.rs`, add these definitions after `ChunkSnapshotDto` and before `ServerMessageDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommandDto {
    SetTileKind(SetTileKindCommandDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetTileKindCommandDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub command_id: String,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
    pub kind: TileKindDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandResponseDto {
    Accepted(CommandAcceptedDto),
    Rejected(CommandRejectedDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandAcceptedDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub command_id: String,
    pub event: WorldEventDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRejectedDto {
    pub protocol_version: u16,
    pub world_id: Option<WorldId>,
    pub command_id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorldEventDto {
    TileKindSet(TileKindSetEventDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileKindSetEventDto {
    pub protocol_version: u16,
    pub event_id: String,
    pub command_id: String,
    pub world_id: WorldId,
    pub tick: u64,
    pub version: u64,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
    pub kind: TileKindDto,
}
```

Then extend `ServerMessageDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    MobilityDelta(MobilityDeltaDto),
    WorldEvent {
        event: WorldEventDto,
    },
    Error(ServerErrorDto),
}
```

- [x] **Step 4: Run protocol tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol command_
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol websocket_
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol mobility_
```

Expected: all commands PASS.

- [x] **Step 5: Commit**

```bash
git add backend/crates/protocol/src/lib.rs
git commit -m "feat: add authoritative command protocol"
```

---

### Task 2: Add In-Memory World Event Store

**Files:**
- Create: `backend/crates/sim-core/src/events.rs`
- Modify: `backend/crates/sim-core/src/lib.rs`

- [x] **Step 1: Write failing event store tests**

Create `backend/crates/sim-core/src/events.rs` with this content:

```rust
use abutown_protocol::{
    ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_event(event_id: &str, version: u64) -> WorldEventDto {
        WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: event_id.to_string(),
            command_id: format!("command:{event_id}"),
            world_id: WorldId("abutown-main".to_string()),
            tick: version,
            version,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 3,
            kind: TileKindDto::Road,
        })
    }

    #[test]
    fn event_store_appends_events_in_order() {
        let mut store = InMemoryWorldEventStore::default();

        store.append(tile_event("event:1", 1));
        store.append(tile_event("event:2", 2));

        assert_eq!(store.event_count(), 2);
        assert_eq!(store.events()[0], tile_event("event:1", 1));
        assert_eq!(store.events()[1], tile_event("event:2", 2));
    }
}
```

Also add this line to `backend/crates/sim-core/src/lib.rs`:

```rust
pub mod events;
```

- [x] **Step 2: Run the failing event store test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core event_store_appends_events_in_order
```

Expected: FAIL because `InMemoryWorldEventStore` does not exist.

- [x] **Step 3: Implement the event store**

Replace `backend/crates/sim-core/src/events.rs` with:

```rust
use abutown_protocol::WorldEventDto;

#[derive(Debug, Default)]
pub struct InMemoryWorldEventStore {
    events: Vec<WorldEventDto>,
}

impl InMemoryWorldEventStore {
    pub fn append(&mut self, event: WorldEventDto) {
        self.events.push(event);
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn events(&self) -> &[WorldEventDto] {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abutown_protocol::{
        ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId,
    };

    fn tile_event(event_id: &str, version: u64) -> WorldEventDto {
        WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id: event_id.to_string(),
            command_id: format!("command:{event_id}"),
            world_id: WorldId("abutown-main".to_string()),
            tick: version,
            version,
            coord: ChunkCoordDto { x: 4, y: 4 },
            local_index: 3,
            kind: TileKindDto::Road,
        })
    }

    #[test]
    fn event_store_appends_events_in_order() {
        let mut store = InMemoryWorldEventStore::default();

        store.append(tile_event("event:1", 1));
        store.append(tile_event("event:2", 2));

        assert_eq!(store.event_count(), 2);
        assert_eq!(store.events()[0], tile_event("event:1", 1));
        assert_eq!(store.events()[1], tile_event("event:2", 2));
    }
}
```

- [x] **Step 4: Run sim-core tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core events
cargo test --manifest-path backend/Cargo.toml -p sim-core
```

Expected: both commands PASS.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/events.rs backend/crates/sim-core/src/lib.rs
git commit -m "feat: add in-memory world event store"
```

---

### Task 3: Add Registry Tile Mutation Boundary

**Files:**
- Modify: `backend/crates/sim-core/src/tile.rs`
- Modify: `backend/crates/sim-server/src/chunk_registry.rs`

- [x] **Step 1: Add failing registry mutation tests**

In `backend/crates/sim-core/src/tile.rs`, add `TileKindDto` conversion support test at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_kind_converts_from_protocol_kind() {
        assert_eq!(TileKind::from(TileKindDto::Grass), TileKind::Grass);
        assert_eq!(TileKind::from(TileKindDto::Water), TileKind::Water);
        assert_eq!(TileKind::from(TileKindDto::Road), TileKind::Road);
        assert_eq!(
            TileKind::from(TileKindDto::BuildingFootprint),
            TileKind::BuildingFootprint
        );
    }
}
```

In `backend/crates/sim-server/src/chunk_registry.rs`, add this test after `registry_reports_loaded_tile_counts`:

```rust
    #[test]
    fn registry_sets_tile_kind_on_loaded_chunk() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let version = registry
            .set_tile_kind(ChunkCoord { x: 4, y: 4 }, 11, TileKind::Water)
            .expect("loaded tile can mutate");

        assert_eq!(version, 2);
        let snapshot = registry
            .chunk_snapshot(&WorldId("abutown-main".to_string()), ChunkCoord { x: 4, y: 4 })
            .expect("chunk snapshot exists");
        assert_eq!(snapshot.dirty_tiles.len(), 2);
        assert_eq!(snapshot.dirty_tiles[1].local_index, 11);
        assert_eq!(
            snapshot.dirty_tiles[1].kind,
            abutown_protocol::TileKindDto::Water
        );
    }

    #[test]
    fn registry_rejects_missing_chunk_mutation() {
        let mut registry = ChunkRegistry::new(32);

        assert!(matches!(
            registry.set_tile_kind(ChunkCoord { x: 9, y: 9 }, 0, TileKind::Road),
            Err(ChunkMutationError::ChunkNotLoaded { coord }) if coord == ChunkCoord { x: 9, y: 9 }
        ));
    }

    #[test]
    fn registry_rejects_out_of_bounds_tile_mutation() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        assert!(matches!(
            registry.set_tile_kind(ChunkCoord { x: 4, y: 4 }, 2000, TileKind::Water),
            Err(ChunkMutationError::TileOutOfBounds { index: 2000, tile_count: 1024 })
        ));
    }
```

- [x] **Step 2: Run failing mutation tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core tile_kind_converts_from_protocol_kind
cargo test --manifest-path backend/Cargo.toml -p sim-server registry_sets_tile_kind_on_loaded_chunk
```

Expected: first command FAILS because `From<TileKindDto>` is missing; second command FAILS because `ChunkRegistry::set_tile_kind` and `ChunkMutationError` are missing.

- [x] **Step 3: Implement tile kind conversion**

In `backend/crates/sim-core/src/tile.rs`, add this impl after the existing `impl From<TileKind> for TileKindDto`:

```rust
impl From<TileKindDto> for TileKind {
    fn from(value: TileKindDto) -> Self {
        match value {
            TileKindDto::Grass => Self::Grass,
            TileKindDto::Water => Self::Water,
            TileKindDto::Road => Self::Road,
            TileKindDto::BuildingFootprint => Self::BuildingFootprint,
        }
    }
}
```

- [x] **Step 4: Implement registry mutation**

In `backend/crates/sim-server/src/chunk_registry.rs`, add `use sim_core::chunk::ChunkError;` and `use sim_core::tile::TileKind;` to the imports.

Add this error type above `LoadedChunk`:

```rust
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChunkMutationError {
    ChunkNotLoaded { coord: ChunkCoord },
    TileOutOfBounds { index: u16, tile_count: u16 },
}
```

Add this method to `impl ChunkRegistry` after `tile_count`:

```rust
pub(crate) fn set_tile_kind(
    &mut self,
    coord: ChunkCoord,
    local_index: u16,
    kind: TileKind,
) -> Result<u64, ChunkMutationError> {
    let loaded = self
        .chunks
        .get_mut(&coord)
        .ok_or(ChunkMutationError::ChunkNotLoaded { coord })?;

    loaded
        .chunk
        .set_tile_kind(local_index, kind)
        .map_err(|error| match error {
            ChunkError::IndexOutOfBounds { index, tile_count } => {
                ChunkMutationError::TileOutOfBounds { index, tile_count }
            }
            ChunkError::InvalidChunkSize { .. } => unreachable!("loaded chunks are already valid"),
        })?;

    Ok(loaded.chunk.version())
}
```

- [x] **Step 5: Run registry and sim-core tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core tile_kind_converts_from_protocol_kind
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry
```

Expected: both commands PASS.

- [x] **Step 6: Commit**

```bash
git add backend/crates/sim-core/src/tile.rs backend/crates/sim-server/src/chunk_registry.rs
git commit -m "feat: add loaded chunk mutation boundary"
```

---

### Task 4: Apply Commands In Runtime And Append Events

**Files:**
- Create: `backend/crates/sim-server/src/commands.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Add failing runtime command tests**

Create `backend/crates/sim-server/src/commands.rs`:

```rust
use abutown_protocol::{CommandRejectedDto, PROTOCOL_VERSION, WorldId};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AppliedCommand {
    pub response: abutown_protocol::CommandAcceptedDto,
    pub event: abutown_protocol::WorldEventDto,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommandRejection {
    pub world_id: Option<WorldId>,
    pub command_id: Option<String>,
    pub code: &'static str,
    pub message: String,
}

impl CommandRejection {
    pub(crate) fn into_dto(self) -> CommandRejectedDto {
        CommandRejectedDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id,
            command_id: self.command_id,
            code: self.code.to_string(),
            message: self.message,
        }
    }
}
```

Add this line to `backend/crates/sim-server/src/lib.rs`:

```rust
pub(crate) mod commands;
```

In `backend/crates/sim-server/src/runtime.rs`, add these tests inside the existing `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn runtime_applies_set_tile_kind_command_and_appends_event() {
        let mut runtime = SimulationRuntime::new();

        let applied = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:test:1".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .expect("command should apply");

        let abutown_protocol::WorldEventDto::TileKindSet(event) = &applied.event;
        assert_eq!(event.event_id, "event:1");
        assert_eq!(event.command_id, "command:test:1");
        assert_eq!(event.version, 2);
        assert_eq!(event.local_index, 11);
        assert_eq!(event.kind, abutown_protocol::TileKindDto::Water);
        assert_eq!(runtime.event_count(), 1);

        let snapshot = runtime
            .chunk_snapshot(sim_core::ids::ChunkCoord { x: 4, y: 4 })
            .expect("mutated chunk snapshot exists");
        assert!(snapshot.dirty_tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }

    #[test]
    fn runtime_rejects_commands_for_other_worlds() {
        let mut runtime = SimulationRuntime::new();

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("other-world".to_string()),
                    command_id: "command:test:2".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .expect_err("wrong world should reject");

        assert_eq!(rejection.code, "wrong_world");
        assert_eq!(runtime.event_count(), 0);
    }

    #[test]
    fn runtime_rejects_commands_for_unloaded_chunks() {
        let mut runtime = SimulationRuntime::new();

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:test:3".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 9, y: 9 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .expect_err("unloaded chunk should reject");

        assert_eq!(rejection.code, "chunk_not_loaded");
        assert_eq!(runtime.event_count(), 0);
    }
```

- [x] **Step 2: Run failing runtime command test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_applies_set_tile_kind_command_and_appends_event
```

Expected: FAIL because runtime command application methods and event store fields are missing.

- [x] **Step 3: Add runtime event store fields and helpers**

In `backend/crates/sim-server/src/runtime.rs`, update imports:

```rust
use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, ClientCommandDto, CommandAcceptedDto, HealthResponse,
    MobilityDeltaDto, MobilitySnapshotDto, PROTOCOL_VERSION, ServerHelloDto, ServerMessageDto,
    SetTileKindCommandDto, TileKindSetEventDto, TilePulseDeltaDto, WorldEventDto, WorldId,
    WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk,
    events::InMemoryWorldEventStore,
    ids::ChunkCoord,
    mobility::{MobilityWorld, build_mobility_delta_dto, build_mobility_snapshot_dto},
    persistence::InMemoryChunkSnapshotStore,
    scheduler::ChunkActivity,
    tile::TileKind,
};

use crate::{
    chunk_registry::{ChunkMutationError, ChunkRegistry},
    commands::{AppliedCommand, CommandRejection},
};
```

Add fields to `SimulationRuntime`:

```rust
event_store: InMemoryWorldEventStore,
next_event_id: u64,
```

Initialize them in `SimulationRuntime::new()`:

```rust
event_store: InMemoryWorldEventStore::default(),
next_event_id: 1,
```

Add this helper near `stored_chunk_snapshot`:

```rust
pub fn event_count(&self) -> usize {
    self.event_store.event_count()
}
```

- [x] **Step 4: Implement command application**

Add these methods to `impl SimulationRuntime` after `stored_chunk_snapshot`:

```rust
pub(crate) fn apply_client_command(
    &mut self,
    command: ClientCommandDto,
) -> Result<AppliedCommand, CommandRejection> {
    match command {
        ClientCommandDto::SetTileKind(command) => self.apply_set_tile_kind(command),
    }
}

fn apply_set_tile_kind(
    &mut self,
    command: SetTileKindCommandDto,
) -> Result<AppliedCommand, CommandRejection> {
    if command.protocol_version != PROTOCOL_VERSION {
        return Err(CommandRejection {
            world_id: Some(command.world_id),
            command_id: Some(command.command_id),
            code: "protocol_mismatch",
            message: format!(
                "protocol version {} is not supported by server version {}",
                command.protocol_version, PROTOCOL_VERSION
            ),
        });
    }

    if command.world_id != self.world_id {
        return Err(CommandRejection {
            world_id: Some(command.world_id),
            command_id: Some(command.command_id),
            code: "wrong_world",
            message: format!("command targets a different world than {}", self.world_id.0),
        });
    }

    let coord = ChunkCoord {
        x: command.coord.x,
        y: command.coord.y,
    };
    let kind = TileKind::from(command.kind);
    let version = self
        .registry
        .set_tile_kind(coord, command.local_index, kind)
        .map_err(|error| match error {
            ChunkMutationError::ChunkNotLoaded { coord } => CommandRejection {
                world_id: Some(command.world_id.clone()),
                command_id: Some(command.command_id.clone()),
                code: "chunk_not_loaded",
                message: format!("chunk {}:{} is not loaded", coord.x, coord.y),
            },
            ChunkMutationError::TileOutOfBounds { index, tile_count } => CommandRejection {
                world_id: Some(command.world_id.clone()),
                command_id: Some(command.command_id.clone()),
                code: "tile_out_of_bounds",
                message: format!("tile index {index} is outside chunk tile count {tile_count}"),
            },
        })?;

    let event_id = format!("event:{}", self.next_event_id);
    self.next_event_id += 1;
    let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
        protocol_version: PROTOCOL_VERSION,
        event_id,
        command_id: command.command_id.clone(),
        world_id: self.world_id.clone(),
        tick: self.tick,
        version,
        coord: command.coord,
        local_index: command.local_index,
        kind: command.kind,
    });
    self.event_store.append(event.clone());

    let response = CommandAcceptedDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: self.world_id.clone(),
        command_id: command.command_id,
        event: event.clone(),
    };

    Ok(AppliedCommand { response, event })
}
```

- [x] **Step 5: Run runtime command tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_applies_set_tile_kind_command_and_appends_event
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_rejects_commands
```

Expected: both commands PASS.

- [x] **Step 6: Run sim-server unit tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --lib
```

Expected: PASS.

- [x] **Step 7: Commit**

```bash
git add backend/crates/sim-server/src/commands.rs backend/crates/sim-server/src/lib.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat: apply authoritative runtime commands"
```

---

### Task 5: Add `POST /commands` HTTP Ingress

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/http.rs`

- [x] **Step 1: Add failing HTTP command tests**

In `backend/crates/sim-server/tests/http.rs`, add imports:

```rust
use abutown_protocol::{ClientCommandDto, PROTOCOL_VERSION, SetTileKindCommandDto, TileKindDto, WorldId};
```

Append these tests:

```rust
#[tokio::test]
async fn command_sets_tile_kind_and_returns_event() {
    let app = build_app();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:http:1".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 11,
        kind: TileKindDto::Water,
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&command).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "accepted");
    assert_eq!(json["event"]["type"], "tile_kind_set");
    assert_eq!(json["event"]["command_id"], "command:http:1");
    assert_eq!(json["event"]["local_index"], 11);
    assert_eq!(json["event"]["kind"], "water");

    let snapshot_response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot_response.status(), StatusCode::OK);
    let body = snapshot_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let snapshot: Value = serde_json::from_slice(&body).unwrap();
    assert!(snapshot["dirty_tiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tile| tile["local_index"] == 11 && tile["kind"] == "water"));
}

#[tokio::test]
async fn command_rejects_unloaded_chunk() {
    let app = build_app();
    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:http:2".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 9, y: 9 },
        local_index: 11,
        kind: TileKindDto::Water,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&command).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "rejected");
    assert_eq!(json["code"], "chunk_not_loaded");
    assert_eq!(json["command_id"], "command:http:2");
}
```

- [x] **Step 2: Run failing HTTP command test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http command_sets_tile_kind_and_returns_event
```

Expected: FAIL because `/commands` is not routed.

- [x] **Step 3: Implement app route and handler**

In `backend/crates/sim-server/src/app.rs`, update imports:

```rust
use abutown_protocol::{
    ChunkSnapshotDto, ClientCommandDto, CommandResponseDto, HealthResponse, MobilitySnapshotDto,
    ServerMessageDto, WorldSummaryDto,
};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
```

Add the route in `build_app_with_runtime()`:

```rust
.route("/commands", post(command))
```

Add this handler after `chunk`:

```rust
async fn command(
    State(state): State<AppState>,
    Json(command): Json<ClientCommandDto>,
) -> Response {
    let result = {
        let runtime = state.runtime();
        let mut runtime = runtime.lock().await;
        runtime.apply_client_command(command)
    };

    match result {
        Ok(applied) => {
            let _ = state
                .deltas
                .send(ServerMessageDto::WorldEvent {
                    event: applied.event.clone(),
                });
            (
                StatusCode::OK,
                Json(CommandResponseDto::Accepted(applied.response)),
            )
                .into_response()
        }
        Err(rejection) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(CommandResponseDto::Rejected(rejection.into_dto())),
        )
            .into_response(),
    }
}
```

- [x] **Step 4: Run HTTP command tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http command_
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http
```

Expected: both commands PASS.

- [x] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: expose authoritative command ingress"
```

---

### Task 6: Broadcast Accepted Commands Over WebSocket

**Files:**
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [x] **Step 1: Add failing WebSocket command broadcast test**

In `backend/crates/sim-server/tests/websocket.rs`, update imports:

```rust
use abutown_protocol::{
    ClientCommandDto, PROTOCOL_VERSION, ServerMessageDto, SetTileKindCommandDto, TileKindDto,
    TilePulseDeltaDto, WorldEventDto, WorldId,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
```

Append this test before `read_server_message`:

```rust
#[tokio::test]
async fn websocket_broadcasts_accepted_command_event() {
    let app = build_app();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let command = ClientCommandDto::SetTileKind(SetTileKindCommandDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        command_id: "command:ws:1".to_string(),
        coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
        local_index: 12,
        kind: TileKindDto::BuildingFootprint,
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/commands")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&command).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.into_body().collect().await.unwrap();

    loop {
        let message = read_server_message(&mut stream).await;
        if let ServerMessageDto::WorldEvent {
            event: WorldEventDto::TileKindSet(event),
        } = message
        {
            assert_eq!(event.command_id, "command:ws:1");
            assert_eq!(event.coord, abutown_protocol::ChunkCoordDto { x: 4, y: 4 });
            assert_eq!(event.local_index, 12);
            assert_eq!(event.kind, TileKindDto::BuildingFootprint);
            break;
        }
    }

    server.abort();
}
```

- [x] **Step 2: Run WebSocket command broadcast test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket websocket_broadcasts_accepted_command_event
```

Expected: PASS because Task 5 broadcasts accepted events through `ServerMessageDto::WorldEvent`.

- [x] **Step 3: Run all WebSocket tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: PASS.

- [x] **Step 4: Commit**

```bash
git add backend/crates/sim-server/tests/websocket.rs
git commit -m "test: cover command event websocket broadcast"
```

---

### Task 7: Document And Verify The Backend Slice

**Files:**
- Modify: `backend/README.md`

- [x] **Step 1: Update backend README**

Add this section to `backend/README.md` after `## Runtime Surface`:

```markdown
## Command Event Boundary

The first mutation ingress is `POST /commands`. It accepts versioned JSON client commands, validates them inside the Rust runtime, applies accepted changes to loaded hot state, appends an in-memory world event, and broadcasts that event to `/ws` subscribers.

Implemented command:

- `set_tile_kind`: changes one tile in one already-loaded chunk.

Current boundaries:

- Commands are unauthenticated local-development inputs.
- Commands only target loaded chunks.
- Accepted mutations are stored in an in-memory append-only event store.
- Supabase/Postgres, command idempotency, permissions, chunk loading, and recovery remain later slices.
```

Add this command to the targeted commands list:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol command_
cargo test --manifest-path backend/Cargo.toml -p sim-core events
cargo test --manifest-path backend/Cargo.toml -p sim-server command_
```

- [x] **Step 2: Run backend formatting**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

Expected: PASS. If it fails, run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

Expected: second check PASS.

- [x] **Step 3: Run complete backend verification**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: both commands PASS.

- [x] **Step 4: Confirm frontend remains untouched**

Run:

```bash
git status --short
```

Expected: only backend files and this plan's docs are changed for this slice.

- [x] **Step 5: Commit docs and final verification**

```bash
git add backend/README.md
git commit -m "docs: document command event boundary"
```

---

## Self-Review

- Spec coverage: This plan implements the next missing backend boundary from the architecture spec: validated indirect actions, Rust authority, append-only mutation events, dirty hot-state mutation, and WebSocket replication. It intentionally leaves Supabase/Postgres for a later adapter slice.
- Duplicate check: This does not redo the visible backend, snapshot loop, mobility, or simulation foundation work. It builds on `Chunk::set_tile_kind`, `ChunkRegistry`, `SimulationRuntime`, and the existing `/ws` broadcast channel.
- Placeholder scan: No placeholder tokens, copy-forward shortcuts, or vague "add tests" instructions are used.
- Type consistency: DTO names, enum variants, command IDs, event IDs, status tags, and route names match across tasks.
- Risk: `ServerMessageDto::WorldEvent { event: WorldEventDto }` intentionally uses a struct variant so the outer server message has its own `type` tag and the inner event keeps its own nested `type` tag. The protocol test in Task 1 locks the exact JSON shape before server work proceeds.
