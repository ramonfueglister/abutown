# Persistent World Event Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist accepted Rust-authoritative world events through an explicit store boundary before hot-state mutation, HTTP acceptance, and websocket broadcast.

**Architecture:** Add an async `WorldEventStore` contract in `sim-core`, keep the in-memory store as the default local implementation, and add a Postgres/Supabase-compatible adapter in `sim-server`. Split tile mutation into validate/plan and apply steps so append failure leaves loaded chunk state unchanged.

**Tech Stack:** Rust 2024, Axum, Tokio, `async-trait`, `sqlx` Postgres, serde JSON DTOs, existing `sim-core` chunk/runtime boundaries.

---

## Scope

This plan implements the design in `docs/superpowers/specs/2026-05-15-persistent-world-event-store-design.md`.

It includes:

- event-store trait and typed append errors,
- in-memory default store behind that trait,
- mutation planning before application,
- command handling that appends before mutating and broadcasting,
- Postgres table migration SQL,
- Postgres event-store adapter with opt-in integration tests,
- README updates and verification.

It does not include auth, command idempotency semantics, frontend command UI, durable chunk snapshots, or replay/recovery from persisted events.

## Current Baseline

Relevant current behavior:

- `backend/crates/sim-core/src/events.rs` contains only `InMemoryWorldEventStore`.
- `SimulationRuntime` directly owns `InMemoryWorldEventStore`.
- `ChunkRegistry::set_tile_kind()` validates and mutates in one method.
- `SimulationRuntime::apply_client_command()` mutates hot state first, then appends in-memory, then returns accepted.
- `app::command()` broadcasts accepted events from the `AppliedCommand`.

## File Structure

- Modify `backend/Cargo.toml`
  - Add `async-trait` workspace dependency.
- Modify `backend/crates/sim-core/Cargo.toml`
  - Add `async-trait` and test-only `tokio`.
- Modify `backend/crates/sim-core/src/events.rs`
  - Add `WorldEventStore`, `WorldEventStoreError`, event metadata helpers, and failing test store while preserving in-memory diagnostics helpers.
- Modify `backend/crates/sim-server/Cargo.toml`
  - Add `async-trait` and `sqlx` after the adapter is introduced.
- Create `backend/crates/sim-server/migrations/202605150001_world_events.sql`
  - Add `world_events` table and indexes.
- Create `backend/crates/sim-server/src/postgres_events.rs`
  - Add `PostgresWorldEventStore`.
- Modify `backend/crates/sim-server/src/lib.rs`
  - Export `postgres_events` internally.
- Modify `backend/crates/sim-server/src/chunk_registry.rs`
  - Add mutation planning and application methods.
- Modify `backend/crates/sim-server/src/runtime.rs`
  - Store `Box<dyn WorldEventStore + Send>`.
  - Make command application async.
  - Append before applying planned mutation.
- Modify `backend/crates/sim-server/src/app.rs`
  - Await async command application.
  - Add async app builder from environment for persistent store config.
- Modify `backend/crates/sim-server/src/main.rs`
  - Use async app builder.
- Modify `backend/crates/sim-server/tests/http.rs`
  - Add append-failure HTTP test.
- Modify `backend/crates/sim-server/tests/websocket.rs`
  - Add no-broadcast-on-append-failure test.
- Modify `backend/README.md`
  - Document persistent event-store config and limitations.

---

### Task 1: Add Event Store Contract

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/crates/sim-core/Cargo.toml`
- Modify: `backend/crates/sim-core/src/events.rs`

- [ ] **Step 1: Add dependency entries**

Add this workspace dependency in `backend/Cargo.toml` under `[workspace.dependencies]`:

```toml
async-trait = "0.1"
```

Add these dependencies in `backend/crates/sim-core/Cargo.toml`:

```toml
async-trait.workspace = true

