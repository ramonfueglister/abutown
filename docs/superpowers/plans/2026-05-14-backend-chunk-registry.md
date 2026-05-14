# Backend Chunk Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single hard-coded backend chunk with a small authoritative chunk registry that can hold multiple loaded chunks, expose them through HTTP snapshots, and emit broadcast pulses from the loaded set.

**Architecture:** Add a focused `ChunkRegistry` module inside `sim-server` to own loaded chunk lookup, sorted chunk listing, and snapshot construction. Keep `SimulationRuntime` as the public server runtime facade: it owns world identity, tick/version counters, and pulse scheduling while delegating chunk storage to the registry.

**Tech Stack:** Rust, Axum, Tokio broadcast, `sim-core::Chunk`, `abutown-protocol` DTOs, Cargo integration tests.

---

## Scope

This plan implements only the next backend architecture slice:

- multiple loaded chunks in memory,
- deterministic loaded chunk ordering,
- HTTP snapshot access for every loaded chunk,
- central broadcast pulses that rotate through loaded chunks.

It does not implement Supabase/Postgres persistence, dynamic chunk loading from clients, spatial interest management, cross-worker authority transfer, or frontend multi-chunk visualization changes.

## File Structure

- Create `backend/crates/sim-server/src/chunk_registry.rs`
  - Owns `HashMap<ChunkCoord, LoadedChunk>`.
  - Provides sorted chunk coordinates.
  - Builds snapshots for loaded chunks.
  - Provides tile count lookup for pulse generation.
- Modify `backend/crates/sim-server/src/lib.rs`
  - Exposes the new module to the crate.
- Modify `backend/crates/sim-server/src/runtime.rs`
  - Replaces `chunk: Chunk` with `registry: ChunkRegistry`.
  - Seeds a small visible set of chunks.
  - Rotates pulse chunks deterministically.
- Modify `backend/crates/sim-server/tests/http.rs`
  - Verifies `/world` lists multiple chunks.
  - Verifies each loaded chunk can be fetched.
  - Keeps unknown chunk 404 behavior.
- Modify `backend/crates/sim-server/tests/websocket.rs`
  - Verifies the broadcast stream emits chunks from the loaded registry while preserving shared ticks.
- Modify `backend/README.md`
  - Documents that the visible dev slice now loads multiple chunks.

---

### Task 1: Add Chunk Registry Unit

**Files:**
- Create: `backend/crates/sim-server/src/chunk_registry.rs`
- Modify: `backend/crates/sim-server/src/lib.rs`

- [ ] **Step 1: Write the failing registry tests**

Create `backend/crates/sim-server/src/chunk_registry.rs` with this initial test-focused content:

```rust
use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, WorldId};
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    scheduler::ChunkActivity,
    tile::TileKind,
};

#[derive(Debug)]
pub(crate) struct LoadedChunk {
    chunk: Chunk,
    activity: ChunkActivity,
}

#[derive(Debug)]
pub(crate) struct ChunkRegistry {
    chunk_size: u16,
    chunks: HashMap<ChunkCoord, LoadedChunk>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_with_seed(coord: ChunkCoord, local_index: u16, kind: TileKind) -> Chunk {
        let mut chunk = Chunk::new(coord, 32);
        chunk
            .set_tile_kind(local_index, kind)
            .expect("seed index exists");
        chunk
    }

    #[test]
    fn registry_lists_loaded_chunks_in_deterministic_order() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 5, y: 4 }, 0, TileKind::Road),
            ChunkActivity::Warm,
        );
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 0, TileKind::Water),
            ChunkActivity::Active,
        );

        assert_eq!(
            registry.loaded_coords(),
            vec![ChunkCoord { x: 4, y: 4 }, ChunkCoord { x: 5, y: 4 }]
        );
    }

    #[test]
    fn registry_builds_snapshots_only_for_loaded_chunks() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 4 }, 17, TileKind::Road),
            ChunkActivity::Active,
        );

        let world_id = WorldId("abutown-main".to_string());
        let snapshot = registry
            .chunk_snapshot(&world_id, ChunkCoord { x: 4, y: 4 })
            .expect("loaded chunk snapshot exists");

        assert_eq!(snapshot.coord.x, 4);
        assert_eq!(snapshot.coord.y, 4);
        assert_eq!(snapshot.chunk_state, abutown_protocol::ChunkStateDto::Active);
        assert_eq!(snapshot.tile_count, 1024);
        assert_eq!(snapshot.dirty_tiles.len(), 1);
        assert_eq!(snapshot.dirty_tiles[0].local_index, 17);
        assert_eq!(snapshot.dirty_tiles[0].kind, abutown_protocol::TileKindDto::Road);
        assert!(registry
            .chunk_snapshot(&world_id, ChunkCoord { x: 0, y: 0 })
            .is_none());
    }

    #[test]
    fn registry_reports_loaded_tile_counts() {
        let mut registry = ChunkRegistry::new(32);
        registry.insert_chunk(
            chunk_with_seed(ChunkCoord { x: 4, y: 5 }, 0, TileKind::BuildingFootprint),
            ChunkActivity::Warm,
        );

        assert_eq!(registry.tile_count(ChunkCoord { x: 4, y: 5 }), Some(1024));
        assert_eq!(registry.tile_count(ChunkCoord { x: 9, y: 9 }), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry
```

