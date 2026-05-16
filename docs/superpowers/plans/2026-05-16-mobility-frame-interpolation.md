# Mobility Frame Interpolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise backend tick from 1 Hz to 10 Hz and add frontend linear interpolation between the last two server states per mobility entity so the canvas renders smooth 60 fps motion.

**Architecture:** Backend `SIMULATION_TICK_INTERVAL` becomes 100 ms; `WorldSummaryDto` exposes `tick_period_ms: 100`. Frontend mobility state buffers `{prev, current, lastTickAt}` per entity; the drawables projector linearly interpolates `world_coord` between `prev` and `current` using `now()` per render frame. Direction stays discrete.

**Tech Stack:** Rust 2024 (backend), TypeScript + Vite + Vitest (frontend), Playwright (E2E). Existing protocol, no wire-shape change beyond the new `tick_period_ms` field.

**Spec:** `docs/superpowers/specs/2026-05-16-mobility-frame-interpolation-design.md`
**Roadmap:** `docs/superpowers/specs/2026-05-16-million-agent-roadmap-design.md` (Phase 2 of 8)

---

## File Structure

Backend:
- Modify `backend/crates/protocol/src/lib.rs` — add `tick_period_ms: u32` to `WorldSummaryDto`; update protocol unit tests.
- Modify `backend/crates/sim-server/src/app.rs` — change `SIMULATION_TICK_INTERVAL` from `Duration::from_secs(1)` to `Duration::from_millis(100)`.
- Modify `backend/crates/sim-server/src/runtime.rs` — populate `tick_period_ms: 100` in `world_summary()`; expose `world_summary_tick_period_ms()` constant or use a single `const TICK_PERIOD_MS: u32 = 100` shared between `app.rs` and `runtime.rs`.
- Modify `backend/crates/sim-server/tests/http.rs` — assert `tick_period_ms` field on `/world` response.
- Modify `backend/crates/sim-server/tests/websocket.rs` — replace 500 ms-empty cadence assertions with 10 Hz-cadence assertions (multiple ticks within 500 ms).

Frontend:
- Modify `src/backend/mobilityProtocol.ts` — add `tick_period_ms` to `WorldSummaryDto` type (if mirrored on the frontend; otherwise this is a new type).
- Modify `src/backend/mobilityState.ts` — `InterpolatedEntry<T>` type, change map shapes, update `applyMobilitySnapshot` / `applyMobilityDelta`, add `interpolatedAgents` / `interpolatedVehicles` helpers.
- Modify `src/backend/roadVehicleState.ts` — same shape change for road vehicles, add `interpolatedRoadVehicles` helper.
- Modify `src/backend/mobilityClient.ts` — `requireMobilitySnapshot` returns `{ state, tickPeriodMs }`; fetch `/world` if not already done at boot to source the tick period.
- Modify `src/render/backendMobilityDrawables.ts` — accept `now: number` + `tickPeriodMs: number`, project from interpolated state.
- Modify `src/main.ts` — read tick period at boot, pass `now` + `tickPeriodMs` to projector every frame.
- Modify `tests/backend/mobilityState.test.ts` — update for buffered shape.
- Modify `tests/backend/roadVehicleState.test.ts` — same.
- Modify `tests/backend/mobilityClient.test.ts` — assert tick period is surfaced and `/world` is fetched.
- Modify `tests/render/backendMobilityDrawables.test.ts` — interpolation assertions at `t = 0`, `t = 0.5`, `t = 1`, `t > 1` clamp.
- Modify `tests/e2e/render-smoke.spec.ts` — assert mobility-entity screen positions differ between two reads taken 50 ms apart (i.e. they animate).

Docs:
- Modify `progress.md` — record this phase's completion.

---

## Task 1: Add `tick_period_ms` To `WorldSummaryDto`

**Files:**
- Modify: `backend/crates/protocol/src/lib.rs`

- [ ] **Step 1: Write the failing protocol test**

In `backend/crates/protocol/src/lib.rs`, append inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn world_summary_dto_serializes_tick_period_ms() {
    let dto = WorldSummaryDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: WorldId("abutown-main".to_string()),
        chunk_size: 32,
        loaded_chunks: vec![],
        tick_period_ms: 100,
    };
    let json = serde_json::to_value(&dto).unwrap();
    assert_eq!(json["tick_period_ms"], 100);
    let back: WorldSummaryDto = serde_json::from_value(json).unwrap();
    assert_eq!(back, dto);
}
```

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol world_summary_dto_serializes_tick_period_ms
```

Expected: FAIL — field does not exist.

- [ ] **Step 3: Add the field**

In `backend/crates/protocol/src/lib.rs`, update `WorldSummaryDto`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSummaryDto {
    pub protocol_version: u16,
    pub world_id: WorldId,
    pub chunk_size: u16,
    pub loaded_chunks: Vec<ChunkCoordDto>,
    pub tick_period_ms: u32,
}
```

- [ ] **Step 4: Verify protocol crate**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p abutown-protocol
```

Expected: green; new test passes.

The workspace will NOT build yet because `runtime.rs` doesn't populate the new field. That's fixed in Task 2.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/protocol/src/lib.rs
git commit -m "feat: expose tick_period_ms in world summary DTO"
```

---

## Task 2: Backend Ticks At 10 Hz

**Files:**
- Modify: `backend/crates/sim-server/src/app.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [ ] **Step 1: Change the tick interval constant**

In `backend/crates/sim-server/src/app.rs`, find:

```rust
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_secs(1);
```

Replace with:

```rust
const SIMULATION_TICK_INTERVAL: Duration = Duration::from_millis(100);
```

Leave `SNAPSHOT_INTERVAL` unchanged at 5 s.

- [ ] **Step 2: Expose tick period via runtime**

In `backend/crates/sim-server/src/runtime.rs`, at the top of the impl block (alongside other constants), add:

```rust
pub const TICK_PERIOD_MS: u32 = 100;
```

Update `world_summary()`:

```rust
pub fn world_summary(&self) -> WorldSummaryDto {
    WorldSummaryDto {
        protocol_version: PROTOCOL_VERSION,
        world_id: self.world_id.clone(),
        chunk_size: self.registry.chunk_size(),
        loaded_chunks: self
            .registry
            .loaded_coords()
            .into_iter()
            .map(ChunkCoordDto::from)
            .collect(),
        tick_period_ms: TICK_PERIOD_MS,
    }
}
```