[dev-dependencies]
tokio.workspace = true
```

- [ ] **Step 2: Write failing event-store contract tests**

Replace the existing tests in `backend/crates/sim-core/src/events.rs` with these tests:

```rust
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

    #[tokio::test]
    async fn event_store_appends_events_in_order() {
        let mut store = InMemoryWorldEventStore::default();

        WorldEventStore::append(&mut store, tile_event("event:1", 1))
            .await
            .unwrap();
        WorldEventStore::append(&mut store, tile_event("event:2", 2))
            .await
            .unwrap();

        assert_eq!(store.event_count(), 2);
        assert_eq!(store.events()[0], tile_event("event:1", 1));
        assert_eq!(store.events()[1], tile_event("event:2", 2));
    }

    #[tokio::test]
    async fn failing_event_store_returns_typed_error_without_appending() {
        let mut store = FailingWorldEventStore::new("database offline");

        let error = WorldEventStore::append(&mut store, tile_event("event:1", 1))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "event_store_unavailable");
        assert!(error.to_string().contains("database offline"));
        assert_eq!(store.event_count(), 0);
    }

    #[test]
    fn event_metadata_extracts_queryable_columns() {
        let metadata = WorldEventMetadata::from_event(&tile_event("event:7", 7));

        assert_eq!(metadata.event_id, "event:7");
        assert_eq!(metadata.world_id, "abutown-main");
        assert_eq!(metadata.command_id, "command:event:7");
        assert_eq!(metadata.event_type, "tile_kind_set");
        assert_eq!(metadata.tick, 7);
        assert_eq!(metadata.version, 7);
    }
}
```

- [ ] **Step 3: Run failing sim-core events tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core events
```

Expected: FAIL because `WorldEventStore`, `WorldEventStoreError`, `FailingWorldEventStore`, and `WorldEventMetadata` do not exist.

- [ ] **Step 4: Implement event-store contract**

Replace `backend/crates/sim-core/src/events.rs` with:

```rust
use abutown_protocol::WorldEventDto;
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldEventMetadata {
    pub event_id: String,
    pub world_id: String,
    pub command_id: String,
    pub event_type: &'static str,
    pub tick: u64,
    pub version: u64,
}

impl WorldEventMetadata {
    pub fn from_event(event: &WorldEventDto) -> Self {
        match event {
            WorldEventDto::TileKindSet(event) => Self {
                event_id: event.event_id.clone(),
                world_id: event.world_id.0.clone(),
                command_id: event.command_id.clone(),
                event_type: "tile_kind_set",
                tick: event.tick,
                version: event.version,
            },
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct WorldEventStoreError {
    code: &'static str,
    message: String,
}

impl WorldEventStoreError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "event_store_unavailable",
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[async_trait]
pub trait WorldEventStore: std::fmt::Debug + Send {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryWorldEventStore {
    events: Vec<WorldEventDto>,
}

#[async_trait]
impl WorldEventStore for InMemoryWorldEventStore {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        InMemoryWorldEventStore::append(self, event);
        Ok(())
    }
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

#[derive(Debug)]
pub struct FailingWorldEventStore {
    message: String,
}

impl FailingWorldEventStore {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
impl WorldEventStore for FailingWorldEventStore {
    async fn append(&mut self, _event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        Err(WorldEventStoreError::unavailable(self.message.clone()))
    }
}

impl FailingWorldEventStore {
    pub fn event_count(&self) -> usize {
        0
    }
}
```

Keep the tests from Step 2 below this code.

- [ ] **Step 5: Run sim-core events tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core events
cargo check --locked --manifest-path backend/Cargo.toml --workspace
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/sim-core/Cargo.toml backend/crates/sim-core/src/events.rs
git commit -m "feat: add world event store contract"
```

---

### Task 2: Split Chunk Mutation Into Plan And Apply

**Files:**
- Modify: `backend/crates/sim-server/src/chunk_registry.rs`

- [ ] **Step 1: Write failing registry planning tests**

Add this type assertion and test inside `#[cfg(test)] mod tests` in `backend/crates/sim-server/src/chunk_registry.rs` after `registry_sets_tile_kind_on_loaded_chunk`:

