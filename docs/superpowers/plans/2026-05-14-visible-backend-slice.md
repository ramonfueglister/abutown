# Visible Backend Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first visible browser-to-Rust runtime slice: the existing Zurich canvas scene shows a Rust-owned live chunk overlay and server-driven pulse.

**Status 2026-05-15:** Backend protocol, runtime, HTTP routes, and WebSocket stream are implemented and tested. The frontend bridge files referenced by this original plan (`src/backend/*`, `src/render/backendOverlay.ts`, and related tests) are not present in this branch, so the visible browser overlay portion remains pending.

**Architecture:** Extend the Rust protocol with JSON WebSocket messages, give `sim-server` a small in-memory authoritative runtime, and expose `/ws` for low-frequency tile pulse deltas. Add a frontend backend bridge that fetches snapshot state, consumes WebSocket messages, and passes a render-friendly overlay state to a focused canvas overlay renderer.

**Tech Stack:** Rust 2024, Axum WebSocket, Tokio, `serde` JSON DTOs, Vite, TypeScript, Vitest, Playwright for final visual smoke.

---

## Scope Check

This plan implements one vertical slice from `docs/superpowers/specs/2026-05-14-visible-backend-slice-design.md`. It does not add Supabase, auth, durable persistence, full Zurich server authority, or load testing.

## File Structure

- Modify `backend/crates/protocol/src/lib.rs`: add versioned WebSocket DTOs and serialization tests.
- Modify `backend/crates/sim-server/Cargo.toml`: add runtime JSON/CORS/WebSocket test dependencies.
- Create `backend/crates/sim-server/src/lib.rs`: conventional library root that exposes `app` and `runtime`.
- Replace `backend/crates/sim-server/src/app.rs`: top-level Axum routes, CORS, state injection, and `/ws`.
- Create `backend/crates/sim-server/src/runtime.rs`: in-memory authoritative runtime for chunk `4:4`, tick/version, snapshot, and pulse deltas. Chunk `4:4` starts at grid `128:128`, matching the current Zurich camera focus so the first slice is visible immediately.
- Modify `backend/crates/sim-server/src/main.rs`: keep binary entrypoint using the new library root.
- Modify `backend/crates/sim-server/tests/http.rs`: keep HTTP tests and add state-backed assertions.
- Create `backend/crates/sim-server/tests/websocket.rs`: integration test for hello plus a server pulse delta.
- Create `src/backend/protocol.ts`: TypeScript protocol types and runtime guards.
- Create `src/backend/backendState.ts`: pure state reducer for health, world, snapshots, and server messages.
- Create `src/backend/backendClient.ts`: browser bridge for HTTP snapshot loading and WebSocket reconnect.
- Create `src/render/backendOverlay.ts`: canvas overlay rendering and pure coordinate/pulse helpers.
- Modify `src/main.ts`: wire backend bridge state into the existing render loop.
- Create `tests/backend/backendState.test.ts`: frontend state reducer tests.
- Create `tests/render/backendOverlay.test.ts`: overlay coordinate and pulse expiry tests.
- Modify `backend/README.md`: add local two-process run instructions.

## Task 1: Rust Protocol WebSocket DTOs

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`

- [ ] **Step 1: Add failing protocol serialization tests**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `backend/crates/protocol/src/lib.rs`:

```rust
    #[test]
    fn websocket_hello_serializes_with_type_tag() {
        let message = ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            chunk_size: 32,
        });

        let json = serde_json::to_string(&message).expect("hello serializes");

        assert_eq!(
            json,
            r#"{"type":"hello","protocol_version":1,"world_id":"abutown-main","chunk_size":32}"#
        );
    }

    #[test]
    fn websocket_tile_pulse_serializes_chunk_and_version() {
        let message = ServerMessageDto::TilePulse(TilePulseDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: WorldId("abutown-main".to_string()),
            tick: 7,
            version: 11,
            coord: ChunkCoordDto { x: 0, y: 0 },
            local_index: 231,
        });

        let json = serde_json::to_string(&message).expect("tile pulse serializes");

        assert_eq!(
            json,
            r#"{"type":"tile_pulse","protocol_version":1,"world_id":"abutown-main","tick":7,"version":11,"coord":{"x":0,"y":0},"local_index":231}"#
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol websocket_
```

Expected: FAIL because `ServerMessageDto`, `ServerHelloDto`, and `TilePulseDeltaDto` are not defined.

- [ ] **Step 3: Add the protocol DTOs**

Insert this code after `ChunkSnapshotDto` in `backend/crates/protocol/src/lib.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessageDto {
    Hello(ServerHelloDto),
    TilePulse(TilePulseDeltaDto),
    Error(ServerErrorDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerHelloDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TilePulseDeltaDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub tick: u64,
    pub version: u64,
    pub coord: ChunkCoordDto,
    pub local_index: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerErrorDto {
    pub protocol_version: u16,
    pub world_id: Option<WorldId>,
    pub code: String,
    pub message: String,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol websocket_
```

Expected: PASS with 2 protocol tests passing.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/protocol/src/lib.rs
git commit -m "feat: add backend websocket protocol messages"
```

## Task 2: State-Backed HTTP Runtime

**Files:**
- Modify: `backend/crates/sim-server/Cargo.toml`
- Create: `backend/crates/sim-server/src/lib.rs`
- Replace: `backend/crates/sim-server/src/app.rs`
- Create: `backend/crates/sim-server/src/runtime.rs`
- Modify: `backend/crates/sim-server/tests/http.rs`

- [ ] **Step 1: Write failing runtime-backed HTTP test**

Replace `backend/crates/sim-server/tests/http.rs` with:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use sim_server::app::build_app;

#[tokio::test]
async fn health_and_world_summary_are_available() {
    let app = build_app();

    let health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health_response.status(), StatusCode::OK);

    let world_response = app
        .oneshot(
            Request::builder()
                .uri("/world")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(world_response.status(), StatusCode::OK);

    let body = world_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["protocol_version"], 1);
    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["chunk_size"], 32);
    assert_eq!(json["loaded_chunks"][0]["x"], 4);
    assert_eq!(json["loaded_chunks"][0]["y"], 4);
}

#[tokio::test]
async fn chunk_snapshot_is_available_for_loaded_chunk() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/4/4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["world_id"], "abutown-main");
    assert_eq!(json["coord"]["x"], 4);
    assert_eq!(json["coord"]["y"], 4);
    assert_eq!(json["tile_count"], 1024);
    assert_eq!(json["chunk_state"], "active");

    let dirty_tiles = json["dirty_tiles"].as_array().unwrap();
    assert_eq!(dirty_tiles.len(), 1);
    assert_eq!(dirty_tiles[0]["local_index"], 0);
    assert_eq!(dirty_tiles[0]["kind"], "road");
}

