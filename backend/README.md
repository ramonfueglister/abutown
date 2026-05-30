# Abutown Backend

Rust authoritative simulation foundation for the current always-on Abutopia world.

Common commands:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
cargo fmt --manifest-path backend/Cargo.toml --all
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Run the authority server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server --bin sim-server
```

The server entrypoint loads the repository-root `.env` and requires:

- `DATABASE_URL`: SQLx Postgres/Supabase connection string for `world_events`, `chunk_snapshots`, and `user_card_hands`.
- `SUPABASE_URL`: Supabase project URL used for JWT/JWKS authentication.
- `CORS_ALLOWED_ORIGINS`: comma-separated browser origins allowed to call the API. Local Vite uses `http://127.0.0.1:5173`.

Other root `.env` keys currently have narrower ownership:

- `VITE_SUPABASE_URL` and `VITE_SUPABASE_PUBLISHABLE_KEY`: frontend login/client config. Only a low-privilege publishable key belongs here.
- `VITE_ABUTOWN_BACKEND_URL`: optional frontend backend URL; defaults to `http://127.0.0.1:8080`.

Do not add non-Vite publishable keys, service-role keys, or copied JWKS material
to the local `.env`; this backend reads only the keys listed above.

## Supabase Setup

Local secrets live in the repository-root `.env`. The file is ignored by Git and should be mode `0600`. Do not commit `.env`, `supabase/.temp/`, database passwords, `sb_secret_*`, or service-role keys.

Use Supabase's session-pooler connection string for `DATABASE_URL` when running the persistent local backend:

```bash
DATABASE_URL=postgresql://postgres.<project-ref>:<db-password>@aws-1-<region>.pooler.supabase.com:5432/postgres?sslmode=require
```

Session pooler mode supports IPv4 and keeps prepared statements available for SQLx. Avoid transaction-pooler URLs for this server unless SQLx prepared statements are explicitly disabled. Direct database URLs are fine only where IPv6 to `db.<project-ref>.supabase.co` works.

The repo includes `supabase/config.toml` for local Supabase CLI configuration. The link metadata in `supabase/.temp/` is local state and ignored.

## Runtime Surface

The server exposes the current backend runtime directly:

- `GET /health` returns service and protocol health.
- `GET /world` returns the loaded chunk summary.
- `GET /chunks/{x}/{y}` returns snapshots for loaded chunks.
- `GET /mobility` returns the seeded mobility snapshot.
- `POST /commands` accepts validated local-development commands.
- `GET /ws` streams a hello message, tile pulses, and mobility deltas.

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server --bin sim-server
```

The runtime loads the authored Abutopia base-world chunks and broadcasts
mobility/world deltas from one server-side scheduler. The Vite client requires
the backend and consumes `/world`, `/mobility`, and `/ws`; there is no production
frontend fallback world.

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

- The Vite client renders authoritative backend mobility from `/mobility` and
  `/ws`.
- The current Abutopia seed exposes 300 walking agents and no vehicles.
- Browser smoke must start the backend; there is no local canvas fallback world.

## Snapshot Loop

The server runs a snapshot loop every five seconds. The configured server entrypoint writes snapshots for all loaded chunks to the Postgres `chunk_snapshots` table and clears chunk dirty flags only after successful writes. Direct test/local app builders use explicit in-memory snapshot stores.

Design rules:

- Rust owns hot simulation state.
- Tiles are durable but live as dense chunk arrays in memory.
- ECS is for materialized dynamic entities, not every tile.
- Database writes stay outside fixed-tick hot paths.