(Place `TICK_PERIOD_MS` as a `pub const` inside `impl SimulationRuntime` or as a module-level `const` next to the existing `CHUNK_SIZE` / `WORLD_ID` constants — whichever the file already does.)

- [ ] **Step 3: Verify backend builds**

```bash
cargo build --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: compiles.

- [ ] **Step 4: Verify unit + integration tests still pass (timing-sensitive tests will fail)**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: most pass; `websocket_sends_hello_and_tile_pulse` may fail because of the 500 ms-empty assertion. Move to Task 3 for the test updates.

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/app.rs backend/crates/sim-server/src/runtime.rs
git commit -m "feat: tick simulation at 10 Hz"
```

---

## Task 3: Update Backend Timing-Dependent Tests

**Files:**
- Modify: `backend/crates/sim-server/tests/http.rs`
- Modify: `backend/crates/sim-server/tests/websocket.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs` (its embedded test module)

- [ ] **Step 1: Assert tick_period_ms on the /world endpoint**

In `backend/crates/sim-server/tests/http.rs`, locate the `health_and_world_summary_are_available` test. Append after the existing `loaded_chunks` assertions:

```rust
    assert_eq!(json["tick_period_ms"], 100);
```

- [ ] **Step 2: Update runtime.rs unit test**

In `backend/crates/sim-server/src/runtime.rs`, the embedded test `runtime_summarizes_multiple_loaded_chunks` builds a `WorldSummaryDto` literal in its assertion. Find the test (search `runtime_summarizes_multiple_loaded_chunks`) and update any explicit `WorldSummaryDto { … }` construction to include `tick_period_ms: 100`. If the test only inspects field-by-field (not struct equality), add an assertion:

```rust
    assert_eq!(summary.tick_period_ms, 100);
```

If the test uses struct equality on the whole DTO, add the field to the expected literal.

- [ ] **Step 3: Rewrite the websocket cadence assertions**

In `backend/crates/sim-server/tests/websocket.rs`, locate `websocket_sends_hello_and_tile_pulse`. The current test has two `tokio::time::timeout(Duration::from_millis(500), stream.next()).is_err()` assertions that check "no message arrives within 500 ms." At 10 Hz, that's now false — 5 ticks worth of messages arrive within 500 ms.

Replace BOTH 500 ms-empty assertions with the corresponding "next tick arrives within 200 ms" positive check. Concretely, replace this block:

```rust
    assert!(
        tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await
            .is_err(),
        "tile pulse should not arrive immediately after hello"
    );
```

with:

```rust
    // 10 Hz tick: the first tile pulse arrives within one tick period plus jitter.
    // We use 250 ms to absorb scheduler jitter on slow CI without weakening intent.
    let first_arrival = tokio::time::timeout(Duration::from_millis(250), stream.next())
        .await
        .expect("tile pulse must arrive within one tick window");
    assert!(first_arrival.is_some(), "stream did not yield tile pulse");
```

Then read the first three messages as the test currently does (tile pulse, mobility delta, road vehicle delta) — that part stays. After consuming those three messages, replace the second `is_err()` block with:

```rust
    // Next tick must arrive within roughly one tick period.
    let _next_pulse = tokio::time::timeout(Duration::from_millis(250), stream.next())
        .await
        .expect("next tick must arrive within one tick window");
```

If the existing test then expects `let next_delta = read_next_tile_pulse(&mut stream).await;` after the cadence check, keep that call but adjust the surrounding logic so the message just consumed by the timeout block above is not double-consumed. The cleanest pattern: drop the timeout-based wait, and just call `read_next_tile_pulse(&mut stream)` with a `Duration::from_millis(500)` outer timeout via:

```rust
    let next_delta = tokio::time::timeout(Duration::from_millis(500), read_next_tile_pulse(&mut stream))
        .await
        .expect("next tile pulse arrives within 500 ms")
        .expect("stream not closed");
    assert_eq!(next_delta.tick, 2);
```

(Adapt to the actual `read_next_tile_pulse` signature — if it doesn't return an `Option`, drop the second `.expect`.)

Also: scan the rest of `websocket.rs` for other `Duration::from_millis(150)` / `Duration::from_secs(2)` / `Duration::from_millis(500)` assertions and verify they still make sense at 10 Hz. The only "no message" cadence check I see is in the hello-and-tile-pulse test; the rest are positive waits that scale fine.

- [ ] **Step 4: Run backend tests**

```bash
cargo test --locked --manifest-path backend/Cargo.toml -p sim-server
```

Expected: all pass. If any test still fails on timing, it likely also expected the 1 Hz cadence — fix the same way (shorter timeout window).

- [ ] **Step 5: Workspace tests + clippy**

```bash
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: green.

- [ ] **Step 6: Commit**

```bash
git add backend/crates/sim-server/tests/http.rs backend/crates/sim-server/tests/websocket.rs backend/crates/sim-server/src/runtime.rs
git commit -m "test: adapt timing-sensitive backend tests to 10 Hz cadence"
```

---

## Task 4: Mirror `tick_period_ms` On The Frontend Protocol

**Files:**
- Modify: `src/backend/mobilityProtocol.ts` (or wherever `WorldSummaryDto` is typed; verify)

- [ ] **Step 1: Locate the frontend type**

Run:

```bash
grep -rn "WorldSummaryDto\|world_summary\|tick_period_ms\|loaded_chunks" src/
```

If a TypeScript `WorldSummaryDto` type already exists (likely in `src/backend/mobilityProtocol.ts` or a `backendGate.ts`-style module), add the field. If no type exists yet, declare a minimal one in `src/backend/mobilityClient.ts`:

```ts
export type WorldSummaryDto = {
  protocol_version: number;
  world_id: string;
  chunk_size: number;
  loaded_chunks: Array<{ x: number; y: number }>;
  tick_period_ms: number;
};

export function isWorldSummaryDto(value: unknown): value is WorldSummaryDto {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.protocol_version === 'number' &&
    typeof v.world_id === 'string' &&
    typeof v.chunk_size === 'number' &&
    Array.isArray(v.loaded_chunks) &&
    typeof v.tick_period_ms === 'number' &&
    v.tick_period_ms > 0
  );
}
```

- [ ] **Step 2: Verify frontend compile**

```bash
npx tsc --noEmit
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/backend/
git commit -m "feat: type world summary tick_period_ms on frontend"
```

---

## Task 5: `InterpolatedEntry` And Buffered Mobility State

**Files:**
- Modify: `src/backend/mobilityState.ts`
- Modify: `tests/backend/mobilityState.test.ts`

- [ ] **Step 1: Write failing tests**

Append to `tests/backend/mobilityState.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import {
  applyMobilityDelta,
  applyMobilitySnapshot,
  createMobilityOverlayState,
  interpolatedAgents,
} from '../../src/backend/mobilityState';
import type { AgentMobilityDto, MobilityDeltaDto, MobilitySnapshotDto } from '../../src/backend/mobilityProtocol';