#[tokio::test]
async fn unloaded_chunk_returns_not_found() {
    let app = build_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/chunks/0/0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run test to verify it fails against the old static chunk**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test http
```

Expected: FAIL because the old static handler exposes chunk `0:0`, while this visible slice uses chunk `4:4`.

- [ ] **Step 3: Update sim-server dependencies**

In `backend/crates/sim-server/Cargo.toml`, change the library section and dependencies to:

```toml
[lib]
name = "sim_server"
path = "src/lib.rs"

[[bin]]
name = "sim-server"
path = "src/main.rs"

[dependencies]
abutown-protocol = { path = "../protocol" }
sim-core = { path = "../sim-core" }
anyhow.workspace = true
axum.workspace = true
serde_json.workspace = true
tokio.workspace = true
tower-http.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

Keep the existing `[package]` and `[dev-dependencies]` sections, with `serde_json.workspace = true` removed from `[dev-dependencies]` if it now appears under `[dependencies]`.

- [ ] **Step 4: Create the library root**

Create `backend/crates/sim-server/src/lib.rs`:

```rust
pub mod app;
pub mod runtime;
```

- [ ] **Step 5: Create the in-memory runtime**

Create `backend/crates/sim-server/src/runtime.rs`:

```rust
use abutown_protocol::{
    ChunkCoordDto, ChunkSnapshotDto, HealthResponse, PROTOCOL_VERSION, ServerHelloDto,
    ServerMessageDto, TilePulseDeltaDto, WorldId, WorldSummaryDto,
};
use sim_core::{
    chunk::Chunk, ids::ChunkCoord, persistence::build_chunk_snapshot, scheduler::ChunkActivity,
    tile::TileKind,
};

const WORLD_ID: &str = "abutown-main";
const CHUNK_SIZE: u16 = 32;
const VISIBLE_CHUNK_COORD: ChunkCoord = ChunkCoord { x: 4, y: 4 };
const PULSE_STRIDE: u64 = 37;

#[derive(Debug)]
pub struct SimulationRuntime {
    world_id: WorldId,
    chunk: Chunk,
    tick: u64,
    version: u64,
}

impl SimulationRuntime {
    pub fn new() -> Self {
        let mut chunk = Chunk::new(VISIBLE_CHUNK_COORD, CHUNK_SIZE);
        chunk
            .set_tile_kind(0, TileKind::Road)
            .expect("seed tile index is valid for visible chunk");

        Self {
            world_id: WorldId(WORLD_ID.to_string()),
            chunk,
            tick: 0,
            version: 0,
        }
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            service: "abutown-sim".to_string(),
            world_id: self.world_id.clone(),
            ok: true,
            protocol_version: PROTOCOL_VERSION,
        }
    }

    pub fn world_summary(&self) -> WorldSummaryDto {
        WorldSummaryDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            chunk_size: self.chunk.chunk_size(),
            loaded_chunks: vec![ChunkCoordDto {
                x: self.chunk.coord().x,
                y: self.chunk.coord().y,
            }],
        }
    }

    pub fn chunk_snapshot(&self, coord: ChunkCoord) -> Option<ChunkSnapshotDto> {
        if coord != self.chunk.coord() {
            return None;
        }

        Some(build_chunk_snapshot(
            &self.world_id.0,
            &self.chunk,
            ChunkActivity::Active,
        ))
    }

    pub fn hello(&self) -> ServerMessageDto {
        ServerMessageDto::Hello(ServerHelloDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            chunk_size: self.chunk.chunk_size(),
        })
    }

    pub fn next_pulse(&mut self) -> ServerMessageDto {
        self.tick += 1;
        self.version += 1;
        let tile_count = u64::from(self.chunk.tile_count());
        let local_index = ((self.tick * PULSE_STRIDE) % tile_count) as u16;

        ServerMessageDto::TilePulse(TilePulseDeltaDto {
            protocol_version: PROTOCOL_VERSION,
            world_id: self.world_id.clone(),
            tick: self.tick,
            version: self.version,
            coord: ChunkCoordDto {
                x: self.chunk.coord().x,
                y: self.chunk.coord().y,
            },
            local_index,
        })
    }
}

impl Default for SimulationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_produces_monotonic_pulses_inside_seed_chunk() {
        let mut runtime = SimulationRuntime::new();

        let first = runtime.next_pulse();
        let second = runtime.next_pulse();

        let ServerMessageDto::TilePulse(first) = first else {
            panic!("first message should be a tile pulse");
        };
        let ServerMessageDto::TilePulse(second) = second else {
            panic!("second message should be a tile pulse");
        };

        assert_eq!(first.tick, 1);
        assert_eq!(first.version, 1);
        assert_eq!(first.coord, ChunkCoordDto { x: 4, y: 4 });
        assert!(first.local_index < 1024);
        assert_eq!(second.tick, 2);
        assert_eq!(second.version, 2);
        assert!(second.local_index < 1024);
        assert_ne!(first.local_index, second.local_index);
    }
}
```

- [ ] **Step 6: Replace app routing with state-backed handlers**

Replace `backend/crates/sim-server/src/app.rs` with:

```rust
use std::sync::Arc;

use abutown_protocol::{ChunkSnapshotDto, HealthResponse, WorldSummaryDto};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use sim_core::ids::ChunkCoord;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

use crate::runtime::SimulationRuntime;

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<Mutex<SimulationRuntime>>,
}

impl AppState {
    pub fn new(runtime: SimulationRuntime) -> Self {
        Self {
            runtime: Arc::new(Mutex::new(runtime)),
        }
    }

    pub(crate) fn runtime(&self) -> Arc<Mutex<SimulationRuntime>> {
        Arc::clone(&self.runtime)
    }
}

pub fn build_app() -> Router {
    build_app_with_runtime(SimulationRuntime::new())
}

pub fn build_app_with_runtime(runtime: SimulationRuntime) -> Router {
    let state = AppState::new(runtime);

    Router::new()
        .route("/health", get(health))
        .route("/world", get(world))
        .route("/chunks/{x}/{y}", get(chunk))
        .with_state(state)
        .layer(CorsLayer::permissive())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let runtime = state.runtime.lock().await;
    Json(runtime.health())
}

async fn world(State(state): State<AppState>) -> Json<WorldSummaryDto> {
    let runtime = state.runtime.lock().await;
    Json(runtime.world_summary())
}

async fn chunk(
    State(state): State<AppState>,
    Path((x, y)): Path<(i32, i32)>,
) -> Result<Json<ChunkSnapshotDto>, StatusCode> {
    let runtime = state.runtime.lock().await;
    runtime
        .chunk_snapshot(ChunkCoord { x, y })
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
```

- [ ] **Step 7: Run runtime and HTTP tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server
```

Expected: PASS for runtime unit tests and HTTP integration tests.

- [ ] **Step 8: Commit**

```bash
git add backend/crates/sim-server/Cargo.toml backend/crates/sim-server/src/lib.rs backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs backend/crates/sim-server/tests/http.rs
git commit -m "feat: add state-backed simulation runtime"
```

## Task 3: WebSocket Delta Stream

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/crates/sim-server/Cargo.toml`
- Modify: `backend/crates/sim-server/src/app.rs`
- Create: `backend/crates/sim-server/tests/websocket.rs`

- [ ] **Step 1: Add failing WebSocket integration test**

Create `backend/crates/sim-server/tests/websocket.rs`:

```rust
use std::time::Duration;

use abutown_protocol::ServerMessageDto;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;

use sim_server::app::build_app;

#[tokio::test]
async fn websocket_sends_hello_and_tile_pulse() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_app()).await.unwrap();
    });

    let url = format!("ws://{addr}/ws");
    let (mut stream, _) = connect_async(url).await.unwrap();

    let hello_text = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap()
        .to_string();
    let hello: ServerMessageDto = serde_json::from_str(&hello_text).unwrap();
    assert!(matches!(hello, ServerMessageDto::Hello(_)));

    let pulse_text = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap()
        .to_string();
    let pulse: ServerMessageDto = serde_json::from_str(&pulse_text).unwrap();

    let ServerMessageDto::TilePulse(delta) = pulse else {
        panic!("second websocket message should be a tile pulse");
    };
    assert_eq!(delta.world_id.0, "abutown-main");
    assert_eq!(delta.coord.x, 4);
    assert_eq!(delta.coord.y, 4);
    assert_eq!(delta.tick, 1);
    assert_eq!(delta.version, 1);
    assert!(delta.local_index < 1024);

    server.abort();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: FAIL because `/ws` is not routed and `futures-util` / `tokio-tungstenite` are not declared direct dev dependencies.

- [ ] **Step 3: Add workspace test dependencies**

In `backend/Cargo.toml`, add these entries under `[workspace.dependencies]`:

```toml
futures-util = "0.3"
tokio-tungstenite = "0.29"
```

In `backend/crates/sim-server/Cargo.toml`, add these entries under `[dev-dependencies]`:

```toml
futures-util.workspace = true
tokio-tungstenite.workspace = true
```

- [ ] **Step 4: Add WebSocket route and stream handler**

Modify `backend/crates/sim-server/src/app.rs` imports to include:

```rust
use std::{sync::Arc, time::Duration};

use abutown_protocol::{ChunkSnapshotDto, HealthResponse, ServerMessageDto, WorldSummaryDto};
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
```

Add the route inside `build_app_with_runtime`:

```rust
        .route("/ws", get(websocket))
```

Add these functions after `chunk`:

```rust
async fn websocket(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_world_deltas(socket, state))
}

