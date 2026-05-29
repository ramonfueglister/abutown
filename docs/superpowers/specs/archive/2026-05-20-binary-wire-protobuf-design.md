# Binary Wire Protocol via Protobuf

**Date:** 2026-05-20
**Status:** Spec
**Author:** Claude (with @ramonfueglister)

## §1 — Goal & Success Criterion

Replace JSON wire encoding (WS + HTTP) with Protocol Buffers (proto3). Schema
lives in `backend/crates/protocol/proto/abutown.proto` as the single source
of truth. Rust types are codegen'd via `prost-build` in `build.rs`. TS types
are codegen'd via `@bufbuild/protoc-gen-es`. DB persistence (mobility
snapshots, chunk snapshots in Postgres JSONB) stays JSON — only the wire
format changes.

### Acceptance

- New wire-size bench `protocol/benches/wire_size` reports protobuf
  encoding ≥ 3× smaller than the equivalent JSON for a representative
  `MobilityChunkDelta` payload (50 agents). CI gate.
- Browser smoke `scripts/smoke-7b.mjs` 9/9 green with binary WS frames.
- 158/158 vitest, all cargo workspace tests, clippy `-D warnings`, tsc all
  clean.
- Frontend bundle size increase from `@bufbuild/protobuf` ≤ 50 KB
  gzipped. Recorded in commit message.
- `tick_100k_all_active` bench within ±5 % of pre-migration baseline.
- HTTP `curl /world` returns `application/x-protobuf`. Debug recipe
  documented in `progress.md`.
- Zero `serde_json::to_string` / `serde_json::from_str` calls on the wire
  path (allowed in DB persistence + tests + debug examples).

### Out of scope

- DB schema changes — JSONB stays
- Wire formats other than Protobuf (MessagePack, FlatBuffers, etc. were
  considered and rejected during brainstorming)
- JSON fallback / content negotiation on HTTP — binary only
- Wire versioning beyond protobuf's built-in field tags
- Migration shim period — atomic cutover
- Removing `serde` entirely; storage DTOs in sim-core still need it

## §2 — Architecture

```
        ┌────────────────────────────────────────────────────────┐
        │ backend/crates/protocol/proto/abutown.proto             │
        │   ← single source of truth (canonical schema)           │
        └────────────────────────────────────────────────────────┘
              │                                       │
        prost-build (build.rs)              @bufbuild/protoc-gen-es
              │                                       │
              ▼                                       ▼
   ┌──────────────────────┐         ┌─────────────────────────────┐
   │ Rust types:          │         │ TS types:                   │
   │  abutown_protocol::* │         │  src/backend/proto/*.ts     │
   │  (re-exported)       │         │  (gitignored, regenerated)  │
   └──────────────────────┘         └─────────────────────────────┘
              │                                       │
   ┌──────────────────────┐         ┌─────────────────────────────┐
   │ Backend:             │ binary  │ Frontend:                   │
   │  WS Message::Binary  │◄───────►│  WS: binaryType=arraybuffer │
   │  HTTP Bytes responder│         │  HTTP fetch → arrayBuffer   │
   │  prost encode/decode │         │  protobuf-es decode         │
   └──────────────────────┘         └─────────────────────────────┘
```

### New files

- `backend/crates/protocol/proto/abutown.proto` — schema (canonical).
- `backend/crates/protocol/build.rs` — `prost_build::compile_protos`
  call. Generates Rust code into `OUT_DIR`.
- `buf.yaml` and `buf.gen.yaml` at repo root — buf config pointing at
  the proto directory.
- `scripts/generate-proto-ts.mjs` — invokes `buf generate` to produce
  TS code under `src/backend/proto/`.
- `src/backend/proto/` directory (gitignored, codegen output).
- `backend/crates/protocol/benches/wire_size.rs` — JSON-vs-Protobuf
  size comparison bench.

### Removed files / sections

