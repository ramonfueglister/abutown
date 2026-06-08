# syntax=docker/dockerfile:1

# ---- Builder: compile the release sim-server binary ----
FROM rust:1-bookworm AS builder
WORKDIR /build
# The backend is a self-contained cargo workspace (backend/Cargo.toml is the root).
COPY backend ./backend
RUN cargo build --release --manifest-path backend/Cargo.toml -p sim-server

# ---- Runtime: slim image with the binary, the world bundle, and the CA cert ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/backend/target/release/sim-server /app/sim-server
COPY data/worlds/abutopia /app/data/worlds/abutopia
COPY deploy/supabase-prod-ca.crt /app/certs/supabase-ca.crt
# ABUTOWN_BASE_WORLD_PATH default is "data/worlds/abutopia" relative to CWD (/app).
ENV LISTEN_HOST=0.0.0.0 \
    LISTEN_PORT=8080 \
    PGSSLROOTCERT=/app/certs/supabase-ca.crt \
    RUST_LOG=warn,sim_server=info,economy::liveness=info
EXPOSE 8080
CMD ["/app/sim-server"]
