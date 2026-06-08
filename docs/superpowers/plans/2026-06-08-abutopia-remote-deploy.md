# Abutopia Remote Deploy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the deploy artifacts (one small code change + Dockerfile/fly.toml/cert/runbook) so the shared abutopia `sim-server` can run as a single public Fly.io instance over `wss://`, with the static frontend on Vercel and persistence on Supabase `:5432`.

**Architecture:** One stateful `sim-server` (single writer) in a container binding `0.0.0.0:8080`; Fly terminates TLS; browsers load the Vercel static build and connect `wss://<app>.fly.dev`. Only one production-code change — a bindable `LISTEN_HOST` — everything else is deploy config. Spec: `docs/superpowers/specs/2026-06-08-abutopia-remote-deploy-design.md`.

**Tech Stack:** Rust (axum/sqlx, edition 2024 → Rust ≥1.85), Docker (multi-stage), Fly.io, Vercel, Supabase Postgres (`:5432` session pooler, `verify-full`).

---

## File Structure

- **Modify** `backend/crates/sim-server/src/main.rs` — add `resolve_listen_addr(host, port)` helper + `LISTEN_HOST` env; unit tests in the same file.
- **Create** `deploy/supabase-prod-ca.crt` — public Supabase CA (copied from gitignored `.certs/prod-ca-2021.crt`).
- **Create** `Dockerfile` (repo root) — multi-stage build → slim runtime with binary + world bundle + cert.
- **Create** `.dockerignore` (repo root) — keep the build context lean.
- **Create** `fly.toml` (repo root) — single-Machine service, `force_https`, `/health` check, EU region.
- **Create** `deploy/README.md` — the operational deploy runbook (fly + vercel + fresh-seed + CORS).

No existing file is restructured; the one code touch is additive + isolated.

---

### Task 1: Bindable `LISTEN_HOST`

**Files:**
- Modify: `backend/crates/sim-server/src/main.rs`
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the bottom of `backend/crates/sim-server/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::resolve_listen_addr;

    #[test]
    fn defaults_to_loopback() {
        let addr = resolve_listen_addr("127.0.0.1", 8080).expect("valid loopback addr");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8080);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn binds_all_interfaces_when_overridden() {
        let addr = resolve_listen_addr("0.0.0.0", 8080).expect("valid wildcard addr");
        assert!(addr.ip().is_unspecified(), "0.0.0.0 must be the unspecified (all-interfaces) addr");
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn rejects_non_ip_host() {
        // SocketAddr parsing is numeric-only — a hostname like "localhost" must error,
        // not silently bind nowhere.
        assert!(resolve_listen_addr("not-an-ip", 8080).is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server --bin sim-server resolve`
Expected: FAIL to compile — `cannot find function resolve_listen_addr`.

- [ ] **Step 3: Add the helper + wire it into `main`**

In `backend/crates/sim-server/src/main.rs`, add the helper above `main` (after the `use` lines):

```rust
/// Resolve the TCP listen address from host + port. Host must be a numeric IP
/// (`SocketAddr` does not resolve hostnames): `127.0.0.1` for dev (loopback only),
/// `0.0.0.0` in a container (all interfaces). Returns an error for a non-IP host
/// rather than silently failing.
fn resolve_listen_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    format!("{host}:{port}")
        .parse()
        .with_context(|| format!("parse listen address {host}:{port}"))
}
```

Then replace the port/addr block in `main` (current lines 14–20) with:

```rust
    let port: u16 = match std::env::var("LISTEN_PORT") {
        Err(_) => 8080,
        Ok(v) => v.parse().context("LISTEN_PORT must be a valid u16")?,
    };
    let host = std::env::var("LISTEN_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr = resolve_listen_addr(&host, port)?;
```