- `src/backend/mobilityProtocol.ts` — manual JSON parsers /
  encoders are replaced by codegen. The file collapses to a thin
  re-export layer.
- Most `#[derive(Serialize, Deserialize)]` derives on
  `abutown-protocol::*` DTOs that are *only* used on the wire.

### Stays unchanged

- DB persistence types (used by `MobilitySnapshotStore`,
  `ChunkSnapshotStore`) — these write JSON to JSONB columns and need to
  stay queryable from `psql`. We split "wire DTO" (protobuf, in
  `abutown-protocol`) from "storage DTO" (serde-JSON, in
  `sim-core/src/mobility/dto.rs` or similar). Explicit `From`/`Into`
  conversions between the two.

### Wire encoding

- **WS:** frames are `Message::Binary(Vec<u8>)` carrying
  `prost::Message::encode_to_vec`. Frontend reads
  `MessageEvent<ArrayBuffer>` (after setting
  `socket.binaryType = 'arraybuffer'`).
- **HTTP:** all endpoints return
  `Content-Type: application/x-protobuf`. Frontend uses
  `fetch(url).then(r => r.arrayBuffer())`. Request bodies
  (`POST /commands`) carry the same content type.

## §3 — Schema Design (`abutown.proto`)

Proto3. `package abutown.v1` (allows future v2 schemas side-by-side).
Field tags are the long-term backward-compatibility guarantee: never
renumber, never reuse.