Expected: FAIL with missing associated items such as `ChunkRegistry::new`, `insert_chunk`, `loaded_coords`, `chunk_snapshot`, and `tile_count`.

- [ ] **Step 3: Implement the registry**

Replace the non-test content in `backend/crates/sim-server/src/chunk_registry.rs` with:

```rust
use std::collections::HashMap;

use abutown_protocol::{ChunkSnapshotDto, WorldId};
use sim_core::{
    chunk::Chunk,
    ids::ChunkCoord,
    persistence::build_chunk_snapshot,
    scheduler::ChunkActivity,
};

#[derive(Debug)]
pub(crate) struct LoadedChunk {
    chunk: Chunk,
    activity: ChunkActivity,
}

#[derive(Debug)]
pub(crate) struct ChunkRegistry {
    chunk_size: u16,
    chunks: HashMap<ChunkCoord, LoadedChunk>,
}

impl ChunkRegistry {
    pub(crate) fn new(chunk_size: u16) -> Self {
        Self {
            chunk_size,
            chunks: HashMap::new(),
        }
    }

    pub(crate) fn chunk_size(&self) -> u16 {
        self.chunk_size
    }

    pub(crate) fn insert_chunk(&mut self, chunk: Chunk, activity: ChunkActivity) {
        debug_assert_eq!(chunk.chunk_size(), self.chunk_size);
        self.chunks
            .insert(chunk.coord(), LoadedChunk { chunk, activity });
    }

    pub(crate) fn loaded_coords(&self) -> Vec<ChunkCoord> {
        let mut coords: Vec<ChunkCoord> = self.chunks.keys().copied().collect();
        coords.sort_by_key(|coord| (coord.y, coord.x));
        coords
    }

    pub(crate) fn chunk_snapshot(
        &self,
        world_id: &WorldId,
        coord: ChunkCoord,
    ) -> Option<ChunkSnapshotDto> {
        let loaded = self.chunks.get(&coord)?;
        Some(build_chunk_snapshot(
            &world_id.0,
            &loaded.chunk,
            loaded.activity,
        ))
    }

    pub(crate) fn tile_count(&self, coord: ChunkCoord) -> Option<u16> {
        self.chunks.get(&coord).map(|loaded| loaded.chunk.tile_count())
    }
}
```

Keep the tests from Step 1 at the bottom of the file.

- [ ] **Step 4: Expose the module inside the crate**

Modify `backend/crates/sim-server/src/lib.rs` to:

```rust
pub mod app;
pub(crate) mod chunk_registry;
pub mod runtime;
```

- [ ] **Step 5: Run the registry tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server chunk_registry
```

Expected: PASS, with 3 registry tests passing.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/src/chunk_registry.rs backend/crates/sim-server/src/lib.rs
git commit -m "feat: add backend chunk registry"
```

---

