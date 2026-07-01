# syntax=docker/dockerfile:1

# ---- Builder: compile the release sim-server binary ----
FROM rust:1-bookworm AS builder
# protobuf-compiler: the abutown-protocol build script runs prost-build, which
# needs `protoc` (libprotoc 3.20+) in PATH. bookworm ships 3.21.
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /build
# The backend is a self-contained cargo workspace (backend/Cargo.toml is the root).
COPY backend ./backend
RUN cargo build --release --manifest-path backend/Cargo.toml -p sim-server

# ---- Runtime: slim image with the binary and the CA cert ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/backend/target/release/sim-server /app/sim-server
COPY deploy/supabase-prod-ca.crt /app/certs/supabase-ca.crt
ENV LISTEN_HOST=0.0.0.0 \
    LISTEN_PORT=8080 \
    PGSSLROOTCERT=/app/certs/supabase-ca.crt \
    RUST_LOG=warn,sim_server=info
EXPOSE 8080
CMD ["/app/sim-server"]