async fn stream_world_deltas(mut socket: WebSocket, state: AppState) {
    let hello = {
        let runtime = state.runtime.lock().await;
        runtime.hello()
    };
    if send_server_message(&mut socket, hello).await.is_err() {
        return;
    }

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let pulse = {
            let runtime = state.runtime();
            let mut runtime = runtime.lock().await;
            runtime.next_pulse()
        };

        if send_server_message(&mut socket, pulse).await.is_err() {
            return;
        }
    }
}

async fn send_server_message(
    socket: &mut WebSocket,
    message: ServerMessageDto,
) -> Result<(), axum::Error> {
    let text = match serde_json::to_string(&message) {
        Ok(text) => text,
        Err(error) => {
            tracing::error!(%error, "failed to serialize websocket message");
            return Ok(());
        }
    };

    socket.send(Message::Text(text.into())).await
}
```

- [ ] **Step 5: Run WebSocket test**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml -p sim-server --test websocket
```

Expected: PASS and receive hello plus tile pulse.

- [ ] **Step 6: Run full backend tests**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend/Cargo.toml backend/Cargo.lock backend/crates/sim-server/Cargo.toml backend/crates/sim-server/src/app.rs backend/crates/sim-server/tests/websocket.rs
git commit -m "feat: stream visible backend deltas"
```

## Task 4: Frontend Backend State Bridge

**Files:**
- Create: `src/backend/protocol.ts`
- Create: `src/backend/backendState.ts`
- Create: `src/backend/backendClient.ts`
- Create: `tests/backend/backendState.test.ts`

- [ ] **Step 1: Write failing frontend state tests**

Create `tests/backend/backendState.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import {
  applyChunkSnapshot,
  applyHealth,
  applyServerMessage,
  applyWorldSummary,
  createInitialBackendOverlayState,
} from '../../src/backend/backendState';