### Task 2: Move SimulationRuntime To Registry

**Files:**
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Write failing runtime tests for multiple loaded chunks**

Replace the current `runtime_produces_monotonic_pulses_inside_seed_chunk` test module in `backend/crates/sim-server/src/runtime.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn tile_pulse(message: ServerMessageDto) -> TilePulseDeltaDto {
        let ServerMessageDto::TilePulse(delta) = message else {
            panic!("message should be a tile pulse");
        };
        delta
    }

    #[test]
    fn runtime_summarizes_multiple_loaded_chunks() {
        let runtime = SimulationRuntime::new();

        let summary = runtime.world_summary();

        assert_eq!(summary.chunk_size, 32);
        assert_eq!(
            summary.loaded_chunks,
            vec![
                ChunkCoordDto { x: 4, y: 4 },
                ChunkCoordDto { x: 5, y: 4 },
                ChunkCoordDto { x: 4, y: 5 },
            ]
        );
    }

    #[test]
    fn runtime_returns_snapshots_for_each_loaded_chunk() {
        let runtime = SimulationRuntime::new();

        let visible = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 4 })
            .expect("visible chunk loaded");
        let east = runtime
            .chunk_snapshot(ChunkCoord { x: 5, y: 4 })
            .expect("east chunk loaded");
        let south = runtime
            .chunk_snapshot(ChunkCoord { x: 4, y: 5 })
            .expect("south chunk loaded");

        assert_eq!(visible.coord, ChunkCoordDto { x: 4, y: 4 });
        assert_eq!(east.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(south.coord, ChunkCoordDto { x: 4, y: 5 });
        assert!(runtime.chunk_snapshot(ChunkCoord { x: 0, y: 0 }).is_none());
    }

    #[test]
    fn runtime_rotates_pulses_through_loaded_chunks() {
        let mut runtime = SimulationRuntime::new();

        let first = tile_pulse(runtime.next_pulse());
        let second = tile_pulse(runtime.next_pulse());
        let third = tile_pulse(runtime.next_pulse());
        let fourth = tile_pulse(runtime.next_pulse());

        assert_eq!(first.tick, 1);
        assert_eq!(first.version, 1);
        assert_eq!(first.coord, ChunkCoordDto { x: 4, y: 4 });
        assert!(first.local_index < 1024);
        assert_eq!(second.tick, 2);
        assert_eq!(second.coord, ChunkCoordDto { x: 5, y: 4 });
        assert_eq!(third.tick, 3);
        assert_eq!(third.coord, ChunkCoordDto { x: 4, y: 5 });
        assert_eq!(fourth.tick, 4);
        assert_eq!(fourth.coord, ChunkCoordDto { x: 4, y: 4 });
    }
}
```

- [ ] **Step 2: Run the runtime tests to verify they fail**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime::
```

Expected: FAIL because `SimulationRuntime` still exposes one loaded chunk and every pulse uses chunk `4:4`.

- [ ] **Step 3: Replace runtime storage with the registry**

Modify the top of `backend/crates/sim-server/src/runtime.rs` to use the registry:

```rust
use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, HealthResponse, PROTOCOL_VERSION, ServerHelloDto,
    ServerMessageDto, TilePulseDeltaDto, WorldId, WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk, ids::ChunkCoord, scheduler::ChunkActivity, tile::TileKind,
};

use crate::chunk_registry::ChunkRegistry;

const WORLD_ID: &str = "abutown-main";
const CHUNK_SIZE: u16 = 32;
const SEEDED_CHUNKS: [ChunkCoord; 3] = [
    ChunkCoord { x: 4, y: 4 },
    ChunkCoord { x: 5, y: 4 },
    ChunkCoord { x: 4, y: 5 },
];
const PULSE_STRIDE: u64 = 37;

