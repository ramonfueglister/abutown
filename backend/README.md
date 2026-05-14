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

Open the Vite URL. The city should render normally and show a `RUST LIVE` badge. Chunk `4:4` is outlined from the server snapshot, and a server-driven pulse appears from `/ws` roughly once per second.

Design rules:

- Rust owns hot simulation state.
- Tiles are durable but live as dense chunk arrays in memory.
- ECS is for materialized dynamic entities, not every tile.
- Database writes stay outside fixed-tick hot paths.