describe('backend overlay state', () => {
  it('loads HTTP snapshot state without requiring websocket data', () => {
    let state = createInitialBackendOverlayState();

    state = applyHealth(state, {
      service: 'abutown-sim',
      world_id: 'abutown-main',
      ok: true,
      protocol_version: 1,
    });
    state = applyWorldSummary(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });
    state = applyChunkSnapshot(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      coord: { x: 4, y: 4 },
      chunk_state: 'active',
      chunk_version: 1,
      tile_count: 1024,
      dirty_tiles: [{ local_index: 0, kind: 'road', version: 1 }],
    });

    expect(state.status).toBe('snapshot');
    expect(state.worldId).toBe('abutown-main');
    expect(state.chunkSize).toBe(32);
    expect(state.loadedChunk?.coord).toEqual({ x: 4, y: 4 });
  });

  it('applies websocket tile pulses only when protocol and world match', () => {
    let state = createInitialBackendOverlayState();
    state = applyWorldSummary(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });

    state = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 4,
        version: 9,
        coord: { x: 4, y: 4 },
        local_index: 99,
      },
      1200,
    );

    expect(state.status).toBe('live');
    expect(state.latestTick).toBe(4);
    expect(state.latestVersion).toBe(9);
    expect(state.pulses).toHaveLength(1);
    expect(state.pulses[0]).toMatchObject({ localIndex: 99, receivedAtMs: 1200 });

    const afterWrongWorld = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'other-world',
        tick: 5,
        version: 10,
        coord: { x: 4, y: 4 },
        local_index: 100,
      },
      1300,
    );

    expect(afterWrongWorld.pulses).toHaveLength(1);
    expect(afterWrongWorld.warning).toBe('Ignored websocket message for other-world');
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
npm test -- tests/backend/backendState.test.ts
```

Expected: FAIL because frontend backend modules do not exist.

- [ ] **Step 3: Create TypeScript protocol guards**

Create `src/backend/protocol.ts`:

```ts
export const CLIENT_PROTOCOL_VERSION = 1;

export type ChunkCoordDto = {
  x: number;
  y: number;
};

export type HealthResponse = {
  service: string;
  world_id: string;
  ok: boolean;
  protocol_version: number;
};

export type WorldSummaryDto = {
  protocol_version: number;
  world_id: string;
  chunk_size: number;
  loaded_chunks: ChunkCoordDto[];
};

export type ChunkStateDto = 'asleep' | 'warm' | 'active' | 'hot';
export type TileKindDto = 'grass' | 'water' | 'road' | 'building_footprint';

export type TileMutationDto = {
  local_index: number;
  kind: TileKindDto;
  version: number;
};

export type ChunkSnapshotDto = {
  protocol_version: number;
  world_id: string;
  coord: ChunkCoordDto;
  chunk_state: ChunkStateDto;
  chunk_version: number;
  tile_count: number;
  dirty_tiles: TileMutationDto[];
};

export type ServerHelloMessage = {
  type: 'hello';
  protocol_version: number;
  world_id: string;
  chunk_size: number;
};

