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

The first mutation ingress is `POST /commands`. It accepts versioned JSON client commands, validates them inside the Rust runtime, applies accepted changes to loaded hot state, appends an in-memory world event, and broadcasts that event to `/ws` subscribers.

Implemented command:

- `set_tile_kind`: changes one tile in one already-loaded chunk.

Current boundaries:

- Commands are unauthenticated local-development inputs.
- Commands only target loaded chunks.
- Accepted mutations are stored in an in-memory append-only event store.
- Supabase/Postgres, command idempotency, permissions, chunk loading, and recovery remain later slices.

Targeted commands:

```bash
cargo test --manifest-path backend/Cargo.toml -p abutown-protocol command_
cargo test --manifest-path backend/Cargo.toml -p sim-core events
cargo test --manifest-path backend/Cargo.toml -p sim-server command_
cargo test --manifest-path backend/Cargo.toml -p sim-server websocket_broadcasts_accepted_command_event
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

Frontend mobility bridge:

- Run `cargo run --manifest-path backend/Cargo.toml -p sim-server`.
- Run `npm run dev`.
- Open the Vite URL with `?mobility=1` to use the Vite same-origin proxy, or
  `?mobilityBackend=http://127.0.0.1:8080` to connect directly.
- Without either flag, the browser keeps the local city-only view and makes no
  mobility backend request.

## Snapshot Loop

The server also runs an in-memory snapshot loop every five seconds. It writes snapshots for all loaded chunks into the current process snapshot store and clears chunk dirty flags after each successful pass. This is the first persistence boundary; Supabase/Postgres adapters remain a later slice.

Design rules:

- Rust owns hot simulation state.
- Tiles are durable but live as dense chunk arrays in memory.
- ECS is for materialized dynamic entities, not every tile.
- Database writes stay outside fixed-tick hot paths.