```protobuf
syntax = "proto3";
package abutown.v1;

// === Primitives ===

message ChunkCoord { sint32 x = 1; sint32 y = 2; }
message WorldCoord { float x = 1; float y = 2; }

enum Direction {
  DIRECTION_UNSPECIFIED = 0;
  DIRECTION_N = 1; DIRECTION_NE = 2; DIRECTION_E = 3; DIRECTION_SE = 4;
  DIRECTION_S = 5; DIRECTION_SW = 6; DIRECTION_W = 7; DIRECTION_NW = 8;
}

enum ChunkState {
  CHUNK_STATE_UNSPECIFIED = 0;
  CHUNK_STATE_ASLEEP = 1;
  CHUNK_STATE_WARM = 2;
  CHUNK_STATE_ACTIVE = 3;
  CHUNK_STATE_HOT = 4;
}

enum TileKind {
  TILE_KIND_UNSPECIFIED = 0;
  TILE_KIND_ROAD = 1;
  TILE_KIND_WATER = 2;
  TILE_KIND_BUILDING_FOOTPRINT = 3;
  // ... full set per existing TileKindDto
}

enum VehicleKind {
  VEHICLE_KIND_UNSPECIFIED = 0;
  VEHICLE_KIND_CAR = 1;
  VEHICLE_KIND_TRAM = 2;
}

// === Server → Client envelope ===

message ServerMessage {
  oneof body {
    Hello hello = 1;
    TilePulse tile_pulse = 2;
    MobilityChunkDelta mobility_chunk_delta = 3;
    MobilityChunkSnapshot mobility_chunk_snapshot = 4;
    WorldEvent world_event = 5;
  }
}

message Hello {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint32 chunk_size = 3;
}

message TilePulse {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  uint64 version = 4;
  ChunkCoord coord = 5;
  uint32 local_index = 6;
}

message MobilityChunkDelta {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  ChunkCoord chunk = 4;
  repeated AgentMobility changed_agents = 5;
  repeated VehicleMobility changed_vehicles = 6;
  repeated string left_agents = 7;
  repeated string left_vehicles = 8;
}

message MobilityChunkSnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  ChunkCoord chunk = 4;
  repeated AgentMobility agents = 5;
  repeated VehicleMobility vehicles = 6;
}

message WorldEvent {
  oneof event {
    TileKindSetEvent tile_kind_set = 1;
  }
}

message TileKindSetEvent {
  string command_id = 1;
  ChunkCoord coord = 2;
  uint32 local_index = 3;
  TileKind kind = 4;
}

// === Client → Server envelope ===

message ClientMessage {
  oneof body {
    ChunkSubscribe chunk_subscribe = 1;
    ChunkUnsubscribe chunk_unsubscribe = 2;
    // ClientCommand reserved for future WS-side command path; the HTTP
    // /commands endpoint uses the ClientCommand message directly as
    // body.
  }
}

message ChunkSubscribe {
  uint32 protocol_version = 1;
  repeated ChunkCoord coords = 2;
}

message ChunkUnsubscribe {
  uint32 protocol_version = 1;
  repeated ChunkCoord coords = 2;
}

// === HTTP request/response bodies ===

message ClientCommand {
  oneof command {
    SetTileKindCommand set_tile_kind = 1;
  }
}

message SetTileKindCommand {
  uint32 protocol_version = 1;
  string world_id = 2;
  string command_id = 3;
  ChunkCoord coord = 4;
  uint32 local_index = 5;
  TileKind kind = 6;
}

message CommandResponse {
  oneof outcome {
    CommandAccepted accepted = 1;
    CommandRejected rejected = 2;
  }
}

message CommandAccepted { WorldEvent event = 1; }
message CommandRejected { string reason = 1; }

message HealthResponse {
  uint32 protocol_version = 1;
  string service = 2;
  string world_id = 3;
  bool ok = 4;
}

message WorldSummary {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint32 chunk_size = 3;
  repeated ChunkCoord loaded_chunks = 4;
  uint32 tick_period_ms = 5;
}

message ChunkSnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  ChunkCoord coord = 3;
  uint32 tile_count = 4;
  ChunkState chunk_state = 5;
  repeated TileMutation tiles = 6;
}

message TileMutation {
  uint32 local_index = 1;
  TileKind kind = 2;
}

message MobilitySnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  repeated AgentMobility agents = 4;
  repeated VehicleMobility vehicles = 5;
  repeated Stop stops = 6;
  repeated Route routes = 7;
}

// === Mobility entity DTOs ===

message AgentMobility {
  string id = 1;
  AgentState state = 2;
  WorldCoord world_coord = 3;
  Direction direction = 4;
  string sprite_key = 5;
  uint32 plan_cursor = 6;
}

message AgentState {
  oneof state {
    Walking walking = 1;
    WaitingAtStop waiting_at_stop = 2;
    InVehicle in_vehicle = 3;
    Boarding boarding = 4;
    Alighting alighting = 5;
    AtActivity at_activity = 6;
  }
}

message Walking { string link_id = 1; float progress = 2; }
message WaitingAtStop { string stop_id = 1; }
message InVehicle { string vehicle_id = 1; uint32 seat_index = 2; }
message Boarding { string vehicle_id = 1; string stop_id = 2; }
message Alighting { string vehicle_id = 1; string stop_id = 2; }
message AtActivity { string activity_id = 1; }

message VehicleMobility {
  string id = 1;
  VehicleKind kind = 2;
  string route_id = 3;
  uint32 link_index = 4;
  float progress = 5;
  uint32 capacity = 6;
  repeated string occupants = 7;
  WorldCoord world_coord = 8;
  Direction direction = 9;
  string sprite_key = 10;
}

message Stop {
  string id = 1;
  string route_id = 2;
  uint32 link_index = 3;
  float progress = 4;
  repeated string waiting_agents = 5;
}

message Route {
  string id = 1;
  repeated string links = 2;
  repeated string stops = 3;
}
```

### Schema conventions

- **Enum prefix** (`DIRECTION_N` not `N`): protobuf convention prevents
  symbol collisions in cross-language codegen. TS codegen produces
  unprefixed enum members.
- **`UNSPECIFIED = 0` for every enum**: required by proto3 (default
  value). Consumers reject `UNSPECIFIED` as a data error — it signals
  either an uninitialized field or a wire-version mismatch.