```rust
    #[test]
    fn registry_plans_tile_kind_mutation_without_changing_chunk() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let plan = registry
            .plan_set_tile_kind(ChunkCoord { x: 4, y: 4 }, 11, TileKind::Water)
            .expect("loaded tile can be planned");

        assert_eq!(plan.coord, ChunkCoord { x: 4, y: 4 });
        assert_eq!(plan.local_index, 11);
        assert_eq!(plan.kind, TileKind::Water);
        assert_eq!(plan.version, 2);

        let snapshot = registry
            .chunk_snapshot(
                &WorldId("abutown-main".to_string()),
                ChunkCoord { x: 4, y: 4 },
            )
            .expect("chunk snapshot exists");
        assert!(!snapshot.dirty_tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }

    #[test]
    fn registry_applies_planned_tile_kind_mutation() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Active,
        );

        let plan = registry
            .plan_set_tile_kind(ChunkCoord { x: 4, y: 4 }, 11, TileKind::Water)
            .expect("loaded tile can be planned");

        registry
            .apply_set_tile_kind(plan)
            .expect("planned mutation applies");

        let snapshot = registry
            .chunk_snapshot(
                &WorldId("abutown-main".to_string()),
                ChunkCoord { x: 4, y: 4 },
            )
            .expect("chunk snapshot exists");
        assert!(snapshot.dirty_tiles.iter().any(|tile| {
            tile.local_index == 11 && tile.kind == abutown_protocol::TileKindDto::Water
        }));
    }
```

- [ ] **Step 2: Run failing registry tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry::tests::registry_
```

Expected: FAIL because `SetTileKindPlan`, `plan_set_tile_kind`, and `apply_set_tile_kind` do not exist.

- [ ] **Step 3: Add plan type and methods**

Add this public-in-crate type near `ChunkMutationError` in `chunk_registry.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SetTileKindPlan {
    pub(crate) coord: ChunkCoord,
    pub(crate) local_index: u16,
    pub(crate) kind: TileKind,
    pub(crate) version: u64,
}
```

Replace the body of `set_tile_kind` and add the new methods:

```rust
    pub(crate) fn plan_set_tile_kind(
        &self,
        coord: ChunkCoord,
        local_index: u16,
        kind: TileKind,
    ) -> Result<SetTileKindPlan, ChunkMutationError> {
        let loaded = self
            .chunks
            .get(&coord)
            .ok_or(ChunkMutationError::ChunkNotLoaded { coord })?;
        let existing_kind =
            loaded
                .chunk
                .kind_at(local_index)
                .ok_or(ChunkMutationError::TileOutOfBounds {
                    index: local_index,
                    tile_count: loaded.chunk.tile_count(),
                })?;

        if existing_kind == kind {
            return Err(ChunkMutationError::NoStateChange { coord, local_index });
        }

        Ok(SetTileKindPlan {
            coord,
            local_index,
            kind,
            version: loaded.chunk.version() + 1,
        })
    }

    pub(crate) fn apply_set_tile_kind(
        &mut self,
        plan: SetTileKindPlan,
    ) -> Result<u64, ChunkMutationError> {
        let loaded = self
            .chunks
            .get_mut(&plan.coord)
            .ok_or(ChunkMutationError::ChunkNotLoaded { coord: plan.coord })?;

        loaded
            .chunk
            .set_tile_kind(plan.local_index, plan.kind)
            .map_err(|error| match error {
                ChunkError::IndexOutOfBounds { index, tile_count } => {
                    ChunkMutationError::TileOutOfBounds { index, tile_count }
                }
                ChunkError::InvalidChunkSize { .. } => {
                    unreachable!("loaded chunks are already valid")
                }
            })?;

        debug_assert_eq!(loaded.chunk.version(), plan.version);
        Ok(loaded.chunk.version())
    }

    pub(crate) fn set_tile_kind(
        &mut self,
        coord: ChunkCoord,
        local_index: u16,
        kind: TileKind,
    ) -> Result<u64, ChunkMutationError> {
        let plan = self.plan_set_tile_kind(coord, local_index, kind)?;
        self.apply_set_tile_kind(plan)
    }
```

- [ ] **Step 4: Run registry tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry::tests::registry_
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/chunk_registry.rs
git commit -m "feat: split chunk mutation planning and apply"
```

---

### Task 3: Append Events Before Hot-State Mutation

**Files:**
- Modify: `backend/crates/sim-server/src/commands.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/tests/http.rs`
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [ ] **Step 1: Add failing runtime append-failure test**

In `backend/crates/sim-server/src/runtime.rs`, add `FailingWorldEventStore` to the test imports by using the fully qualified path in the test. Add this test after `runtime_rejects_no_op_tile_kind_commands_without_appending_event`:

```rust
    #[tokio::test]
    async fn runtime_rejects_store_failure_without_mutating_chunk() {
        let mut runtime = SimulationRuntime::new_with_event_store(Box::new(
            sim_core::events::FailingWorldEventStore::new("database offline"),
        ));

        let before = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("chunk exists");

        let rejection = runtime
            .apply_client_command(abutown_protocol::ClientCommandDto::SetTileKind(
                abutown_protocol::SetTileKindCommandDto {
                    protocol_version: abutown_protocol::PROTOCOL_VERSION,
                    world_id: abutown_protocol::WorldId("abutown-main".to_string()),
                    command_id: "command:test:store-failure".to_string(),
                    coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
                    local_index: 11,
                    kind: abutown_protocol::TileKindDto::Water,
                },
            ))
            .await
            .expect_err("store failure should reject");

        assert_eq!(rejection.code, "event_store_unavailable");
        assert_eq!(runtime.event_count(), 0);
        assert_eq!(
            runtime
                .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
                .expect("chunk still exists"),
            before
        );
    }
```

Convert the existing runtime command tests that call `apply_client_command` to `#[tokio::test]`, add `.await` before `expect`/`expect_err`, and keep their existing assertions.

- [ ] **Step 2: Run failing runtime test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime::tests::runtime_rejects_store_failure_without_mutating_chunk
```

Expected: FAIL because `SimulationRuntime::new_with_event_store` does not exist and command application is not async.

- [ ] **Step 3: Extend command rejection type**

No field change is needed in `CommandRejection`, but runtime code will now build a rejection from `WorldEventStoreError`:

```rust
CommandRejection {
    world_id: Some(self.world_id.clone()),
    command_id: Some(command.command_id.clone()),
    code: error.code(),
    message: error.to_string(),
}
```

- [ ] **Step 4: Make runtime own a boxed event store**

In `backend/crates/sim-server/src/runtime.rs`, change the event imports to:

```rust
events::{InMemoryWorldEventStore, WorldEventStore},
```

Change the field:

```rust
event_store: Box<dyn WorldEventStore + Send>,
```

Add this constructor after `pub fn new() -> Self`:

```rust
    pub fn new_with_event_store(event_store: Box<dyn WorldEventStore + Send>) -> Self {
        let mut runtime = Self::new();
        runtime.event_store = event_store;
        runtime
    }
```

In `new()`, initialize:

```rust
event_store: Box::new(InMemoryWorldEventStore::default()),
```

- [ ] **Step 5: Append before applying planned mutation**

Make `apply_client_command` async:

```rust
    pub(crate) async fn apply_client_command(
        &mut self,
        command: ClientCommandDto,
    ) -> Result<AppliedCommand, CommandRejection> {
        match command {
            ClientCommandDto::SetTileKind(command) => self.apply_set_tile_kind(command).await,
        }
    }
```

Make `apply_set_tile_kind` async and replace its mutation block with planning, event append, then apply:

```rust
        let coord = ChunkCoord {
            x: command.coord.x,
            y: command.coord.y,
        };
        let kind = TileKind::from(command.kind);
        let plan = self
            .registry
            .plan_set_tile_kind(coord, command.local_index, kind)
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
                ChunkMutationError::NoStateChange { coord, local_index } => CommandRejection {
                    world_id: Some(command.world_id.clone()),
                    command_id: Some(command.command_id.clone()),
                    code: "no_state_change",
                    message: format!(
                        "tile {local_index} in chunk {}:{} already has the requested kind",
                        coord.x, coord.y
                    ),
                },
            })?;

        let event_id = format!("event:{}", self.next_event_id);
        let event = WorldEventDto::TileKindSet(TileKindSetEventDto {
            protocol_version: PROTOCOL_VERSION,
            event_id,
            command_id: command.command_id.clone(),
            world_id: self.world_id.clone(),
            tick: self.tick,
            version: plan.version,
            coord: command.coord,
            local_index: command.local_index,
            kind: command.kind,
        });

        self.event_store
            .append(event.clone())
            .await
            .map_err(|error| CommandRejection {
                world_id: Some(self.world_id.clone()),
                command_id: Some(command.command_id.clone()),
                code: error.code(),
                message: error.to_string(),
            })?;

        self.next_event_id += 1;
        self.registry
            .apply_set_tile_kind(plan)
            .expect("planned mutation should apply after event append");
```

Keep the existing `CommandAcceptedDto` construction after this block.

- [ ] **Step 6: Await command application in app handler**

In `backend/crates/sim-server/src/app.rs`, change:

```rust
        runtime.apply_client_command(command)