(The existing `let listener = … bind(addr) …` and `tracing::info!(%addr, "starting sim-server")` lines are unchanged.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server --bin sim-server resolve`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/main.rs
git commit -m "feat(server): bindable LISTEN_HOST (default 127.0.0.1; container sets 0.0.0.0)"
```

---

### Task 2: Commit the public Supabase CA cert

**Files:**
- Create: `deploy/supabase-prod-ca.crt`

The Supabase pooler CA is a **public** root cert; it currently lives only in the gitignored `.certs/`. Copy it into a tracked path so the image build and CI have it.

- [ ] **Step 1: Copy the cert**

```bash
mkdir -p deploy
cp .certs/prod-ca-2021.crt deploy/supabase-prod-ca.crt
```

- [ ] **Step 2: Verify it is a PEM CA cert (not a private key)**

Run: `head -1 deploy/supabase-prod-ca.crt`
Expected: `-----BEGIN CERTIFICATE-----` (NOT `PRIVATE KEY` — if it shows a key, stop; wrong file).

- [ ] **Step 3: Confirm it is not blocked by .gitignore**

Run: `git check-ignore deploy/supabase-prod-ca.crt; echo "ignored_exit=$?"`
Expected: `ignored_exit=1` (not ignored — `.gitignore` only excludes `.certs/`).

- [ ] **Step 4: Commit**

```bash
git add deploy/supabase-prod-ca.crt
git commit -m "deploy: vendor public Supabase CA cert for verify-full in the image"
```

---

### Task 3: `.dockerignore`

**Files:**
- Create: `.dockerignore` (repo root)

- [ ] **Step 1: Create the file**

```
# Keep the Docker build context lean. The image needs: backend/ (sans target),
# data/worlds/abutopia, deploy/supabase-prod-ca.crt. Everything else is excluded.
.git
.worktrees
backend/target
node_modules
dist
.playwright-mcp
**/*.png
.certs
public/simutrans-assets
docs
```

- [ ] **Step 2: Commit**

```bash
git add .dockerignore
git commit -m "deploy: add .dockerignore for a lean backend image context"
```

---

### Task 4: `Dockerfile`

**Files:**
- Create: `Dockerfile` (repo root)

- [ ] **Step 1: Create the multi-stage Dockerfile**

```dockerfile
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
```

- [ ] **Step 2: Build the image locally (smoke)**

First ensure no other cargo is running (the image build runs cargo INSIDE the container with its own target, so it does not touch the host `backend/target` lock — but check anyway):
Run: `pgrep -fl cargo | grep -v grep || echo clean`
Then: `docker build -t abutown-sim:smoke .`
Expected: build succeeds; final line `naming to docker.io/library/abutown-sim:smoke`.

- [ ] **Step 3: Verify the image contents**

Run: `docker run --rm --entrypoint sh abutown-sim:smoke -c "ls -l /app/sim-server /app/certs/supabase-ca.crt && ls /app/data/worlds/abutopia/layers"`
Expected: the binary, the cert, and `markets.json` (among other layers) are present.

- [ ] **Step 4: Commit**

```bash
git add Dockerfile
git commit -m "deploy: multi-stage Dockerfile (release sim-server + world bundle + CA cert)"
```

---

### Task 5: `fly.toml`

**Files:**
- Create: `fly.toml` (repo root)

- [ ] **Step 1: Create the Fly config**

```toml
# Single-instance, stateful sim-server. The in-memory world is the SOLE writer —
# NEVER scale count > 1 (a second machine would double-write the same world_id).
app = "abutown-abutopia"
primary_region = "lhr"  # EU, near Supabase eu-west-1 (cuts persist write latency)

[build]
  dockerfile = "Dockerfile"

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = false
  auto_start_machines = false
  min_machines_running = 1

  [[http_service.checks]]
    method = "get"
    path = "/health"
    interval = "15s"
    timeout = "5s"
    grace_period = "30s"

[[vm]]
  size = "shared-cpu-1x"
  memory = "1gb"
```

- [ ] **Step 2: Validate the config (no deploy)**

Run: `fly config validate` (requires `fly` CLI; if not installed, skip and note — validated at deploy time).
Expected: `Configuration is valid` (or skip with a note if `fly` is absent locally).

- [ ] **Step 3: Commit**

```bash
git add fly.toml
git commit -m "deploy: fly.toml — single-instance sim-server, force_https, /health, EU region"
```

---

### Task 6: Deploy runbook

**Files:**
- Create: `deploy/README.md`

- [ ] **Step 1: Write the runbook**

```markdown
# Abutopia public deploy runbook

One shared abutopia `sim-server` on Fly.io (single instance) + static frontend on
Vercel + Supabase `:5432`. Design: `docs/superpowers/specs/2026-06-08-abutopia-remote-deploy-design.md`.

> **Single writer.** Never run more than one Fly machine — the world lives in memory.

## 0. One-time: fresh-seed abutopia on the remote DB (authorized)
Clears any old `home_market=0` records so the #86 rebind regenerates bindings:
```bash
psql "$DATABASE_URL" \
  -c "DELETE FROM mobility_snapshots WHERE world_id='abutopia';" \
  -c "DELETE FROM economy_snapshots  WHERE world_id='abutopia';"
```

## 1. Backend (Fly.io)
```bash
fly auth login                      # interactive — run via `! fly auth login`
fly launch --no-deploy --copy-config --name abutown-abutopia --region lhr
fly secrets set \
  DATABASE_URL='postgresql://…@…pooler.supabase.com:5432/postgres?sslmode=verify-full' \
  CORS_ALLOWED_ORIGINS='https://PLACEHOLDER-set-after-vercel' \
  ABUTOWN_DB_MAX_CONNECTIONS='8'
fly deploy
# Verify:
curl -s https://abutown-abutopia.fly.dev/health -o /dev/null -w '%{http_code}\n'   # 200
```
`PGSSLROOTCERT`, `LISTEN_HOST`, `LISTEN_PORT`, `RUST_LOG` come from the image `ENV`.
Confirm the largest safe `ABUTOWN_DB_MAX_CONNECTIONS` against the Supabase dashboard
pooler ceiling (8 is conservative-safe).

## 2. Frontend (Vercel)
```bash
vercel login                        # interactive — run via `! vercel login`
VITE_ABUTOWN_BACKEND_URL='https://abutown-abutopia.fly.dev' vercel --prod
# Note the production URL it prints, e.g. https://abutown.vercel.app
```
(Set `VITE_ABUTOWN_BACKEND_URL` as a Vercel project env var for repeat builds.)

## 3. Wire CORS to the real frontend origin
```bash
fly secrets set CORS_ALLOWED_ORIGINS='https://abutown.vercel.app'   # restarts the machine
```

## 4. Verify (acceptance)
- `GET https://abutown-abutopia.fly.dev/health` → `ok=true`, persistence not `stale`, `world_id=abutopia`.
- Open the Vercel URL in a clean browser → abutopia renders over `wss://`, no
  "persistence stale" overlay, no CORS/mixed-content console errors.
- `fly logs` shows `economy::liveness … routed > 0`.
- Open the URL in a second browser → both see the live, ticking world.

## Rollback
`fly releases` + `fly deploy --image <previous>`; or `fly apps restart abutown-abutopia`.
```

- [ ] **Step 2: Commit**

```bash
git add deploy/README.md
git commit -m "deploy: add Fly + Vercel + Supabase deploy runbook"
```

---

### Task 7: Full gate

No new behavior in the running dev stack (the `LISTEN_HOST` change defaults to `127.0.0.1`), but run the complete gate so the deploy branch is mergeable.

- [ ] **Step 1: Rust gate** (route through `scripts/cargo-serial.sh`, background slow ones)
```bash
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
```
Expected: fmt clean; clippy 0 warnings; sim-server tests pass incl. the 3 `resolve_listen_addr` tests.

- [ ] **Step 2: Frontend gate**
```bash
npm run typecheck && npm test && npm run build
```
Expected: typecheck clean, vitest all pass, build ok (no frontend change — confirms no regression).

- [ ] **Step 3: e2e render-smoke** (frees nothing on the wire; confirms no regression)
```bash
CORS_ALLOWED_ORIGINS="http://127.0.0.1:5173" npm run test:e2e
```
Expected: render-smoke 2/2 pass. (The e2e_server still binds via its own path; the dev stack omits `LISTEN_HOST` → stays `127.0.0.1`.)

- [ ] **Step 4: Docker build smoke** (already run in Task 4 Step 2; re-confirm)
```bash
docker build -t abutown-sim:gate . && echo "docker build OK"
```
Expected: `docker build OK`.

---

### Task 8: PR (finishing-a-development-branch)

- [ ] **Step 1:** Use **superpowers:finishing-a-development-branch** → Push + create PR against `main`.
- [ ] **Step 2:** PR body: summary (public single-instance deploy artifacts + the one `LISTEN_HOST` code change), the grounding (why `:5432` not `:6543`), test plan (gate green + docker build), and that the live `fly`/`vercel` deploy is the post-merge operational step.
- [ ] **Step 3:** Wait for ALL CI checks green (never merge on pending/UNSTABLE), squash-merge, clean up the worktree + branch.

**Post-merge (operational, controller + user — NOT a code task):** execute `deploy/README.md` (interactive `fly`/`vercel` logins via `!`), then verify the live acceptance criteria.

---

## Self-Review

**Spec coverage:** §3.1 LISTEN_HOST → Task 1; §3.4 CA cert → Task 2; §3.3 .dockerignore → Task 3; §3.2 Dockerfile → Task 4; §3.5 fly.toml → Task 5; §4 runbook + §3.6 secrets + §3.7 frontend → Task 6; §6 testing → Task 7; PR → Task 8. All spec sections covered. (§7 out-of-scope items intentionally have no task; the `db.rs`/spec `:6543`-comment doc-fix is deferred per §7.)

**Placeholder scan:** `<app>`/`PLACEHOLDER-set-after-vercel`/`abutown.vercel.app` are deploy-time runtime values (the Fly app name + Vercel URL), explicitly resolved in the runbook — not design gaps. No "TBD/implement later" in any step.

**Type consistency:** `resolve_listen_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr>` used identically in Task 1 Steps 1, 3, 4. `LISTEN_HOST`/`LISTEN_PORT`/`PGSSLROOTCERT` consistent across Tasks 1, 4, 6. Cert path `/app/certs/supabase-ca.crt` consistent in Tasks 4 + 6. Binary name `sim-server` consistent.