export type TilePulseMessage = {
  type: 'tile_pulse';
  protocol_version: number;
  world_id: string;
  tick: number;
  version: number;
  coord: ChunkCoordDto;
  local_index: number;
};

export type ServerErrorMessage = {
  type: 'error';
  protocol_version: number;
  world_id?: string;
  code: string;
  message: string;
};

export type ServerMessage = ServerHelloMessage | TilePulseMessage | ServerErrorMessage;

export function parseServerMessage(value: unknown): ServerMessage | undefined {
  if (!isRecord(value) || typeof value.type !== 'string') return undefined;
  if (value.type === 'hello' && isHello(value)) return value;
  if (value.type === 'tile_pulse' && isTilePulse(value)) return value;
  if (value.type === 'error' && isServerError(value)) return value;
  return undefined;
}

function isHello(value: Record<string, unknown>): value is ServerHelloMessage {
  return (
    value.type === 'hello' &&
    typeof value.protocol_version === 'number' &&
    typeof value.world_id === 'string' &&
    typeof value.chunk_size === 'number'
  );
}

function isTilePulse(value: Record<string, unknown>): value is TilePulseMessage {
  return (
    value.type === 'tile_pulse' &&
    typeof value.protocol_version === 'number' &&
    typeof value.world_id === 'string' &&
    typeof value.tick === 'number' &&
    typeof value.version === 'number' &&
    isChunkCoord(value.coord) &&
    typeof value.local_index === 'number'
  );
}

function isServerError(value: Record<string, unknown>): value is ServerErrorMessage {
  return (
    value.type === 'error' &&
    typeof value.protocol_version === 'number' &&
    (value.world_id === undefined || typeof value.world_id === 'string') &&
    typeof value.code === 'string' &&
    typeof value.message === 'string'
  );
}

function isChunkCoord(value: unknown): value is ChunkCoordDto {
  return isRecord(value) && typeof value.x === 'number' && typeof value.y === 'number';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
```

- [ ] **Step 4: Create pure backend state reducer**

Create `src/backend/backendState.ts`:

```ts
import {
  CLIENT_PROTOCOL_VERSION,
  type ChunkCoordDto,
  type ChunkSnapshotDto,
  type HealthResponse,
  type ServerMessage,
  type WorldSummaryDto,
} from './protocol';

export type BackendOverlayStatus = 'idle' | 'connecting' | 'snapshot' | 'live' | 'disconnected' | 'incompatible';

export type BackendPulse = {
  coord: ChunkCoordDto;
  localIndex: number;
  tick: number;
  version: number;
  receivedAtMs: number;
};

export type LoadedBackendChunk = {
  coord: ChunkCoordDto;
  state: string;
  version: number;
  tileCount: number;
};

export type BackendOverlayState = {
  status: BackendOverlayStatus;
  protocolVersion: number;
  worldId?: string;
  service?: string;
  ok: boolean;
  chunkSize?: number;
  loadedChunk?: LoadedBackendChunk;
  latestTick?: number;
  latestVersion?: number;
  pulses: BackendPulse[];
  warning?: string;
};

export function createInitialBackendOverlayState(): BackendOverlayState {
  return {
    status: 'idle',
    protocolVersion: CLIENT_PROTOCOL_VERSION,
    ok: false,
    pulses: [],
  };
}

export function markBackendConnecting(state: BackendOverlayState): BackendOverlayState {
  return { ...state, status: 'connecting', warning: undefined };
}

export function markBackendDisconnected(state: BackendOverlayState, warning: string): BackendOverlayState {
  return { ...state, status: 'disconnected', ok: false, warning };
}

export function applyHealth(state: BackendOverlayState, health: HealthResponse): BackendOverlayState {
  if (health.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      ok: false,
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${health.protocol_version}`,
    };
  }

  return {
    ...state,
    status: health.ok ? 'snapshot' : 'disconnected',
    service: health.service,
    worldId: health.world_id,
    ok: health.ok,
    warning: health.ok ? undefined : 'Backend health check failed',
  };
}

export function applyWorldSummary(state: BackendOverlayState, world: WorldSummaryDto): BackendOverlayState {
  if (world.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${world.protocol_version}`,
    };
  }

  return {
    ...state,
    worldId: world.world_id,
    chunkSize: world.chunk_size,
  };
}