```

to:

```rust
        runtime.apply_client_command(command).await
```

- [ ] **Step 7: Add HTTP append-failure test**

In `backend/crates/sim-server/tests/http.rs`, add a test that builds an app with a failing store:

```rust
#[tokio::test]
async fn command_store_failure_returns_rejection_and_preserves_snapshot() {
    let app = build_app_with_runtime(SimulationRuntime::new_with_event_store(Box::new(
        sim_core::events::FailingWorldEventStore::new("database offline"),
    )));

    let before_response = app
        .clone()
        .oneshot(Request::builder().uri("/chunks/4/4").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let before_body = before_response.into_body().collect().await.unwrap().to_bytes();
    let before: Value = serde_json::from_slice(&before_body).unwrap();

    let command = abutown_protocol::ClientCommandDto::SetTileKind(
        abutown_protocol::SetTileKindCommandDto {
            protocol_version: abutown_protocol::PROTOCOL_VERSION,
            world_id: abutown_protocol::WorldId("abutown-main".to_string()),
            command_id: "command:http:store-failure".to_string(),
            coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
            local_index: 11,
            kind: abutown_protocol::TileKindDto::Water,
        },
    );

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

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "rejected");
    assert_eq!(json["code"], "event_store_unavailable");

    let after_response = app
        .oneshot(Request::builder().uri("/chunks/4/4").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let after_body = after_response.into_body().collect().await.unwrap().to_bytes();
    let after: Value = serde_json::from_slice(&after_body).unwrap();
    assert_eq!(after, before);
}
```

- [ ] **Step 8: Add websocket no-broadcast-on-failure test**

In `backend/crates/sim-server/tests/websocket.rs`, add a test that connects a websocket to an app with `FailingWorldEventStore`, posts a command, and asserts the HTTP response is rejected. Do not wait for a world event, because no event should be broadcast. Use a short timeout to prove no message arrives:

```rust
#[tokio::test]
async fn websocket_does_not_broadcast_failed_command_append() {
    let app = build_app_with_runtime(SimulationRuntime::new_with_event_store(Box::new(
        sim_core::events::FailingWorldEventStore::new("database offline"),
    )));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.unwrap();
    });

    let (mut websocket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
    let _hello = read_server_message(&mut websocket).await;

    let command = abutown_protocol::ClientCommandDto::SetTileKind(
        abutown_protocol::SetTileKindCommandDto {
            protocol_version: abutown_protocol::PROTOCOL_VERSION,
            world_id: abutown_protocol::WorldId("abutown-main".to_string()),
            command_id: "command:ws:store-failure".to_string(),
            coord: abutown_protocol::ChunkCoordDto { x: 4, y: 4 },
            local_index: 11,
            kind: abutown_protocol::TileKindDto::Water,
        },
    );

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
    let no_message = tokio::time::timeout(Duration::from_millis(150), websocket.next()).await;
    assert!(no_message.is_err());

    server.abort();
}
```

- [ ] **Step 9: Run command tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server command_
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_does_not_broadcast_failed_command_append
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add backend/crates/sim-server/src/commands.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/http.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat: append command events before mutation"
```

---

### Task 4: Add Postgres Event Store Adapter

**Files:**
- Modify: `backend/crates/sim-server/Cargo.toml`
- Create: `backend/crates/sim-server/migrations/202605150001_world_events.sql`
- Create: `backend/crates/sim-server/src/postgres_events.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`

- [ ] **Step 1: Add sim-server dependencies**

Add these to `backend/crates/sim-server/Cargo.toml`:

```toml
async-trait.workspace = true
sqlx.workspace = true
```