function agentAt(id: string, x: number, y: number): AgentMobilityDto {
  return {
    id,
    state: { type: 'walking', link_id: 'link:walk:default', progress: 0.0 },
    plan_cursor: 0,
    world_coord: { x, y },
    direction: 'e',
    sprite_key: 'pedestrian:0',
  };
}

describe('mobility state interpolation buffer', () => {
  it('initial snapshot sets prev == current for each agent', () => {
    const snapshot: MobilitySnapshotDto = {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 1,
      agents: [agentAt('agent:seed:0', 100, 200)],
      vehicles: [],
      stops: [],
    };
    const state = applyMobilitySnapshot(createMobilityOverlayState(), snapshot, 1000);
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.lastTickAt).toBe(1000);
  });

  it('delta moves prev←current and sets current=new dto', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [agentAt('agent:seed:0', 100, 200)],
        vehicles: [],
        stops: [],
      },
      1000,
    );
    const delta: MobilityDeltaDto = {
      protocol_version: 1,
      world_id: 'abutown-main',
      tick: 2,
      changed_agents: [agentAt('agent:seed:0', 110, 200)],
      changed_vehicles: [],
    };
    state = applyMobilityDelta(state, delta, 1100);
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 110, y: 200 });
    expect(entry.lastTickAt).toBe(1100);
  });

  it('delta for a new agent sets prev == current', () => {
    let state = createMobilityOverlayState();
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        changed_agents: [agentAt('agent:seed:0', 50, 60)],
        changed_vehicles: [],
      },
      500,
    );
    const entry = state.agents.get('agent:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 50, y: 60 });
    expect(entry.current.world_coord).toEqual({ x: 50, y: 60 });
  });

  it('interpolatedAgents lerps world_coord by t = (now - lastTickAt) / tickPeriodMs', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [agentAt('agent:seed:0', 100, 200)],
        vehicles: [],
        stops: [],
      },
      1000,
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [agentAt('agent:seed:0', 110, 200)],
        changed_vehicles: [],
      },
      1100,
    );
    const agents = interpolatedAgents(state, 1150, 100);
    expect(agents).toHaveLength(1);
    expect(agents[0].world_coord.x).toBeCloseTo(105.0, 5);
    expect(agents[0].world_coord.y).toBeCloseTo(200.0, 5);
  });

  it('interpolatedAgents clamps t to [0, 1]', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [agentAt('agent:seed:0', 0, 0)],
        vehicles: [],
        stops: [],
      },
      0,
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [agentAt('agent:seed:0', 100, 0)],
        changed_vehicles: [],
      },
      1000,
    );
    const earlyAgents = interpolatedAgents(state, 500, 100); // before lastTickAt
    expect(earlyAgents[0].world_coord.x).toBeCloseTo(0, 5); // t clamps to 0? actually 500 < 1000 so negative; clamp to 0 → prev
    const lateAgents = interpolatedAgents(state, 5000, 100); // long after lastTickAt
    expect(lateAgents[0].world_coord.x).toBeCloseTo(100, 5); // t clamps to 1 → current
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/backend/mobilityState.test.ts
```

Expected: FAIL — `InterpolatedEntry`-shaped map and `interpolatedAgents` don't exist yet.

- [ ] **Step 3: Update mobilityState.ts**

Replace `src/backend/mobilityState.ts` with this shape (preserving the existing dispatch in `applyServerMessage`):

```ts
import {
  isMobilityDeltaDto,
  parseServerMessage,
  type AgentMobilityDto,
  type MobilityDeltaDto,
  type MobilitySnapshotDto,
  type StopMobilityDto,
  type VehicleMobilityDto,
} from './mobilityProtocol';
import {
  applyRoadVehicleSnapshot,
  applyRoadVehicleDelta,
  createRoadVehicleOverlayState,
  type RoadVehicleOverlayState,
} from './roadVehicleState';
import type { RoadVehicleSnapshotDto } from './roadVehicleProtocol';

export type MobilityConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export type InterpolatedEntry<T> = {
  prev: T;
  current: T;
  lastTickAt: number;
};

export type MobilityDiagnostics = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: number;
  vehicles: number;
  stops: number;
  roadVehicles: number;
  invalidMessages: number;
  lastError: string | null;
};

export type MobilityOverlayState = {
  status: MobilityConnectionStatus;
  tick: number;
  agents: Map<string, InterpolatedEntry<AgentMobilityDto>>;
  vehicles: Map<string, InterpolatedEntry<VehicleMobilityDto>>;
  stops: Map<string, StopMobilityDto>;
  roadVehicles: RoadVehicleOverlayState;
  invalidMessages: number;
  lastError: string | null;
  lastUpdatedAt: number;
};

export function createMobilityOverlayState(): MobilityOverlayState {
  return {
    status: 'disconnected',
    tick: 0,
    agents: new Map(),
    vehicles: new Map(),
    stops: new Map(),
    roadVehicles: createRoadVehicleOverlayState(),
    invalidMessages: 0,
    lastError: null,
    lastUpdatedAt: 0,
  };
}

export function markMobilityConnecting(state: MobilityOverlayState, now = Date.now()): MobilityOverlayState {
  return { ...state, status: 'connecting', lastError: null, lastUpdatedAt: now };
}

export function markMobilityDisconnected(
  state: MobilityOverlayState,
  error: string | null,
  now = Date.now(),
): MobilityOverlayState {
  return { ...state, status: 'disconnected', lastError: error, lastUpdatedAt: now };
}

function initEntry<T>(dto: T, lastTickAt: number): InterpolatedEntry<T> {
  return { prev: dto, current: dto, lastTickAt };
}