#[derive(Debug)]
pub struct SimulationRuntime {
    world_id: WorldId,
    registry: ChunkRegistry,
    tick: u64,
    version: u64,
}
```

Replace `SimulationRuntime::new()` with:

```rust
pub fn new() -> Self {
    let mut registry = ChunkRegistry::new(CHUNK_SIZE);
    for (offset, coord) in SEEDED_CHUNKS.into_iter().enumerate() {
        let mut chunk = Chunk::new(coord, CHUNK_SIZE);
        let seed_index = (offset as u16) * 17;
        let seed_kind = match offset {
            0 => TileKind::Road,
            1 => TileKind::Water,
            _ => TileKind::BuildingFootprint,
        };
        chunk
            .set_tile_kind(seed_index, seed_kind)
            .expect("seed tile index is valid for visible chunk");
        let activity = if offset == 0 {
            ChunkActivity::Active
        } else {
            ChunkActivity::Warm
        };
        registry.insert_chunk(chunk, activity);
    }

    Self {
        world_id: WorldId(WORLD_ID.to_string()),
        registry,
        tick: 0,
        version: 0,
    }
}
```

Replace `world_summary`, `chunk_snapshot`, `hello`, and `next_pulse` with:

```rust
pub fn world_summary(&self) -> WorldSummaryDto {
    WorldSummaryDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: self.world_id.clone(),
        chunk_size: self.registry.chunk_size(),
        loaded_chunks: self
            .registry
            .loaded_coords()
            .into_iter()
            .map(Into::into)
            .collect(),
    }
}

pub fn chunk_snapshot(&self, coord: ChunkCoord) -> Option<ChunkSnapshotDto> {
    self.registry.chunk_snapshot(&self.world_id, coord)
}

pub fn hello(&self) -> ServerMessageDto {
    ServerMessageDto::Hello(ServerHelloDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: self.world_id.clone(),
        chunk_size: self.registry.chunk_size(),
    })
}

pub fn next_pulse(&mut self) -> ServerMessageDto {
    self.tick += 1;
    self.version += 1;

    let loaded_coords = self.registry.loaded_coords();
    let coord = loaded_coords[((self.tick - 1) as usize) % loaded_coords.len()];
    let tile_count = u64::from(
        self.registry
            .tile_count(coord)
            .expect("pulse target is loaded"),
    );
    let local_index = ((self.tick * PULSE_STRIDE) % tile_count) as u16;

    ServerMessageDto::TilePulse(TilePulseDeltaDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: self.world_id.clone(),
        tick: self.tick,
        version: self.version,
        coord: coord.into(),
        local_index,
    })
}
```

Keep `health()` and `Default` unchanged except for references to the new fields.

- [ ] **Step 4: Run runtime tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server runtime::
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/runtime.rs
git commit -m "feat: load multiple backend chunks"
```

---

### Task 3: Update HTTP Contract Tests

**Files:**
- Modify: `backend/crates/sim-server/tests/http.rs`

- [ ] **Step 1: Update the failing HTTP assertions**

In `backend/crates/sim-server/tests/http.rs`, replace the final assertions in `health_and_world_summary_are_available` with:

```rust
assert_eq!(json["protocol_version"], 1);
assert_eq!(json["world_id"], "abutown-main");
assert_eq!(json["chunk_size"], 32);
assert_eq!(json["loaded_chunks"].as_array().unwrap().len(), 3);
assert_eq!(json["loaded_chunks"][0]["x"], 4);
assert_eq!(json["loaded_chunks"][0]["y"], 4);
assert_eq!(json["loaded_chunks"][1]["x"], 5);
assert_eq!(json["loaded_chunks"][1]["y"], 4);
assert_eq!(json["loaded_chunks"][2]["x"], 4);
assert_eq!(json["loaded_chunks"][2]["y"], 5);
```

Add this new test after `chunk_snapshot_is_available_for_loaded_chunk`:

```rust
#[tokio::test]
async fn every_loaded_chunk_snapshot_is_available() {
    let app = build_app();

    for (x, y) in [(4, 4), (5, 4), (4, 5)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/chunks/{x}/{y}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["coord"]["x"], x);
        assert_eq!(json["coord"]["y"], y);
        assert_eq!(json["tile_count"], 1024);
    }
}
```

- [ ] **Step 2: Run HTTP tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http
```

Expected: PASS, with 4 HTTP tests passing.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/tests/http.rs
git commit -m "test: cover multiple backend chunks over http"
```