Add this workspace dependency in `backend/Cargo.toml` under `[workspace.dependencies]`:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "json", "time"] }
```

- [ ] **Step 2: Create migration SQL**

Create `backend/crates/sim-server/migrations/202605150001_world_events.sql`:

```sql
CREATE TABLE IF NOT EXISTS world_events (
    event_id TEXT PRIMARY KEY,
    world_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    tick BIGINT NOT NULL CHECK (tick >= 0),
    version BIGINT NOT NULL CHECK (version >= 0),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS world_events_world_version_idx
    ON world_events (world_id, version);

CREATE INDEX IF NOT EXISTS world_events_world_tick_idx
    ON world_events (world_id, tick);

CREATE INDEX IF NOT EXISTS world_events_world_command_idx
    ON world_events (world_id, command_id);
```

- [ ] **Step 3: Add failing adapter unit tests**

Create `backend/crates/sim-server/src/postgres_events.rs` with tests first:

```rust
use abutown_protocol::{
    ChunkCoordDto, PROTOCOL_VERSION, TileKindDto, TileKindSetEventDto, WorldEventDto, WorldId,
};
use sim_core::events::WorldEventMetadata;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_record_extracts_metadata_and_json_payload() {
        let event = tile_event("event:9", 9);
        let record = SqlWorldEventRecord::from_event(&event).unwrap();

        assert_eq!(record.metadata, WorldEventMetadata::from_event(&event));
        assert_eq!(record.payload["type"], "tile_kind_set");
        assert_eq!(record.payload["event_id"], "event:9");
    }
}
```

- [ ] **Step 4: Run failing adapter tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server postgres_events
```

Expected: FAIL because `SqlWorldEventRecord` does not exist.

- [ ] **Step 5: Implement adapter record and store**

Replace `backend/crates/sim-server/src/postgres_events.rs` with:

```rust
use abutown_protocol::WorldEventDto;
use async_trait::async_trait;
use serde_json::Value;
use sim_core::events::{WorldEventMetadata, WorldEventStore, WorldEventStoreError};
use sqlx::{PgPool, postgres::PgPoolOptions};

const WORLD_EVENTS_MIGRATION: &str = include_str!("../migrations/202605150001_world_events.sql");

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SqlWorldEventRecord {
    pub(crate) metadata: WorldEventMetadata,
    pub(crate) payload: Value,
}

impl SqlWorldEventRecord {
    pub(crate) fn from_event(event: &WorldEventDto) -> Result<Self, WorldEventStoreError> {
        let payload = serde_json::to_value(event)
            .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        Ok(Self {
            metadata: WorldEventMetadata::from_event(event),
            payload,
        })
    }
}

#[derive(Debug)]
pub(crate) struct PostgresWorldEventStore {
    pool: PgPool,
}

impl PostgresWorldEventStore {
    pub(crate) async fn connect(database_url: &str) -> Result<Self, WorldEventStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;

        for statement in WORLD_EVENTS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;
        }

        Ok(Self { pool })
    }
}

#[async_trait]
impl WorldEventStore for PostgresWorldEventStore {
    async fn append(&mut self, event: WorldEventDto) -> Result<(), WorldEventStoreError> {
        let record = SqlWorldEventRecord::from_event(&event)?;
        let tick = i64::try_from(record.metadata.tick)
            .map_err(|_| WorldEventStoreError::unavailable("event tick exceeds i64"))?;
        let version = i64::try_from(record.metadata.version)
            .map_err(|_| WorldEventStoreError::unavailable("event version exceeds i64"))?;

        sqlx::query(
            r#"
            INSERT INTO world_events (
                event_id,
                world_id,
                command_id,
                event_type,
                tick,
                version,
                payload
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(&record.metadata.event_id)
        .bind(&record.metadata.world_id)
        .bind(&record.metadata.command_id)
        .bind(record.metadata.event_type)
        .bind(tick)
        .bind(version)
        .bind(record.payload)
        .execute(&self.pool)
        .await
        .map_err(|error| WorldEventStoreError::unavailable(error.to_string()))?;

        Ok(())
    }
}
```

Then append the test from Step 3 below the implementation. Do not add count/list helpers to the DB adapter in this slice; durable read APIs belong in a later replay or admin-query slice.

- [ ] **Step 6: Export module**

In `backend/crates/sim-server/src/lib.rs`, add:

```rust
pub(crate) mod postgres_events;
```

- [ ] **Step 7: Add opt-in Postgres integration test**

Add this test module to the bottom of `postgres_events.rs`:

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use sim_core::events::WorldEventStore;

    #[tokio::test]
    async fn postgres_store_appends_event_when_database_url_is_set() {
        let Ok(database_url) = std::env::var("ABUTOWN_TEST_DATABASE_URL") else {
            eprintln!("skipping postgres integration test: ABUTOWN_TEST_DATABASE_URL is not set");
            return;
        };

        let mut store = PostgresWorldEventStore::connect(&database_url)
            .await
            .expect("connect postgres event store");
        let event = tile_event(&format!("event:test:{}", uuid::Uuid::now_v7()), 1);

        store.append(event).await.expect("append event");
    }
}
```

Add `uuid.workspace = true` to `backend/crates/sim-server/Cargo.toml` dev dependencies if it is not already available to the package.

- [ ] **Step 8: Run adapter tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server postgres_events
```