export function applyMobilitySnapshot(
  state: MobilityOverlayState,
  snapshot: MobilitySnapshotDto,
  now = Date.now(),
): MobilityOverlayState {
  return {
    ...state,
    status: 'connected',
    tick: snapshot.tick,
    agents: new Map(snapshot.agents.map((agent) => [agent.id, initEntry(agent, now)])),
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, initEntry(vehicle, now)])),
    stops: new Map(snapshot.stops.map((stop) => [stop.id, stop])),
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyMobilityDelta(
  state: MobilityOverlayState,
  delta: MobilityDeltaDto,
  now = Date.now(),
): MobilityOverlayState {
  const agents = new Map(state.agents);
  for (const agent of delta.changed_agents) {
    const previous = agents.get(agent.id);
    agents.set(agent.id, {
      prev: previous?.current ?? agent,
      current: agent,
      lastTickAt: now,
    });
  }
  const vehicles = new Map(state.vehicles);
  for (const vehicle of delta.changed_vehicles) {
    const previous = vehicles.get(vehicle.id);
    vehicles.set(vehicle.id, {
      prev: previous?.current ?? vehicle,
      current: vehicle,
      lastTickAt: now,
    });
  }
  return {
    ...state,
    status: 'connected',
    tick: delta.tick,
    agents,
    vehicles,
    lastError: null,
    lastUpdatedAt: now,
  };
}

export function applyRoadVehicleSnapshotToState(
  state: MobilityOverlayState,
  snapshot: RoadVehicleSnapshotDto,
  now = Date.now(),
): MobilityOverlayState {
  return { ...state, roadVehicles: applyRoadVehicleSnapshot(state.roadVehicles, snapshot, now), lastUpdatedAt: now };
}

export function applyServerMessage(
  state: MobilityOverlayState,
  value: unknown,
  now = Date.now(),
): MobilityOverlayState {
  const message = parseServerMessage(value);
  if (message?.type === 'mobility_delta' && isMobilityDeltaDto(message)) {
    return applyMobilityDelta(state, message, now);
  }
  if (message?.type === 'road_vehicle_delta') {
    return {
      ...state,
      roadVehicles: applyRoadVehicleDelta(state.roadVehicles, message, now),
      lastUpdatedAt: now,
    };
  }
  if (message !== null) return state;
  return { ...state, invalidMessages: state.invalidMessages + 1, lastUpdatedAt: now };
}

