# Mobility Persistence Liveness Design

Date: 2026-05-30

## Status

Approved direction after reviewing the current persistence specs and the live
Supabase state. This is a small hardening slice before the larger
`chunk_mobility_snapshots` work described in the million-agent roadmap.

## Context

The current backend persists mobility through `mobility_snapshots`, one JSONB row
per `world_id`, updated from the snapshot loop every 5 seconds. This is
spec-conformant for the current Abutopia slice and intentionally remains the
storage shape until the later per-chunk persistence phase.

The operational gap is liveness: a stale row can look like "agents are
persisted" even when the backend process is stopped or the snapshot loop can no
longer write. Current code logs failed mobility writes, but the health surface
does not expose whether persistence is fresh.

## Goal

Make mobility persistence freshness observable and testable, so local and
credentialed CI/operator smokes fail loudly when the backend is running but
Supabase is not receiving fresh `mobility_snapshots` writes.

## Non-Goals

- No `chunk_mobility_snapshots` table in this slice.
- No normalized `agent_current_state` table.
- No direct browser writes to canonical world tables.
- No per-tick database writes.
- No auth or RLS redesign.

## Design

### Runtime Persistence Status

Add a small runtime status object owned by the server app layer, not by the hot
simulation loop. It records:

- last persistence attempt time,
- last successful mobility persistence time,
- last successfully persisted `world_id`,
- last successfully persisted mobility tick,
- consecutive mobility persistence failures,
- last error string, redacted and bounded in length.

The snapshot loop updates this status after every mobility write attempt. Chunk
snapshot errors already propagate through `persist_snapshots_once`; mobility
write failures must also become visible in the status instead of being only a
log line.

### Health Surface

Extend the backend health response with a persistence section. The health route
must not query Postgres on every request; it reads the in-memory status updated
by the snapshot loop.

Freshness policy:

- snapshot interval is 5 seconds,
- persistence is fresh if the last successful mobility write is no older than
  15 seconds,
- before the first snapshot-loop attempt, health reports persistence as
  `starting`, not failed,
- after at least one failed attempt, health reports `degraded`,
- after freshness exceeds 15 seconds, health reports `stale`.

The top-level backend health should remain `ok=true` during `starting`, but
must become `ok=false` for `degraded` or `stale`. This keeps frontend startup
fail-closed when the backend cannot persist.

### Frontend Error Surface

The Vite client already treats the backend as required. This slice keeps that
contract and extends the error reason:

- if `/health` is unreachable, the canvas runtime must not boot,
- if `/health.ok` is `false` because persistence is `degraded` or `stale`, the
  canvas runtime must not boot,
- the visible error must say that the backend is required and include the
  persistence status when the backend supplied one,
- `#game[data-ready="true"]` must not be set while the backend is missing or
  persistence health is failed.

The browser must not query Supabase directly for persistence freshness. It only
trusts the backend health surface, because the Rust service is the simulation
authority and the owner of canonical world-table writes.

### Local Supabase Freshness Smoke

Add a developer smoke command that checks the full boundary:

1. `GET /health` succeeds and says `world_id = abutopia`.
2. `GET /mobility` succeeds and returns current runtime mobility tick and agent
   count.
3. Supabase `mobility_snapshots` contains exactly the `abutopia` row.
4. The DB row's `tick` is not behind the runtime mobility tick by more than
   three snapshot intervals.
5. The DB row's `updated_at` is no older than 15 seconds.
6. The DB row's payload has the same agent count as the runtime snapshot.

The smoke reads `DATABASE_URL` from the local ignored `.env` file and must redact
connection strings and secret values in all output.

### Tests

Unit tests cover the freshness classifier:

- no attempts yet -> `starting`,
- recent success -> `fresh`,
- recent failure -> `degraded`,
- old success -> `stale`,
- long error strings are bounded.

Backend tests cover health integration with an injected status:

- healthy persistence keeps `/health` OK,
- degraded persistence sets `/health.ok = false`,
- stale persistence sets `/health.ok = false`,
- the persistence fields encode without breaking existing health consumers.

Frontend tests cover startup failure:

- transport failure to `/health` renders the backend-required message and leaves
  `#game[data-ready="true"]` unset,
- `/health.ok = false` with persistence `degraded` renders a persistence-specific
  backend-required message,
- `/health.ok = false` with persistence `stale` renders a persistence-specific
  backend-required message.

The local Supabase smoke is not a normal unit test because it depends on real
credentials and a running backend. It is documented as an operator/developer
verification command and used before claiming the local environment is live.

## Acceptance Criteria

- Running backend health exposes mobility persistence status.
- A stopped backend fails the freshness smoke because `/health` is unreachable.
- A stopped backend makes the frontend render a backend-required error instead
  of the canvas runtime.
- A running backend with a failing mobility write reports unhealthy health.
- A running backend with unhealthy persistence makes the frontend render a
  backend-required persistence error instead of the canvas runtime.
- A running backend with stale `mobility_snapshots.updated_at` fails the smoke.
- A healthy backend updates the `abutopia` row within 15 seconds and passes the
  smoke.
- No real Supabase secrets are committed or printed.
- Existing `mobility_snapshots` schema stays unchanged.

## Follow-Up

After this guard is green, the next persistence feature is the roadmap's
per-chunk mobility snapshot phase:

- add `chunk_mobility_snapshots`,
- write only dirty mobility chunks,
- hydrate active chunks lazily,
- deprecate `mobility_snapshots` only after the chunked path is verified.