---

### Task 4: Update WebSocket Contract Tests

**Files:**
- Modify: `backend/crates/sim-server/tests/websocket.rs`

- [ ] **Step 1: Add a websocket test for registry-backed pulse chunks**

Add this test after `websocket_sends_hello_and_tile_pulse` and before `websocket_clients_receive_the_same_broadcast_tick`:

```rust
#[tokio::test]
async fn websocket_pulses_rotate_loaded_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello = read_server_message(&mut stream).await;
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let first = read_server_message(&mut stream).await;
    let second = read_server_message(&mut stream).await;
    let third = read_server_message(&mut stream).await;

    let ServerMessageDto::TilePulse(first_delta) = first else {
        panic!("first pulse expected");
    };
    let ServerMessageDto::TilePulse(second_delta) = second else {
        panic!("second pulse expected");
    };
    let ServerMessageDto::TilePulse(third_delta) = third else {
        panic!("third pulse expected");
    };

    assert_eq!(first_delta.coord, abutown_protocol::ChunkCoordDto { x: 4, y: 4 });
    assert_eq!(second_delta.coord, abutown_protocol::ChunkCoordDto { x: 5, y: 4 });
    assert_eq!(third_delta.coord, abutown_protocol::ChunkCoordDto { x: 4, y: 5 });

    server.abort();
}
```

- [ ] **Step 2: Run websocket tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: PASS, with 3 websocket tests passing.

- [ ] **Step 3: Commit**

```bash
git add backend/crates/sim-server/tests/websocket.rs
git commit -m "test: cover chunk registry websocket pulses"
```

---

### Task 5: Document And Verify

**Files:**
- Modify: `backend/README.md`

- [ ] **Step 1: Update backend README**

In `backend/README.md`, replace the visible slice description paragraph with:

```markdown
Open the Vite URL. The city should render normally and show a `RUST LIVE` badge. Chunk `4:4` is outlined from the server snapshot, and server-driven pulses appear from `/ws` roughly once per second. The runtime currently loads three visible chunks (`4:4`, `5:4`, and `4:5`) and rotates broadcast pulses across them.
```

Keep this line:

```markdown
Current `/ws` ticking is driven by one server-side scheduler and broadcast to connected clients.
```

- [ ] **Step 2: Run complete backend verification**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: all commands pass.

- [ ] **Step 3: Run relevant frontend verification**

Run:

```bash
npm test -- tests/backend/backendState.test.ts
npm run build
```

Expected:
- Backend bridge tests pass.
- Build passes.

Do not require full `npm test` for this slice if the existing OpenTTD import test remains failing on the base branch. If it fails, record the failing test name and keep this backend slice scoped.

- [ ] **Step 4: Run browser smoke**

Start the server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

Start the client:

```bash
npm run dev -- --port 5177
```

Open `http://127.0.0.1:5177/` and confirm:

- `RUST LIVE` appears,
- `world abutown-main` appears,
- `chunk 4:4 active` still appears,
- `tick` increments,
- no page errors are reported.

- [ ] **Step 5: Commit**

```bash
git add backend/README.md
git commit -m "docs: document backend chunk registry"
```

- [ ] **Step 6: Push PR branch**

Because this branch was previously rebased onto `origin/codex/zurich-river-city-world`, push with lease:

```bash
git push --force-with-lease origin codex/visible-backend-slice
```

Expected: PR #2 updates successfully.

---

## Self-Review

- Spec coverage: The plan covers multi-chunk in-memory authority, HTTP access, websocket pulse deltas, documentation, and verification. It intentionally excludes persistence, dynamic loading, Supabase, and interest management.
- Placeholder scan: No placeholder tokens or unspecified test instructions remain. Each task contains concrete code and commands.
- Type consistency: `ChunkRegistry`, `loaded_coords`, `chunk_snapshot`, and `tile_count` are introduced in Task 1 before `SimulationRuntime` uses them in Task 2. `ChunkCoord`, `ChunkActivity`, `TileKind`, and protocol DTOs match current backend types.