Expected: PASS without a database URL. The integration test prints a skip message and returns.

- [ ] **Step 9: Commit**

```bash
git add backend/crates/sim-server/Cargo.toml backend/crates/sim-server/migrations/202605150001_world_events.sql backend/crates/sim-server/src/postgres_events.rs backend/crates/sim-server/src/lib.rs
git commit -m "feat: add postgres world event store"
```

---

### Task 5: Wire Persistent Store Configuration

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/main.rs`
- Modify: `backend/README.md`

- [ ] **Step 1: Add app builder from environment**

In `backend/crates/sim-server/src/app.rs`, import the adapter:

```rust
use crate::{postgres_events::PostgresWorldEventStore, runtime::SimulationRuntime};
```

Add this builder after `build_app()`:

```rust
pub async fn build_app_from_env() -> anyhow::Result<Router> {
    if let Ok(database_url) = std::env::var("ABUTOWN_DATABASE_URL") {
        let event_store = PostgresWorldEventStore::connect(&database_url).await?;
        return Ok(build_app_with_runtime(SimulationRuntime::new_with_event_store(
            Box::new(event_store),
        )));
    }

    Ok(build_app())
}
```

- [ ] **Step 2: Update main**

In `backend/crates/sim-server/src/main.rs`, change the import:

```rust
use sim_server::app::build_app_from_env;
```

Change the serve call:

```rust
    axum::serve(listener, build_app_from_env().await?)
        .await
        .context("run simulation server")
```

- [ ] **Step 3: Run server build test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server app::tests::persist_snapshots_once_writes_runtime_snapshots
cargo clippy --manifest-path backend/Cargo.toml -p sim-server --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Update README**

In `backend/README.md`, update the Command Event Boundary section so the current boundaries say:

```markdown
- Accepted mutations are appended through the runtime event-store boundary before hot-state application and websocket broadcast.
- Local development defaults to an in-memory event store.
- Set `ABUTOWN_DATABASE_URL` to a Postgres/Supabase connection string to use the persistent `world_events` store.
- Command idempotency, permissions, chunk loading, recovery replay, and durable chunk snapshots remain later slices.
```

Add targeted commands:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-core events
cargo test --manifest-path backend/Cargo.toml -p sim-server postgres_events
cargo test --manifest-path backend/Cargo.toml -p sim-server command_
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_does_not_broadcast_failed_command_append
```

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/main.rs backend/README.md
git commit -m "feat: configure persistent world event store"
```

---

### Task 6: Final Verification

**Files:**
- Verify all backend files changed by this plan.

- [ ] **Step 1: Run formatting**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

Expected: PASS. If it fails, run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all
```

Then commit formatting with the files rustfmt changed:

```bash
git add backend
git commit -m "style: format persistent event store"
```

- [ ] **Step 2: Run complete backend tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: PASS without requiring `ABUTOWN_TEST_DATABASE_URL`.

- [ ] **Step 3: Run complete backend clippy**

Run:

```bash
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Confirm frontend worktree remains untouched by this slice**

Run:

```bash
git diff --name-only -- src tests package.json vite.config.ts
```

Expected: no new files from this backend slice. Existing user changes may still appear; do not stage them unless they were intentionally part of this plan.

- [ ] **Step 5: Final status**

Run:

```bash
git status --short
git log --oneline -8
```

Expected: only unrelated pre-existing local changes remain unstaged.

---

## Plan Self-Review

- Spec coverage: The plan covers durable event-store boundary, append-before-mutate ordering, failure behavior, Postgres-compatible storage, local no-database testability, README documentation, and explicit non-goals.
- Placeholder scan: No `TBD`, `TODO`, or vague implementation steps remain. Code-changing steps include concrete code or commands.
- Type consistency: `WorldEventStore`, `WorldEventStoreError`, `WorldEventMetadata`, `SetTileKindPlan`, `PostgresWorldEventStore`, and `build_app_from_env` are introduced before later tasks use them.