export function applyChunkSnapshot(state: BackendOverlayState, snapshot: ChunkSnapshotDto): BackendOverlayState {
  if (snapshot.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${snapshot.protocol_version}`,
    };
  }

  return {
    ...state,
    status: state.status === 'live' ? 'live' : 'snapshot',
    worldId: snapshot.world_id,
    loadedChunk: {
      coord: snapshot.coord,
      state: snapshot.chunk_state,
      version: snapshot.chunk_version,
      tileCount: snapshot.tile_count,
    },
  };
}

export function applyServerMessage(
  state: BackendOverlayState,
  message: ServerMessage,
  receivedAtMs: number,
): BackendOverlayState {
  if (message.protocol_version !== CLIENT_PROTOCOL_VERSION) {
    return {
      ...state,
      status: 'incompatible',
      warning: `Protocol mismatch: client ${CLIENT_PROTOCOL_VERSION}, server ${message.protocol_version}`,
    };
  }

  if (message.type === 'hello') {
    return {
      ...state,
      status: 'live',
      ok: true,
      worldId: message.world_id,
      chunkSize: message.chunk_size,
      warning: undefined,
    };
  }

  if (message.type === 'error') {
    return {
      ...state,
      warning: `${message.code}: ${message.message}`,
    };
  }

  if (state.worldId !== undefined && message.world_id !== state.worldId) {
    return {
      ...state,
      warning: `Ignored websocket message for ${message.world_id}`,
    };
  }

  const pulse: BackendPulse = {
    coord: message.coord,
    localIndex: message.local_index,
    tick: message.tick,
    version: message.version,
    receivedAtMs,
  };

  return {
    ...state,
    status: 'live',
    ok: true,
    worldId: message.world_id,
    latestTick: message.tick,
    latestVersion: message.version,
    pulses: [pulse, ...state.pulses].slice(0, 8),
    warning: undefined,
  };
}
```

- [ ] **Step 5: Create browser backend client**

Create `src/backend/backendClient.ts`:

```ts
import {
  applyChunkSnapshot,
  applyHealth,
  applyServerMessage,
  applyWorldSummary,
  createInitialBackendOverlayState,
  markBackendConnecting,
  markBackendDisconnected,
  type BackendOverlayState,
} from './backendState';
import {
  parseServerMessage,
  type ChunkSnapshotDto,
  type HealthResponse,
  type WorldSummaryDto,
} from './protocol';

export type BackendBridgeOptions = {
  baseUrl?: string;
  onState: (state: BackendOverlayState) => void;
  now?: () => number;
  WebSocketCtor?: typeof WebSocket;
};

export type BackendBridge = {
  stop: () => void;
};

const DEFAULT_BASE_URL = 'http://127.0.0.1:8080';
const RECONNECT_DELAY_MS = 2500;

export function startBackendBridge(options: BackendBridgeOptions): BackendBridge {
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? import.meta.env.VITE_SIM_SERVER_URL ?? DEFAULT_BASE_URL);
  const now = options.now ?? (() => performance.now());
  const WebSocketCtor = options.WebSocketCtor ?? WebSocket;
  let stopped = false;
  let socket: WebSocket | undefined;
  let state = markBackendConnecting(createInitialBackendOverlayState());

  const publish = (next: BackendOverlayState): void => {
    state = next;
    options.onState(state);
  };

  publish(state);

  void loadAndConnect();

  async function loadAndConnect(): Promise<void> {
    try {
      const next = await loadSnapshot(baseUrl, state);
      if (stopped) return;
      publish(next);
      connectWebSocket();
    } catch (error: unknown) {
      if (stopped) return;
      publish(markBackendDisconnected(state, error instanceof Error ? error.message : 'Backend snapshot failed'));
      scheduleReconnect();
    }
  }

  function scheduleReconnect(): void {
    window.setTimeout(() => {
      if (!stopped) void loadAndConnect();
    }, RECONNECT_DELAY_MS);
  }

  function connectWebSocket(): void {
    socket?.close();
    socket = new WebSocketCtor(toWebSocketUrl(baseUrl, '/ws'));

    socket.addEventListener('message', (event) => {
      try {
        const parsed = parseServerMessage(JSON.parse(String(event.data)));
        if (!parsed) {
          publish({ ...state, warning: 'Ignored unknown websocket message' });
          return;
        }
        publish(applyServerMessage(state, parsed, now()));
      } catch {
        publish({ ...state, warning: 'Ignored malformed websocket message' });
      }
    });

    socket.addEventListener('close', () => {
      if (stopped) return;
      publish(markBackendDisconnected(state, 'Backend websocket disconnected'));
      scheduleReconnect();
    });

    socket.addEventListener('error', () => {
      if (stopped) return;
      publish(markBackendDisconnected(state, 'Backend websocket error'));
    });
  }

  return {
    stop: () => {
      stopped = true;
      socket?.close();
    },
  };
}

async function loadSnapshot(baseUrl: string, state: BackendOverlayState): Promise<BackendOverlayState> {
  const health = await fetchJson<HealthResponse>(`${baseUrl}/health`);
  let next = applyHealth(state, health);
  if (next.status === 'incompatible') return next;

  const world = await fetchJson<WorldSummaryDto>(`${baseUrl}/world`);
  next = applyWorldSummary(next, world);

  const firstChunk = world.loaded_chunks[0];
  if (!firstChunk) return next;

  const snapshot = await fetchJson<ChunkSnapshotDto>(`${baseUrl}/chunks/${firstChunk.x}/${firstChunk.y}`);
  return applyChunkSnapshot(next, snapshot);
}

async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} returned ${response.status}`);
  return response.json() as Promise<T>;
}

function normalizeBaseUrl(value: string): string {
  return value.endsWith('/') ? value.slice(0, -1) : value;
}

function toWebSocketUrl(baseUrl: string, path: string): string {
  const url = new URL(path, `${baseUrl}/`);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}
```

- [ ] **Step 6: Run frontend state tests**

Run:

```bash
npm test -- tests/backend/backendState.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/backend/protocol.ts src/backend/backendState.ts src/backend/backendClient.ts tests/backend/backendState.test.ts
git commit -m "feat: add browser backend state bridge"
```

## Task 5: Visible Canvas Overlay

**Files:**
- Create: `src/render/backendOverlay.ts`
- Create: `tests/render/backendOverlay.test.ts`
- Modify: `src/main.ts`

- [ ] **Step 1: Write failing overlay helper tests**

Create `tests/render/backendOverlay.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { activeBackendPulses, localIndexToWorldCoord } from '../../src/render/backendOverlay';