- **`oneof` for tagged enums**: `ServerMessage.body`, `AgentState.state`,
  `WorldEvent.event`, `ClientMessage.body`, `ClientCommand.command`,
  `CommandResponse.outcome`. Rust codegen produces variants on a public
  `Body` (etc.) enum; TS codegen produces discriminated unions with
  `.case` and `.value`.
- **`sint32` for ChunkCoord** (not `int32`): chunk coords are signed
  and frequently small; `sint32` zigzag-encoding is more compact for
  small negatives.

### Migration mapping (Rust)

The current serde DTOs in `abutown-protocol/src/lib.rs` are replaced by
prost-generated types from `proto/abutown.proto`. Field names stay
identical (snake_case). Field-tagged enums map to prost's nested
`Body`/`State`/`Event` enums via `oneof`. The crate's `lib.rs` becomes:

```rust
// Generated prost module via build.rs.
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/abutown.v1.rs"));
}

// Re-exports keeping the existing public names where possible.
pub use v1::ServerMessage;
pub use v1::ClientMessage;
pub use v1::ChunkCoord;
// ...

pub const PROTOCOL_VERSION: u32 = 16;
```

## §4 — Per-Site Migration

### Backend

| Site | JSON today | Protobuf morgen |
|---|---|---|
| `chunk_delta_to_dto` (app.rs) | builds `MobilityChunkDeltaDto` (serde) | builds `v1::MobilityChunkDelta` (prost) |
| `chunk_snapshot_to_dto` | builds `MobilityChunkSnapshotDto` | builds `v1::MobilityChunkSnapshot` |
| `send_server_message` | `serde_json::to_string(&msg)?; socket.send(Message::Text(text.into()))` | `prost::Message::encode_to_vec(&msg); socket.send(Message::Binary(bytes.into()))` |
| WS recv arm | `Message::Text` + `serde_json::from_str::<ClientMessageDto>` | `Message::Binary` + `v1::ClientMessage::decode(bytes)` |
| HTTP `/health`, `/world`, `/mobility`, `/chunks/{x}/{y}` | `Json<T>` axum extractor | Custom `Bytes` responder; helper `fn proto_response<M: prost::Message>(msg: M) -> Response` |
| HTTP `POST /commands` body | `Json<ClientCommandDto>` extractor | Custom `Bytes` extractor that calls `v1::ClientCommand::decode` |
| HTTP error responses | `Json(json!({"error": ...}))` | encoded `CommandRejected` message or HTTP-only status code with empty body |
| `MobilitySnapshotStore::write` / `read` | unchanged — still serde-JSON | unchanged |
| `runtime_view::*` (internal types) | unchanged | unchanged |

### Frontend

| Site | JSON today | Protobuf morgen |
|---|---|---|
| `mobilityClient.ts` socket setup | implicit text | `socket.binaryType = 'arraybuffer'` immediately after construction |
| `socket.onmessage` | `event.data: string`, `JSON.parse`, `parseServerMessage` | `event.data: ArrayBuffer`, `ServerMessage.fromBinary(new Uint8Array(data))` |
| `chunkSubscriptionClient.ts` send | `JSON.stringify({type:'chunk_subscribe', ...})` | `new ClientMessage({ body: { case: 'chunkSubscribe', value: {coords} } }).toBinary()` |
| `mobilityProtocol.ts::parseServerMessage` | manual TS parser | DELETE |
| `mobilityProtocol.ts::encodeClientMessage` | `JSON.stringify` | DELETE; callers use prost-es classes directly |
| HTTP `fetch` callers (if any) | `await r.json()` | `await r.arrayBuffer()` → proto decode |
| Unit-test JSON fixtures | string literals | typed proto constructors |

### Build tooling

- `prost-build` and `prost-types` added to `protocol/Cargo.toml`
  build-deps.
- `protocol/build.rs` calls
  `prost_build::compile_protos(&["proto/abutown.proto"], &["proto/"])`.
- `buf.yaml` + `buf.gen.yaml` at repo root; `buf.gen.yaml` plugins:
  `@bufbuild/protoc-gen-es` outputting to `src/backend/proto/`.