export function mobilityDiagnostics(state: MobilityOverlayState): MobilityDiagnostics {
  return {
    status: state.status,
    tick: state.tick,
    agents: state.agents.size,
    vehicles: state.vehicles.size,
    stops: state.stops.size,
    roadVehicles: state.roadVehicles.vehicles.size,
    invalidMessages: state.invalidMessages,
    lastError: state.lastError,
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function lerpCoord(
  prev: { x: number; y: number },
  current: { x: number; y: number },
  t: number,
): { x: number; y: number } {
  return {
    x: prev.x + (current.x - prev.x) * t,
    y: prev.y + (current.y - prev.y) * t,
  };
}

export function interpolatedAgents(
  state: MobilityOverlayState,
  now: number,
  tickPeriodMs: number,
): AgentMobilityDto[] {
  const out: AgentMobilityDto[] = [];
  for (const entry of state.agents.values()) {
    const t = clamp((now - entry.lastTickAt) / tickPeriodMs, 0, 1);
    out.push({
      ...entry.current,
      world_coord: lerpCoord(entry.prev.world_coord, entry.current.world_coord, t),
    });
  }
  return out;
}

export function interpolatedVehicles(
  state: MobilityOverlayState,
  now: number,
  tickPeriodMs: number,
): VehicleMobilityDto[] {
  const out: VehicleMobilityDto[] = [];
  for (const entry of state.vehicles.values()) {
    const t = clamp((now - entry.lastTickAt) / tickPeriodMs, 0, 1);
    out.push({
      ...entry.current,
      world_coord: lerpCoord(entry.prev.world_coord, entry.current.world_coord, t),
    });
  }
  return out;
}
```

`VehicleMobilityDto` is already imported (used by the existing types). `RoadVehicleOverlayState` retains its own buffer shape — implemented in Task 6.

The `roadVehicles` field continues to use `applyRoadVehicleSnapshot` / `applyRoadVehicleDelta` from `roadVehicleState.ts`, which Task 6 updates.

- [ ] **Step 4: Run tests**

```bash
npx vitest run tests/backend/mobilityState.test.ts
```

Expected: green.

- [ ] **Step 5: Update existing fixtures that read `.world_coord` from the map directly**

Run:

```bash
grep -rn "state.agents.get\|state.vehicles.get\|\.agents\.get\|\.vehicles\.get" tests/ src/
```

Anywhere outside of `mobilityState.ts` reads `.world_coord` directly off `state.agents.get(id)` will need to either use `interpolatedAgents` or read `.current.world_coord`. Existing test in `tests/backend/mobilityClient.test.ts` likely doesn't drill into coords; verify by running the full suite below.

- [ ] **Step 6: Run all frontend tests**

```bash
npx vitest run
```

Expected: green. If a test fails because it expected `entry: AgentMobilityDto` instead of `entry: InterpolatedEntry<AgentMobilityDto>`, update the test to read `.current` for the latest DTO (or `interpolatedAgents` for interpolated coords).

- [ ] **Step 7: Commit**

```bash
git add src/backend/mobilityState.ts tests/backend/mobilityState.test.ts
git commit -m "feat: buffer prev+current per agent for interpolation"
```

---

## Task 6: Buffer Road-Vehicle State Analogously

**Files:**
- Modify: `src/backend/roadVehicleState.ts`
- Modify: `tests/backend/roadVehicleState.test.ts`

- [ ] **Step 1: Write failing tests**

Replace the existing test in `tests/backend/roadVehicleState.test.ts` (or append, then drop the obsolete one) with:

```ts
import { describe, expect, it } from 'vitest';
import {
  applyRoadVehicleDelta,
  applyRoadVehicleSnapshot,
  createRoadVehicleOverlayState,
  interpolatedRoadVehicles,
} from '../../src/backend/roadVehicleState';
import type { RoadVehicleDto } from '../../src/backend/roadVehicleProtocol';

function vehicleAt(id: string, x: number, y: number): RoadVehicleDto {
  return { id, world_coord: { x, y }, direction: 'e', sprite_key: 'vehicle:0' };
}

describe('road vehicle state interpolation buffer', () => {
  it('snapshot then delta updates prev+current', () => {
    let state = applyRoadVehicleSnapshot(
      createRoadVehicleOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        vehicles: [vehicleAt('road_vehicle:seed:0', 100, 200)],
      },
      1000,
    );
    let entry = state.vehicles.get('road_vehicle:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 100, y: 200 });

    state = applyRoadVehicleDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed: [vehicleAt('road_vehicle:seed:0', 110, 200)],
      },
      1100,
    );
    entry = state.vehicles.get('road_vehicle:seed:0')!;
    expect(entry.prev.world_coord).toEqual({ x: 100, y: 200 });
    expect(entry.current.world_coord).toEqual({ x: 110, y: 200 });
    expect(entry.lastTickAt).toBe(1100);
  });

  it('interpolatedRoadVehicles lerps at t = 0.5', () => {
    let state = applyRoadVehicleSnapshot(
      createRoadVehicleOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        vehicles: [vehicleAt('road_vehicle:seed:0', 0, 0)],
      },
      1000,
    );
    state = applyRoadVehicleDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed: [vehicleAt('road_vehicle:seed:0', 100, 0)],
      },
      1100,
    );
    const vehicles = interpolatedRoadVehicles(state, 1150, 100);
    expect(vehicles[0].world_coord.x).toBeCloseTo(50.0, 5);
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/backend/roadVehicleState.test.ts
```

Expected: FAIL — `InterpolatedEntry`-shaped map and `interpolatedRoadVehicles` don't exist.

- [ ] **Step 3: Update roadVehicleState.ts**

Replace `src/backend/roadVehicleState.ts`:

```ts
import {
  isRoadVehicleDeltaDto,
  isRoadVehicleSnapshotDto,
  type RoadVehicleDeltaDto,
  type RoadVehicleDto,
  type RoadVehicleSnapshotDto,
} from './roadVehicleProtocol';

export type InterpolatedRoadVehicleEntry = {
  prev: RoadVehicleDto;
  current: RoadVehicleDto;
  lastTickAt: number;
};

export type RoadVehicleOverlayState = {
  tick: number;
  vehicles: Map<string, InterpolatedRoadVehicleEntry>;
  invalidMessages: number;
  lastUpdatedAt: number;
};

export function createRoadVehicleOverlayState(): RoadVehicleOverlayState {
  return { tick: 0, vehicles: new Map(), invalidMessages: 0, lastUpdatedAt: 0 };
}

function initEntry(dto: RoadVehicleDto, lastTickAt: number): InterpolatedRoadVehicleEntry {
  return { prev: dto, current: dto, lastTickAt };
}

export function applyRoadVehicleSnapshot(
  state: RoadVehicleOverlayState,
  snapshot: RoadVehicleSnapshotDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  return {
    ...state,
    tick: snapshot.tick,
    vehicles: new Map(snapshot.vehicles.map((vehicle) => [vehicle.id, initEntry(vehicle, now)])),
    lastUpdatedAt: now,
  };
}

export function applyRoadVehicleDelta(
  state: RoadVehicleOverlayState,
  delta: RoadVehicleDeltaDto,
  now = Date.now(),
): RoadVehicleOverlayState {
  const vehicles = new Map(state.vehicles);
  for (const vehicle of delta.changed) {
    const previous = vehicles.get(vehicle.id);
    vehicles.set(vehicle.id, {
      prev: previous?.current ?? vehicle,
      current: vehicle,
      lastTickAt: now,
    });
  }
  return { ...state, tick: delta.tick, vehicles, lastUpdatedAt: now };
}

export function applyRoadVehicleMessage(
  state: RoadVehicleOverlayState,
  value: unknown,
  now = Date.now(),
): RoadVehicleOverlayState {
  if (isRoadVehicleDeltaDto(value)) return applyRoadVehicleDelta(state, value, now);
  if (isRoadVehicleSnapshotDto(value)) return applyRoadVehicleSnapshot(state, value, now);
  return { ...state, invalidMessages: state.invalidMessages + 1 };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function interpolatedRoadVehicles(
  state: RoadVehicleOverlayState,
  now: number,
  tickPeriodMs: number,
): RoadVehicleDto[] {
  const out: RoadVehicleDto[] = [];
  for (const entry of state.vehicles.values()) {
    const t = clamp((now - entry.lastTickAt) / tickPeriodMs, 0, 1);
    const x = entry.prev.world_coord.x + (entry.current.world_coord.x - entry.prev.world_coord.x) * t;
    const y = entry.prev.world_coord.y + (entry.current.world_coord.y - entry.prev.world_coord.y) * t;
    out.push({ ...entry.current, world_coord: { x, y } });
  }
  return out;
}
```

- [ ] **Step 4: Run to confirm pass**

```bash
npx vitest run tests/backend/roadVehicleState.test.ts
```

Expected: green.

- [ ] **Step 5: Run full vitest suite**

```bash
npx vitest run
```

Expected: green. If any other test reads `state.vehicles.get(id)` directly and expected a `RoadVehicleDto`, update it to read `.current`.

- [ ] **Step 6: Commit**

```bash
git add src/backend/roadVehicleState.ts tests/backend/roadVehicleState.test.ts
git commit -m "feat: buffer prev+current per road vehicle for interpolation"
```

---

## Task 7: Surface `tickPeriodMs` Through `mobilityClient`

**Files:**
- Modify: `src/backend/mobilityClient.ts`
- Modify: `tests/backend/mobilityClient.test.ts`

- [ ] **Step 1: Write failing test**

Append to `tests/backend/mobilityClient.test.ts`:

```ts
it('requireMobilitySnapshot surfaces tickPeriodMs from /world', async () => {
  const worldSummary = {
    protocol_version: 1,
    world_id: 'abutown-main',
    chunk_size: 32,
    loaded_chunks: [],
    tick_period_ms: 100,
  };
  const mobilitySnapshot = {
    protocol_version: 1,
    world_id: 'abutown-main',
    tick: 1,
    agents: [],
    vehicles: [],
    stops: [],
  };
  const roadVehiclesSnapshot = {
    protocol_version: 1,
    world_id: 'abutown-main',
    tick: 1,
    vehicles: [],
  };
  const fetchImpl = (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString();
    if (url.includes('/world')) return Promise.resolve(new Response(JSON.stringify(worldSummary)));
    if (url.includes('/road-vehicles'))
      return Promise.resolve(new Response(JSON.stringify(roadVehiclesSnapshot)));
    return Promise.resolve(new Response(JSON.stringify(mobilitySnapshot)));
  };
  const result = await requireMobilitySnapshot({ baseUrl: 'http://localhost:8080', fetchImpl });
  expect(result.tickPeriodMs).toBe(100);
});
```

(Adjust to match the existing test file's imports/scaffolding.)

- [ ] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/backend/mobilityClient.test.ts
```

Expected: FAIL — current `requireMobilitySnapshot` returns just the state, not `{state, tickPeriodMs}`.

- [ ] **Step 3: Update `requireMobilitySnapshot` return type**

In `src/backend/mobilityClient.ts`:

```ts
export type RequiredMobility = {
  state: MobilityOverlayState;
  tickPeriodMs: number;
};

const DEFAULT_TICK_PERIOD_MS = 100;

export async function requireMobilitySnapshot(
  options: MobilitySnapshotOptions = {},
): Promise<RequiredMobility> {
  const baseUrl = options.baseUrl ?? resolveMobilityBackendBaseUrl();
  const fetchImpl = resolveFetch(options);
  if (!fetchImpl) throw new Error('Mobility fetch transport unavailable');

  const now = options.now ?? Date.now;

  const worldSummary = await requestWorldSummary(baseUrl, fetchImpl);
  const tickPeriodMs = worldSummary.tick_period_ms > 0 ? worldSummary.tick_period_ms : DEFAULT_TICK_PERIOD_MS;

  const mobilityPayload = await requestMobilitySnapshot(baseUrl, fetchImpl);
  const roadVehiclePayload = await requestRoadVehicleSnapshot(baseUrl, fetchImpl);

  let state = createMobilityOverlayState();
  state = markMobilityConnecting(state, now());
  state = applyMobilitySnapshot(state, mobilityPayload, now());
  state = applyRoadVehicleSnapshotToState(state, roadVehiclePayload, now());

  return { state, tickPeriodMs };
}

async function requestWorldSummary(baseUrl: string, fetchImpl: typeof fetch): Promise<WorldSummaryDto> {
  const response = await fetchImpl(new URL('/world', baseUrl).toString());
  if (!response.ok) throw new Error(`World summary HTTP ${response.status}`);
  const payload: unknown = await response.json();
  if (!isWorldSummaryDto(payload)) throw new Error('Invalid world summary payload');
  return payload;
}
```

Import `WorldSummaryDto` and `isWorldSummaryDto` from wherever they live (Task 4 added them). If `requestMobilitySnapshot` / `requestRoadVehicleSnapshot` already exist, keep them as-is.

The existing inner `connect()` function in `connectMobilityBackend` also calls `requestMobilitySnapshot` + `requestRoadVehicleSnapshot` — it does NOT need world summary because the bridge consumer obtained `tickPeriodMs` from the boot path. Leave the bridge's reconnect logic alone.

- [ ] **Step 4: Update boot path in main.ts (preview)**

`src/main.ts` calls `requireMobilitySnapshot({ baseUrl })` and assigns its result. After this task, that assignment needs to destructure:

```ts
const required = await requireMobilitySnapshot({ baseUrl: backendBaseUrl });
mobilityState = required.state;
tickPeriodMs = required.tickPeriodMs;
```

But the actual `tickPeriodMs` usage is Task 9. This task only changes the function signature.

- [ ] **Step 5: Run tests**

```bash
npx vitest run
npx tsc --noEmit
```

Expected: vitest green. `tsc` may flag `src/main.ts` if it consumes the old return shape — that's expected; Task 9 fixes it.

For now, satisfy `tsc` by tweaking the call site in `src/main.ts` to:

```ts
const required = await requireMobilitySnapshot({ baseUrl: backendBaseUrl });
mobilityState = required.state;
```

Don't read `tickPeriodMs` yet — Task 9 wires it. Just unblock the type check.

- [ ] **Step 6: Commit**

```bash
git add src/backend/mobilityClient.ts tests/backend/mobilityClient.test.ts src/main.ts
git commit -m "feat: surface tick period from world summary on boot"
```

---

## Task 8: Drawables Projector Uses Interpolated State

**Files:**
- Modify: `src/render/backendMobilityDrawables.ts`
- Modify: `tests/render/backendMobilityDrawables.test.ts`

- [ ] **Step 1: Update tests**

Replace the existing `tests/render/backendMobilityDrawables.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { pedestriansFromMobilityState, carsFromMobilityState } from '../../src/render/backendMobilityDrawables';
import { applyMobilityDelta, applyMobilitySnapshot, createMobilityOverlayState } from '../../src/backend/mobilityState';
import { applyRoadVehicleDelta, applyRoadVehicleSnapshot } from '../../src/backend/roadVehicleState';

const pedestrianSprites = [
  { sheet: 'pak128/peds.0', frameWidth: 16, frameHeight: 32 },
  { sheet: 'pak128/peds.1', frameWidth: 16, frameHeight: 32 },
];
const vehicleSprites = [
  { sheet: 'pak128/cars.0', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.0' },
  { sheet: 'pak128/cars.1', frameWidth: 32, frameHeight: 32, scale: 1, role: 'vehicle.1' },
];

describe('backendMobilityDrawables (interpolated)', () => {
  it('projects agents at interpolated coord based on now and tickPeriodMs', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [
          {
            id: 'agent:seed:0',
            state: { type: 'walking', link_id: 'link:walk:default', progress: 0 },
            plan_cursor: 0,
            world_coord: { x: 0, y: 0 },
            direction: 'e',
            sprite_key: 'pedestrian:0',
          },
        ],
        vehicles: [],
        stops: [],
      },
      0,
    );
    state = applyMobilityDelta(
      state,
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 2,
        changed_agents: [
          {
            id: 'agent:seed:0',
            state: { type: 'walking', link_id: 'link:walk:default', progress: 0.5 },
            plan_cursor: 0,
            world_coord: { x: 100, y: 0 },
            direction: 'e',
            sprite_key: 'pedestrian:0',
          },
        ],
        changed_vehicles: [],
      },
      100,
    );
    const pedestrians = pedestriansFromMobilityState(state, pedestrianSprites, 150, 100);
    expect(pedestrians).toHaveLength(1);
    expect(pedestrians[0].path[0].x).toBeCloseTo(50, 5);
  });

  it('projects road vehicles at interpolated coord', () => {
    let state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      {
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 1,
        agents: [],
        vehicles: [],
        stops: [],
      },
      0,
    );
    state = {
      ...state,
      roadVehicles: applyRoadVehicleSnapshot(
        state.roadVehicles,
        {
          protocol_version: 1,
          world_id: 'abutown-main',
          tick: 1,
          vehicles: [
            { id: 'road_vehicle:seed:0', world_coord: { x: 0, y: 0 }, direction: 'e', sprite_key: 'vehicle:0' },
          ],
        },
        0,
      ),
    };
    state = {
      ...state,
      roadVehicles: applyRoadVehicleDelta(
        state.roadVehicles,
        {
          protocol_version: 1,
          world_id: 'abutown-main',
          tick: 2,
          changed: [{ id: 'road_vehicle:seed:0', world_coord: { x: 100, y: 0 }, direction: 'e', sprite_key: 'vehicle:0' }],
        },
        100,
      ),
    };
    const cars = carsFromMobilityState(state, vehicleSprites, 150, 100);
    expect(cars).toHaveLength(1);
    expect(cars[0].path[0].x).toBeCloseTo(50, 5);
  });

  it('returns empty arrays when no sprites are available', () => {
    const state = applyMobilitySnapshot(
      createMobilityOverlayState(),
      { protocol_version: 1, world_id: 'abutown-main', tick: 1, agents: [], vehicles: [], stops: [] },
      0,
    );
    expect(pedestriansFromMobilityState(state, [], 0, 100)).toEqual([]);
    expect(carsFromMobilityState(state, [], 0, 100)).toEqual([]);
  });
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
npx vitest run tests/render/backendMobilityDrawables.test.ts
```

Expected: FAIL — `pedestriansFromMobilityState` doesn't yet accept `now` + `tickPeriodMs`.

- [ ] **Step 3: Update the projector**

Replace `src/render/backendMobilityDrawables.ts`:

```ts
import { interpolatedAgents, type MobilityOverlayState } from '../backend/mobilityState';
import { interpolatedRoadVehicles } from '../backend/roadVehicleState';
import type { DirectionDto } from '../backend/mobilityProtocol';

export type Coord = { x: number; y: number };

export type SimutransPedestrianSpriteLike = {
  sheet: string;
  frameWidth?: number;
  frameHeight?: number;
};

export type VehicleSpriteLike = {
  sheet: string;
  frameWidth?: number;
  frameHeight?: number;
  scale?: number;
  role: string;
};

export type BackendPedestrian = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  laneOffset: number;
  sprite: SimutransPedestrianSpriteLike;
  direction: DirectionDto;
};

export type BackendCar = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSpriteLike;
  direction: DirectionDto;
};

const DIRECTION_VECTORS: Record<DirectionDto, Coord> = {
  n: { x: 0, y: -1 },
  ne: { x: 1, y: -1 },
  e: { x: 1, y: 0 },
  se: { x: 1, y: 1 },
  s: { x: 0, y: 1 },
  sw: { x: -1, y: 1 },
  w: { x: -1, y: 0 },
  nw: { x: -1, y: -1 },
};

function spriteIndexFromKey(key: string, modulus: number): number {
  const parts = key.split(':');
  const last = parts[parts.length - 1] ?? '0';
  const n = Number.parseInt(last, 10);
  if (Number.isNaN(n)) return 0;
  return ((n % modulus) + modulus) % modulus;
}

function syntheticPath(start: Coord, direction: DirectionDto): Coord[] {
  const vec = DIRECTION_VECTORS[direction];
  return [start, { x: start.x + vec.x, y: start.y + vec.y }];
}

export function pedestriansFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly SimutransPedestrianSpriteLike[],
  now: number,
  tickPeriodMs: number,
): BackendPedestrian[] {
  if (sprites.length === 0) return [];
  const agents = interpolatedAgents(state, now, tickPeriodMs).sort((a, b) => a.id.localeCompare(b.id));
  const out: BackendPedestrian[] = [];
  for (const agent of agents) {
    const sprite = sprites[spriteIndexFromKey(agent.sprite_key, sprites.length)];
    out.push({
      id: agent.id,
      path: syntheticPath(agent.world_coord, agent.direction),
      offset: 0,
      speed: 0,
      laneOffset: 0,
      sprite,
      direction: agent.direction,
    });
  }
  return out;
}

export function carsFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly VehicleSpriteLike[],
  now: number,
  tickPeriodMs: number,
): BackendCar[] {
  if (sprites.length === 0) return [];
  const vehicles = interpolatedRoadVehicles(state.roadVehicles, now, tickPeriodMs).sort((a, b) =>
    a.id.localeCompare(b.id),
  );
  const out: BackendCar[] = [];
  for (const vehicle of vehicles) {
    const sprite = sprites[spriteIndexFromKey(vehicle.sprite_key, sprites.length)];
    out.push({
      id: vehicle.id,
      path: syntheticPath(vehicle.world_coord, vehicle.direction),
      offset: 0,
      speed: 0,
      sprite,
      direction: vehicle.direction,
    });
  }
  return out;
}
```

- [ ] **Step 4: Run tests to confirm pass**

```bash
npx vitest run tests/render/backendMobilityDrawables.test.ts
npx vitest run
```

Expected: green. The full suite passes because `main.ts`'s call sites still compile — Task 9 wires the new arguments.

Actually `tsc` will fail because the projector signature now requires `now` and `tickPeriodMs`. Add temporary defaults at the call sites in main.ts before this commit lands, OR just include the main.ts wiring in Task 9 and accept a transient build break — but since this is subagent-driven with TDD per task, we want each commit to keep the workspace green. **Solution:** at the end of this task, update the call sites in `src/main.ts` to pass `performance.now()` and `100` as the new args. Task 9 then promotes `100` to the actual cached `tickPeriodMs`.

Add this small main.ts edit at the end of this task — find every `pedestriansFromMobilityState(mobilityState, pedestrianSprites)` and `carsFromMobilityState(mobilityState, vehicleSprites)` and add `, performance.now(), 100`. (There may be 2–3 call sites — at the render-frame builder, at selection/hit-test, at diagnostics.)

- [ ] **Step 5: Verify**

```bash
npx tsc --noEmit
npx vitest run
npm run build
```

Expected: green.

- [ ] **Step 6: Commit**

```bash
git add src/render/backendMobilityDrawables.ts tests/render/backendMobilityDrawables.test.ts src/main.ts
git commit -m "feat: drawables projector interpolates between server ticks"
```

---

## Task 9: Wire `tickPeriodMs` Through `main.ts`

**Files:**
- Modify: `src/main.ts`

- [ ] **Step 1: Cache `tickPeriodMs` at boot**

In `src/main.ts`, locate the boot block that called `requireMobilitySnapshot`. Add a module-level state:

```ts
let mobilityTickPeriodMs = 100;
```

Update the boot:

```ts
const required = await requireMobilitySnapshot({ baseUrl: backendBaseUrl });
mobilityState = required.state;
mobilityTickPeriodMs = required.tickPeriodMs;
```

- [ ] **Step 2: Replace the hardcoded 100 at projector call sites**

Search:

```bash
grep -n "pedestriansFromMobilityState\|carsFromMobilityState" src/main.ts
```

Each call site currently passes `performance.now(), 100`. Replace `100` with `mobilityTickPeriodMs`:

```ts
const pedestrians = pedestriansFromMobilityState(mobilityState, pedestrianSprites, performance.now(), mobilityTickPeriodMs);
const cars = carsFromMobilityState(mobilityState, vehicleSprites, performance.now(), mobilityTickPeriodMs);
```

- [ ] **Step 3: Verify**

```bash
npx tsc --noEmit
npx vitest run
npm run build
```

Expected: green.

- [ ] **Step 4: Commit**

```bash
git add src/main.ts
git commit -m "feat: thread tick period from world summary into render loop"
```

---

## Task 10: E2E Smoke Asserts Smooth Motion

**Files:**
- Modify: `tests/e2e/render-smoke.spec.ts`

- [ ] **Step 1: Add an interpolation assertion**

In `tests/e2e/render-smoke.spec.ts`, after the existing block that obtains `state.city.mobilityAgents.agents`, add a new test or extend the existing one:

```ts
// Verify smooth motion: take two reads ~50 ms apart and confirm a populated agent moves.
const firstSample = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
const sampleAgent = firstSample.city.mobilityAgents.agents[0];
if (!sampleAgent) {
  throw new Error('Expected at least one mobility agent in the render sample');
}
await page.waitForTimeout(80);
const secondSample = JSON.parse(await page.evaluate(() => window.render_game_to_text?.() ?? ''));
const sameAgentLater = secondSample.city.mobilityAgents.agents.find(
  (entry: { id: string }) => entry.id === sampleAgent.id,
);
expect(sameAgentLater).toBeDefined();
const movedX = Math.abs(sameAgentLater.coord.x - sampleAgent.coord.x);
const movedY = Math.abs(sameAgentLater.coord.y - sampleAgent.coord.y);
expect(movedX + movedY).toBeGreaterThan(0);
```

(Adjust the property names to whatever the diagnostics block emits — `coord` is per the Phase-1 shape; `world_coord` may be the actual key. Verify via the local `render_game_to_text` output.)

- [ ] **Step 2: Confirm e2e compiles**

```bash
npx tsc --noEmit
```

Don't run the e2e itself — it needs both Vite preview and the backend running, which isn't part of the subagent harness.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/render-smoke.spec.ts
git commit -m "test: render smoke asserts interpolated agent motion between frames"
```

---

## Task 11: Final Quality Gate + progress.md

**Files:**
- Modify: `progress.md`

- [ ] **Step 1: Run formatter, full test suite, clippy, frontend tests, build**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
cargo test --locked --manifest-path backend/Cargo.toml --workspace
cargo clippy --locked --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
npx vitest run
npm run build
```

Expected: all green. If `cargo fmt --check` complains, run `cargo fmt --manifest-path backend/Cargo.toml --all` and stage the changes.

- [ ] **Step 2: Append progress note**

Append to `progress.md`:

```
2026-05-16T<HH:MM:SS>.000Z - Mobility frame interpolation: backend tick raised from 1 Hz to 10 Hz, WorldSummaryDto exposes tick_period_ms, frontend mobility/road-vehicle state now buffers prev+current+lastTickAt per entity, drawables projector linearly interpolates world_coord by t = (now - lastTickAt) / tick_period_ms. Canvas renders smooth 60 fps motion between server ticks.
```

Use the current UTC timestamp.

- [ ] **Step 3: Commit**

```bash
git add progress.md backend/
git commit -m "chore: format and record mobility interpolation progress"
```

---

## Self-Review

- **Spec coverage:**
  - Backend 10 Hz tick → Task 2.
  - `WorldSummaryDto.tick_period_ms` → Tasks 1, 2.
  - Timing-sensitive backend tests → Task 3.
  - Frontend `WorldSummaryDto` mirror → Task 4.
  - `InterpolatedEntry` and buffered agent/vehicle maps → Task 5.
  - Buffered road-vehicle map → Task 6.
  - Surface `tickPeriodMs` through `requireMobilitySnapshot` → Task 7.
  - Drawables projector accepts `now` + `tickPeriodMs` → Task 8.
  - `main.ts` reads tick period at boot, passes per frame → Task 9.
  - E2E smooth-motion assertion → Task 10.
  - Quality gate + progress note → Task 11.
- **Placeholder scan:** every step has concrete code; no TBDs.
- **Type consistency:** `InterpolatedEntry<T>` used consistently in mobility state and analogue in road-vehicle state; `interpolatedAgents` / `interpolatedVehicles` / `interpolatedRoadVehicles` return `Dto[]` shapes the projector consumes; `tickPeriodMs` is `number` in TS, `u32` in Rust, both keyed `tick_period_ms` on the wire; `SIMULATION_TICK_INTERVAL = Duration::from_millis(100)` matches `TICK_PERIOD_MS = 100`.
- **Existing fixture risk:** Tasks 5 and 6 explicitly call for grepping and updating any test that reads `.world_coord` directly off the state map — that surface changed.
