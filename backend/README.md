# Abutown Backend

Rust authoritative simulation foundation for the single always-on `abutown-main` aquarium world.

Common commands:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
cargo fmt --manifest-path backend/Cargo.toml --all
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Run the authority server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

The server entrypoint loads the repository-root `.env` and requires:

- `DATABASE_URL`: SQLx Postgres/Supabase connection string for `world_events`, `chunk_snapshots`, and `user_card_hands`.
- `SUPABASE_URL`: Supabase project URL used for JWT/JWKS authentication.

Other root `.env` keys currently have narrower ownership:

- `SUPABASE_ANON_KEY`: frontend login/client key; Rust persistence does not use it.
- `SUPABASE_SERVICE_ROLE_KEY`: intentionally unused by this Rust slice.
- `SUPABASE_JWKS_X` and `SUPABASE_JWKS_Y`: local key material present in the env file; Rust auth currently fetches JWKS from `SUPABASE_URL`.

## Runtime Surface

The server exposes the current backend runtime directly:

- `GET /health` returns service and protocol health.
- `GET /world` returns the loaded chunk summary.
- `GET /chunks/{x}/{y}` returns snapshots for loaded chunks.
- `GET /mobility` returns the seeded mobility snapshot.
- `POST /commands` accepts validated local-development commands.
- `GET /ws` streams a hello message, tile pulses, and mobility deltas.

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

The runtime currently loads three chunks (`4:4`, `5:4`, and `4:5`) and rotates tile pulses across them. `/ws` ticking is driven by one server-side scheduler and broadcast to connected clients.

The old frontend bridge described in the visible-slice plan is not present in this branch. The current Vite client still renders the local canvas world without consuming these backend endpoints.

## Command Event Boundary

The first mutation ingress is `POST /commands`. It accepts versioned JSON client commands, validates them inside the Rust runtime, appends accepted events through the configured event store, applies accepted changes to loaded hot state, and broadcasts those events to `/ws` subscribers.

Implemented command:

- `set_tile_kind`: changes one tile in one already-loaded chunk.

Current boundaries:

- Commands are unauthenticated local-development inputs.
- Commands only target loaded chunks.
- Accepted mutations are appended through the runtime event-store boundary before hot-state application and websocket broadcast.
- The server entrypoint uses `DATABASE_URL` for persistent `world_events`.
- Direct test/local app builders use explicit in-memory stores.
- Command idempotency, permissions, chunk loading, and recovery replay remain later slices.

Targeted commands:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol command_
cargo test --manifest-path backend/Cargo.toml -p sim-core events
cargo test --manifest-path backend/Cargo.toml -p sim-server postgres_events
cargo test --manifest-path backend/Cargo.toml -p sim-server command_
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_broadcasts_accepted_command_event
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_does_not_broadcast_failed_command_append
```

## Agent Mobility Foundation

The first mobility slice follows the local literature notes in
`docs/literature/agent-simulation/README.md`.

Architecture rules:

- Agents are people with plans and mobility state.
- Vehicles are separate traffic/transit entities with route, progress, capacity,
  dwell time, and occupants.
- Riding is represented by `AgentMobilityState::InVehicle`; the passenger
  position is derived from the vehicle position.
- Traffic behavior stays behind the vehicle layer. The initial slice uses
  deterministic route movement and stop dwell time only.

Targeted commands:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol mobility_
cargo test --manifest-path backend/Cargo.toml -p sim-core mobility
cargo test --manifest-path backend/Cargo.toml -p sim-server mobility_snapshot_is_available
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_sends_mobility_deltas_after_hello
```

Frontend agent mode:

- The Vite client no longer renders the seeded backend demo mobility marker.
- Existing Simutrans pedestrians are the local frontend agents and can be
  selected directly on the canvas.
- Backend mobility endpoints remain available for later authoritative
  simulation slices, but the current browser agent mode is pedestrian-driven.

## Snapshot Loop

The server runs a snapshot loop every five seconds. The configured server entrypoint writes snapshots for all loaded chunks to the Postgres `chunk_snapshots` table and clears chunk dirty flags only after successful writes. Direct test/local app builders use explicit in-memory snapshot stores.

Design rules:

- Rust owns hot simulation state.
- Tiles are durable but live as dense chunk arrays in memory.
- ECS is for materialized dynamic entities, not every tile.
- Database writes stay outside fixed-tick hot paths.