- `scripts/generate-proto-ts.mjs` runs `buf generate`.
- `package.json` scripts: `"generate:proto": "node scripts/generate-proto-ts.mjs"`,
  `"build": "npm run generate:proto && node scripts/build.mjs"`.
- `package.json` deps: `@bufbuild/protobuf` runtime;
  devDeps: `@bufbuild/buf`, `@bufbuild/protoc-gen-es`.
- `.gitignore`: `src/backend/proto/`.
- `tsconfig.json`: ensure `src/backend/proto/**` is in `include` but
  excluded from lint rules that don't apply to generated code.

### Wire versioning

Protobuf field tags handle most backward-compat needs: clients that
don't know a tag silently drop it; servers that don't know a tag get a
default value. `protocol_version` becomes informational only (logged on
`Hello`, not used for routing). Breaking changes — renumbered tags,
removed required messages — still require coordinated rollout. `buf
breaking` lint in CI guards this.

### Curl debug workflow

Without JSON we lose ergonomic curl. Document the recipe in
`progress.md`:

```bash
# Inspect a wire response with protoc / buf:
curl -s http://127.0.0.1:8080/world | protoc --decode=abutown.v1.WorldSummary -I backend/crates/protocol/proto abutown.proto

# Or with buf:
buf curl http://127.0.0.1:8080/world --schema backend/crates/protocol/proto
```

## §5 — Tests & Verification

### New tests

1. `proto_roundtrip_all_server_messages` (Rust) — for every
   `ServerMessage::Body` variant: construct → `encode_to_vec` →
   `decode` → assert equal. ~10 cases.
2. `proto_roundtrip_all_client_messages` (Rust) — mirror.
3. `wire_size_comparison_bench` (criterion) — JSON vs protobuf size for
   a 50-agent `MobilityChunkDelta`. Logs both sizes. Gates the protobuf
   ≥ 3× smaller.
4. `unknown_field_is_ignored` (Rust) — encode a future message via raw
   bytes (manual proto encoding) containing field tag 99, decode with
   current schema, assert no error and known fields populated.
5. `proto/server-message.test.ts` (TS) — roundtrip + decode known
   fixed-bytes fixtures (paste from Rust encoder output).
6. `mobilityClient.test.ts` (existing, updated) — mock WS emits
   ArrayBuffer, client decodes correctly.

### Removed tests

- `mobilityProtocol.test.ts` JSON-encoder tests — function deleted.

### Modified tests

- `tests/http.rs` integration tests — body assertions go through
  `v1::WorldSummary::decode` etc. Wire format is binary.
- `tests/websocket.rs` integration tests — connect with binary
  expectation; sends/recvs Binary frames; parses via prost.
- Browser smoke `scripts/smoke-7b.mjs` — Playwright `framereceived.payload`
  is now `Buffer | string`; the script decodes the Buffer via
  `ServerMessage.fromBinary` and counts via the `oneof` `case`
  discriminator instead of `parsed.type` string.

### Persistence tests

- `phase3-mobility-snapshot.json` byte-stable roundtrip — STAYS, but
  the test name is updated to make clear it covers *storage* (JSONB),
  not wire.

### Verification gates

- `cargo test` workspace
- `cargo clippy --all-targets -- -D warnings`
- `buf lint` and `buf breaking` (compare HEAD vs `main`)
- `npm run generate:proto && npx tsc --noEmit` — codegen must precede typecheck
- `npx vitest run`
- `node scripts/smoke-7b.mjs` 9/9
- Wire-size bench: ratio ≥ 3× (gate)
- Bundle size delta: report KB increase from `@bufbuild/protobuf`