describe('backend overlay helpers', () => {
  it('maps chunk-local tile indices to world coordinates', () => {
    expect(localIndexToWorldCoord({ x: 0, y: 0 }, 32, 0)).toEqual({ x: 0, y: 0 });
    expect(localIndexToWorldCoord({ x: 0, y: 0 }, 32, 33)).toEqual({ x: 1, y: 1 });
    expect(localIndexToWorldCoord({ x: 2, y: 1 }, 32, 65)).toEqual({ x: 65, y: 34 });
  });

  it('keeps only visible pulse effects inside their lifetime', () => {
    const pulses = [
      { coord: { x: 0, y: 0 }, localIndex: 1, tick: 1, version: 1, receivedAtMs: 100 },
      { coord: { x: 0, y: 0 }, localIndex: 2, tick: 2, version: 2, receivedAtMs: 1300 },
    ];

    expect(activeBackendPulses(pulses, 1500).map((pulse) => pulse.localIndex)).toEqual([2]);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
npm test -- tests/render/backendOverlay.test.ts
```

Expected: FAIL because `backendOverlay.ts` does not exist.

- [ ] **Step 3: Create overlay renderer**

Create `src/render/backendOverlay.ts`:

```ts
import type { Coord } from '../city/worldTypes';
import type { BackendOverlayState, BackendPulse } from '../backend/backendState';

export const BACKEND_PULSE_LIFETIME_MS = 1400;

type ProjectIso = (coord: Coord) => Coord;

export function localIndexToWorldCoord(chunk: Coord, chunkSize: number, localIndex: number): Coord {
  return {
    x: chunk.x * chunkSize + (localIndex % chunkSize),
    y: chunk.y * chunkSize + Math.floor(localIndex / chunkSize),
  };
}

export function activeBackendPulses(pulses: readonly BackendPulse[], nowMs: number): BackendPulse[] {
  return pulses.filter((pulse) => nowMs - pulse.receivedAtMs < BACKEND_PULSE_LIFETIME_MS);
}

export function drawBackendWorldOverlay(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  projectIso: ProjectIso,
  tileWidth: number,
  tileHeight: number,
  nowMs: number,
): void {
  if (!state.loadedChunk || !state.chunkSize) return;

  drawChunkOutline(context, state, projectIso, tileWidth);
  drawPulseMarkers(context, state, projectIso, tileHeight, nowMs);
}

export function drawBackendStatusBadge(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  viewport: { width: number; height: number },
): void {
  const x = 14;
  const y = Math.max(14, viewport.height - 86);
  const statusColor = state.status === 'live' ? '#7df2b2' : state.status === 'snapshot' ? '#f3d37a' : '#ff8f8f';
  const lines = [
    `RUST ${state.status.toUpperCase()}`,
    state.worldId ? `world ${state.worldId}` : 'world offline',
    state.loadedChunk ? `chunk ${state.loadedChunk.coord.x}:${state.loadedChunk.coord.y} ${state.loadedChunk.state}` : 'chunk none',
    `tick ${state.latestTick ?? '-'} v${state.latestVersion ?? '-'}`,
  ];

  context.save();
  context.font = '12px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
  context.textBaseline = 'top';
  context.fillStyle = 'rgba(4, 12, 9, 0.78)';
  roundRect(context, x, y, 186, 64, 5);
  context.fill();
  context.strokeStyle = statusColor;
  context.lineWidth = 1;
  context.stroke();

  lines.forEach((line, index) => {
    context.fillStyle = index === 0 ? statusColor : 'rgba(222, 255, 235, 0.9)';
    context.fillText(line, x + 10, y + 8 + index * 14);
  });
  context.restore();
}

function drawChunkOutline(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  projectIso: ProjectIso,
  tileWidth: number,
): void {
  const chunk = state.loadedChunk;
  const chunkSize = state.chunkSize;
  if (!chunk || !chunkSize) return;

  const start = { x: chunk.coord.x * chunkSize, y: chunk.coord.y * chunkSize };
  const points = [
    projectIso(start),
    projectIso({ x: start.x + chunkSize, y: start.y }),
    projectIso({ x: start.x + chunkSize, y: start.y + chunkSize }),
    projectIso({ x: start.x, y: start.y + chunkSize }),
  ];

  context.save();
  context.strokeStyle = state.status === 'live' ? 'rgba(125, 242, 178, 0.85)' : 'rgba(243, 211, 122, 0.72)';
  context.lineWidth = Math.max(2, tileWidth / 18);
  context.setLineDash([10, 8]);
  context.beginPath();
  context.moveTo(points[0].x, points[0].y);
  for (const point of points.slice(1)) context.lineTo(point.x, point.y);
  context.closePath();
  context.stroke();
  context.restore();
}

function drawPulseMarkers(
  context: CanvasRenderingContext2D,
  state: BackendOverlayState,
  projectIso: ProjectIso,
  tileHeight: number,
  nowMs: number,
): void {
  const chunkSize = state.chunkSize;
  if (!chunkSize) return;

  for (const pulse of activeBackendPulses(state.pulses, nowMs)) {
    const age = nowMs - pulse.receivedAtMs;
    const t = Math.max(0, Math.min(1, age / BACKEND_PULSE_LIFETIME_MS));
    const coord = localIndexToWorldCoord(pulse.coord, chunkSize, pulse.localIndex);
    const point = projectIso(coord);
    const radius = 9 + t * 26;
    const alpha = 1 - t;

    context.save();
    context.globalAlpha = alpha;
    context.strokeStyle = '#ffd166';
    context.lineWidth = 3;
    context.beginPath();
    context.ellipse(point.x, point.y - tileHeight * 0.4, radius, radius * 0.48, 0, 0, Math.PI * 2);
    context.stroke();
    context.fillStyle = 'rgba(255, 209, 102, 0.28)';
    context.beginPath();
    context.arc(point.x, point.y - tileHeight * 0.4, 4, 0, Math.PI * 2);
    context.fill();
    context.restore();
  }
}

function roundRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  context.beginPath();
  context.moveTo(x + radius, y);
  context.lineTo(x + width - radius, y);
  context.quadraticCurveTo(x + width, y, x + width, y + radius);
  context.lineTo(x + width, y + height - radius);
  context.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
  context.lineTo(x + radius, y + height);
  context.quadraticCurveTo(x, y + height, x, y + height - radius);
  context.lineTo(x, y + radius);
  context.quadraticCurveTo(x, y, x + radius, y);
  context.closePath();
}
```

- [ ] **Step 4: Wire overlay state into `src/main.ts`**

Add these imports near the other imports:

```ts
import { startBackendBridge } from './backend/backendClient';
import { createInitialBackendOverlayState, type BackendOverlayState } from './backend/backendState';
import { drawBackendStatusBadge, drawBackendWorldOverlay } from './render/backendOverlay';
```

Add this state near `let cars: Car[] = [];`:

```ts
let backendOverlayState: BackendOverlayState = createInitialBackendOverlayState();
```

Add this line inside `boot()` after `attachCamera();` and before `canvas.dataset.ready = 'true';`:

```ts
  startBackendBridge({ onState: (state) => { backendOverlayState = state; } });
```

Modify `render()` so the backend status badge is drawn after the world transform is restored:

```ts
function render(): void {
  ctx.save();
  ctx.setTransform(window.devicePixelRatio || 1, 0, 0, window.devicePixelRatio || 1, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = '#050705';
  ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  drawScene({ x: 0, y: 0 });
  ctx.restore();
  drawBackendStatusBadge(ctx, backendOverlayState, { width: window.innerWidth, height: window.innerHeight });
}
```

Add this line inside `drawScene()` after the drawable loop and before `drawPerimeterMist();`:

```ts
  drawBackendWorldOverlay(ctx, backendOverlayState, iso, TILE_W, TILE_H, performance.now());
```

- [ ] **Step 5: Run overlay tests and TypeScript build**

Run:

```bash
npm test -- tests/render/backendOverlay.test.ts tests/backend/backendState.test.ts
npm run build
```

Expected: both tests PASS and TypeScript build PASS.

- [ ] **Step 6: Commit**

```bash
git add src/render/backendOverlay.ts tests/render/backendOverlay.test.ts src/main.ts
git commit -m "feat: render visible backend overlay"
```

## Task 6: Verification, README, And Manual Smoke

**Files:**
- Modify: `backend/README.md`

- [ ] **Step 1: Update backend README run instructions**

Append this section to `backend/README.md`:

````md
## Visible Backend Slice

Run the Rust authority server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

In a second terminal, run the Vite client:

```bash
npm run dev
```

Open the Vite URL. The city should render normally and show a `RUST LIVE` badge. Chunk `4:4` is outlined from the server snapshot, and a server-driven pulse appears from `/ws` roughly once per second.
````

- [ ] **Step 2: Run full backend verification**

Run:

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path backend/Cargo.toml --workspace
```

Expected: all commands PASS.

- [ ] **Step 3: Run full frontend verification**

Run:

```bash
npm test
npm run build
```

Expected: all tests PASS and production build PASS.

- [ ] **Step 4: Manual visible smoke**

Run the Rust server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

Run the Vite dev server in another terminal:

```bash
npm run dev
```

Open the Vite URL in the browser. Verify:

- city renders,
- `RUST LIVE` badge appears,
- badge moves from snapshot/disconnected to live when WebSocket connects,
- chunk `4:4` outline is visible,
- pulse marker appears and changes over time.

- [ ] **Step 5: Commit docs and final verification notes**

```bash
git add backend/README.md
git commit -m "docs: document visible backend slice"
```

If verification required code fixes, include only the touched files in the same commit when they directly belong to this task.

## Final Review

- [ ] Run `git status --short` and confirm only intentional tracked changes are committed.
- [ ] Confirm `.gitignore 2` remains untracked and untouched.
- [ ] Confirm the feature branch contains the visible backend slice commits.
- [ ] Prepare a concise summary with exact verification commands and results.

## Spec Coverage

- Visible Rust signal in existing browser scene: Task 5.
- HTTP snapshot path: Task 2 and Task 4.
- WebSocket delta stream: Task 3 and Task 4.
- JSON protocol messages: Task 1.
- Backend unavailable graceful mode: Task 4 and Task 5.
- No Supabase/auth/economy/full-map authority: no tasks introduce those systems.
