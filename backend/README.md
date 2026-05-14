# Abutown Backend

Rust authoritative simulation foundation for the single always-on `abutown-main` aquarium world.

Common commands:

```bash
cargo test --manifest-path backend/Cargo.toml --workspace
cargo fmt --manifest-path backend/Cargo.toml --all
cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Available after the server crate is added:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

## Visible Backend Slice

Run the Rust authority server:

```bash
cargo run --manifest-path backend/Cargo.toml -p sim-server
```

In a second terminal, run the Vite client:

```bash
npm run dev
```

Open the Vite URL. The city should render normally and show a `RUST LIVE` badge. Chunk `4:4` is outlined from the server snapshot, and server-driven pulses appear from `/ws` roughly once per second. The runtime currently loads three visible chunks (`4:4`, `5:4`, and `4:5`) and rotates broadcast pulses across them.

Current `/ws` ticking is driven by one server-side scheduler and broadcast to connected clients.

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

Design rules:

- Rust owns hot simulation state.
- Tiles are durable but live as dense chunk arrays in memory.
- ECS is for materialized dynamic entities, not every tile.
- Database writes stay outside fixed-tick hot paths.