## §6 — Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Codegen drift (proto edited, codegen forgotten) | CI runs `buf generate` and `cargo build`; both validate freshness. Generated files gitignored so drift can't be committed. |
| Breaking schema change without version coordination | `buf breaking` lint in CI fails on tag-renumber / type-change. |
| Bundle-size blow-up | `@bufbuild/protobuf` is ~12 KB gzipped (much smaller than `protobufjs`). Bench bundle before/after; reject if > 50 KB delta. |
| Browser compat: `WebSocket.binaryType` | Set `'arraybuffer'` on construction. Supported in all evergreen browsers since 2014. |
| Persistence-DB drift from wire-DTO | Separate `storage_dto.rs` types in sim-core that derive `serde`. Wire types live in `abutown-protocol`. Explicit `From`/`Into` conversions; tests verify equivalence. |
| `UNSPECIFIED` enum default treated as valid | Consumers reject `UNSPECIFIED` as a data error; emit `tracing::warn!` and skip. |
| Curl-debug workflow lost | Document `protoc --decode_raw` and `buf curl` recipes in `progress.md`. |
| Generated TS files in `src/backend/proto/` confuse IDE / lint | `.gitignore` entry. `tsconfig.json` includes them for typecheck but `.eslintignore` (if used) excludes from lint. |
| WS frame `payload` type changes in Playwright smoke | Add explicit binary-decode branch in smoke; ensure smoke parses `case` discriminator. |
| Migration touches many files; high merge-conflict risk | Land in sequence (per §7), each commit independently verifiable. No feature flag — atomic per-commit. |

## §7 — Implementation Order

1. **Scaffolding (Task 1):** Add `buf.yaml`, `buf.gen.yaml`, empty
   `abutown.proto`, `protocol/build.rs`, `@bufbuild/protobuf` dep,
   generate-proto-ts script. Verify Rust codegen runs (build.rs).
   Verify TS codegen runs (`npm run generate:proto`). No type usage yet.

2. **Schema (Task 2):** Write the full `abutown.proto` per §3. Wire it
   into Rust by re-exporting `pub mod v1` from
   `abutown-protocol/src/lib.rs`. Add Rust roundtrip tests
   (`proto_roundtrip_all_server_messages`, `…_client_messages`).
   `cargo test -p abutown-protocol` green.

3. **TS codegen + roundtrip (Task 3):** Run TS codegen. Add
   `proto/server-message.test.ts` with roundtrip + known-bytes
   fixtures. `npx vitest run` green.

4. **Backend WS migration (Task 4):** Replace `Message::Text` /
   `serde_json` on the WS path. `send_server_message`,
   `chunk_delta_to_dto`, `chunk_snapshot_to_dto`, and the WS handler's
   `recv` arm switch to prost. HTTP endpoints stay JSON for this task.
   Verify `tests/websocket.rs` updated and green.

5. **Frontend WS migration (Task 5):** `socket.binaryType =
   'arraybuffer'`. Replace `parseServerMessage` with
   `ServerMessage.fromBinary`. Replace `encodeClientMessage` with
   `ClientMessage(...).toBinary()`. Smoke 9/9.

6. **HTTP endpoints (Task 6):** Migrate `/health`, `/world`,
   `/mobility`, `/chunks/{x}/{y}`, `POST /commands` to binary. Update
   `tests/http.rs`. Helper functions for proto-bytes responder.

7. **Cleanup (Task 7):** Delete `mobilityProtocol.ts`. Remove serde
   `Serialize`/`Deserialize` derives from wire-only DTOs in
   `abutown-protocol` (any that survived). Confirm no
   `serde_json::to_string` / `from_str` on the wire path
   (`grep`-based check).

8. **Final verification + bench + progress.md (Task 8):** Run all
   gates. Wire-size bench. Record bundle delta. Browser smoke 9/9 with
   binary frames. Progress entry.

## §8 — Rollback

Each commit (per §7) is independently verifiable and revertible. If any
step regresses, revert that commit. The DB schema is untouched, so
rollback never needs DB work. The atomic-cutover design means no shim
period — if Phase 7c-style wire is needed back, revert all 7 commits.
