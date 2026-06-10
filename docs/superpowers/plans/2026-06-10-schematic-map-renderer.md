# Schematic Map Renderer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the map into the schematic (Mini-Metro-school) language with semantic zoom, and make economy flows visible via one additive wire field.

**Architecture:** Evolutionary Canvas2D refactor per `docs/superpowers/specs/2026-06-10-schematic-map-renderer-design.md`. The monolithic `minimalMapRenderer.ts` is split into four layer drawers orchestrated by a slim renderer; a `designTokens.ts` module becomes the only color/geometry source; a pure `layerBlend.ts` drives the zoom cross-fade. Backend adds an in-memory (never persisted) `FlowRateEwma` resource aggregated from `RealizedFlows` and ships it as `EconomySnapshot.flows = 6`.

**Tech Stack:** TypeScript + Canvas2D + vitest (frontend); Rust + bevy_ecs + prost (backend); buf for TS proto gen; Playwright smoke script.

**Working context:** Execute in the worktree `.claude/worktrees/schematic-renderer` on branch `graphics/schematic-map-renderer` (already exists, spec committed). All paths below are relative to that worktree root.

**Hard project rules (from CLAUDE.md / memory):**
- NEVER run two cargo processes at once. Route every cargo invocation through `scripts/cargo-serial.sh`, scoped exactly as written in the steps (no `--workspace --all-targets`). Before any cargo step: `pgrep -f cargo` and wait/clear orphans.
- The feature crosses the frontend↔backend wire → the browser smoke (Task 14) is the acceptance gate, not "tests pass".
- No fallback/legacy shims: missing `flows` simply renders nothing; rate ≤ 0 is not drawn.

---

## File structure

| File | Responsibility |
|---|---|
| `src/render/designTokens.ts` (new) | Palette, glyph geometry, zoom bands — single source of truth |
| `src/render/layerBlend.ts` (new) | Pure `scale → {opacity, detail}` per layer |
| `src/render/drawNetwork.ts` (new) | L0: ground, terrain, water, roads, rail, buildings, trees, details, edge fades |
| `src/render/drawMarkets.ts` (new) | L1: market nodes (radius/ring/trend/pulse) + pure helpers |
| `src/render/drawAgents.ts` (new) | L2: pedestrian/trader/car glyphs + state mapping |
| `src/render/drawFlows.ts` (new) | L3: flow curves + pure geometry helpers |
| `src/render/minimalMapRenderer.ts` | Shrinks to orchestrator (culling, projection, layer calls) |
| `src/render/drawOrder.ts` | Gains kinds `flow`, `marketNode` |
| `src/render/backendMobilityDrawables.ts` | `BackendPedestrian` gains `stateType` |
| `src/backend/mobilityProtocol.ts` | `EconomyFlowDto` + converter |
| `src/backend/economyState.ts` | `EconomyOverlayState.flows` |
| `src/main.ts` | Pass `goods`/`flows` into renderer; flow-count diagnostic |
| `src/app/runtimeDiagnostics.ts` | `economyFlowCount` |
| `backend/crates/sim-core/src/economy/flow_shipments.rs` | `RealizedFlow.qty` |
| `backend/crates/sim-core/src/economy/macro_flow.rs` | populate `qty` |
| `backend/crates/sim-core/src/economy/flow_telemetry.rs` (new) | `FlowRateEwma` + update fn |
| `backend/crates/sim-core/src/economy/systems.rs` | call EWMA update on interval ticks |
| `backend/crates/protocol/proto/abutown.proto` | `EconomyFlow`, `flows = 6` |
| `backend/crates/sim-server/src/app/mod.rs` | include flows in `build_economy_snapshot` |
| `scripts/smoke-schematic.mjs` (new) | Browser smoke (acceptance gate) |

---

### Task 1: designTokens.ts

**Files:**
- Create: `src/render/designTokens.ts`
- Test: `tests/render/designTokens.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// tests/render/designTokens.test.ts
import { describe, expect, it } from 'vitest';
import * as tokens from '../../src/render/designTokens';

describe('designTokens', () => {
  it('zoom bands are ordered inside the camera scale range (0.18..2.8)', () => {
    expect(tokens.ZOOM_ECONOMY_MAX).toBeGreaterThan(0.18);
    expect(tokens.ZOOM_CITY_MIN).toBeGreaterThan(tokens.ZOOM_ECONOMY_MAX);
    expect(tokens.ZOOM_CITY_MIN).toBeLessThan(2.8);
  });

  it('every good id used by the sim (1..5) has a flow color', () => {
    for (const id of [1, 2, 3, 4, 5]) {
      expect(tokens.GOOD_COLORS[id], `good ${id}`).toMatch(/^#[0-9a-f]{6}$/);
    }
    expect(tokens.GOOD_COLOR_FALLBACK).toMatch(/^#[0-9a-f]{6}$/);
  });

  it('opacity floors are sane', () => {
    expect(tokens.FLOW_MIN_OPACITY).toBeGreaterThan(0);
    expect(tokens.FLOW_MIN_OPACITY).toBeLessThan(1);
    expect(tokens.AGENT_SHIMMER_OPACITY).toBeGreaterThan(0);
    expect(tokens.AGENT_SHIMMER_OPACITY).toBeLessThan(1);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/render/designTokens.test.ts`
Expected: FAIL — cannot resolve `../../src/render/designTokens`

- [ ] **Step 3: Write the module**

```typescript
// src/render/designTokens.ts
// Single source of truth for the schematic visual vocabulary.
// Spec: docs/superpowers/specs/2026-06-10-schematic-map-renderer-design.md §1

// --- ground & terrain (L0) ---
export const GROUND = '#e9ede1';
export const OUT_OF_WORLD = '#182018';
export const WATER = '#cfe3ea';
export const RIVERBANK = '#dcebe7';
export const PARK = '#dde6cf';
export const PLAZA = '#eee4cd';

// --- network (L0) ---
export const ROAD_INK = '#3a3f47';
export const ROAD_CENTER_DASH = GROUND;
export const RAIL_CASING = 'rgba(122, 131, 135, 0.32)';
export const RAIL_CORE = 'rgba(122, 131, 135, 0.42)';
export const TREE = '#9bb98a';
export const DETAIL = 'rgba(92, 97, 92, 0.30)';
export const BUILDING_RESIDENTIAL = '#e2a14e';
export const BUILDING_COMMERCIAL = '#9fb4bb';
export const BUILDING_CIVIC = '#cbb878';
export const BUILDING_INDUSTRIAL = '#b3a6c9';

// --- agents (L2) ---
export const AGENT_INK = '#2e3440';
export const TRADER_RED = '#c0392b';
export const SELECTION_HALO_AGENT = '#a87309';
export const SELECTION_HALO_VEHICLE = '#166c83';
export const VEHICLE_COLORS = ['#e85d75', '#3f8fc7', '#49a879', '#e5a944', '#8c73c8', '#ef7f5a', '#28a6b0'] as const;

// --- markets (L1) ---
export const MARKET_ORANGE = '#d9783a';

// --- flows (L3) — keyed by backend GoodId (goods.rs: FOOD=1 WOOD=2 IRON=3 TOOLS=4 RAW=5) ---
export const GOOD_COLORS: Readonly<Record<number, string>> = {
  1: '#7a9e4f', // FOOD green
  2: '#8c6f4a', // WOOD brown
  3: '#5f7d8c', // IRON slate
  4: '#8c73c8', // TOOLS violet
  5: '#d98c3a', // RAW orange
};
export const GOOD_COLOR_FALLBACK = '#8a8f94';

// --- semantic zoom bands (camera scale runs 0.18..2.8, see src/main.ts) ---
export const ZOOM_ECONOMY_MAX = 0.6;
export const ZOOM_CITY_MIN = 1.0;
export const FLOW_MIN_OPACITY = 0.15;
export const AGENT_SHIMMER_OPACITY = 0.35;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/render/designTokens.test.ts`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add src/render/designTokens.ts tests/render/designTokens.test.ts
git commit -m "feat(render): design tokens — schematic palette, good colors, zoom bands"
```

---

### Task 2: layerBlend.ts

**Files:**
- Create: `src/render/layerBlend.ts`
- Test: `tests/render/layerBlend.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// tests/render/layerBlend.test.ts
import { describe, expect, it } from 'vitest';
import { layerBlend } from '../../src/render/layerBlend';
import {
  AGENT_SHIMMER_OPACITY,
  FLOW_MIN_OPACITY,
  ZOOM_CITY_MIN,
  ZOOM_ECONOMY_MAX,
} from '../../src/render/designTokens';

describe('layerBlend', () => {
  it('network and markets are always fully visible', () => {
    for (const scale of [0.18, 0.6, 1.0, 2.8]) {
      expect(layerBlend('network', scale)).toEqual({ opacity: 1, detail: 'individual' });
      expect(layerBlend('markets', scale)).toEqual({ opacity: 1, detail: 'individual' });
    }
  });

  it('agents: shimmer in the economy band, full in the city band, monotone between', () => {
    expect(layerBlend('agents', 0.18)).toEqual({ opacity: AGENT_SHIMMER_OPACITY, detail: 'aggregate' });
    expect(layerBlend('agents', ZOOM_ECONOMY_MAX).opacity).toBeCloseTo(AGENT_SHIMMER_OPACITY);
    expect(layerBlend('agents', ZOOM_CITY_MIN)).toEqual({ opacity: 1, detail: 'individual' });
    expect(layerBlend('agents', 2.8)).toEqual({ opacity: 1, detail: 'individual' });
    const mid = layerBlend('agents', (ZOOM_ECONOMY_MAX + ZOOM_CITY_MIN) / 2).opacity;
    expect(mid).toBeGreaterThan(AGENT_SHIMMER_OPACITY);
    expect(mid).toBeLessThan(1);
    // detail flips to individual as soon as we leave the economy band
    expect(layerBlend('agents', ZOOM_ECONOMY_MAX + 0.01).detail).toBe('individual');
  });

  it('flows: full in the economy band, hint in the city band, monotone between', () => {
    expect(layerBlend('flows', 0.18)).toEqual({ opacity: 1, detail: 'individual' });
    expect(layerBlend('flows', ZOOM_CITY_MIN).opacity).toBeCloseTo(FLOW_MIN_OPACITY);
    expect(layerBlend('flows', 2.8)).toEqual({ opacity: FLOW_MIN_OPACITY, detail: 'aggregate' });
    const mid = layerBlend('flows', (ZOOM_ECONOMY_MAX + ZOOM_CITY_MIN) / 2).opacity;
    expect(mid).toBeLessThan(1);
    expect(mid).toBeGreaterThan(FLOW_MIN_OPACITY);
    // chevrons (detail 'individual') stay on until the city band starts
    expect(layerBlend('flows', ZOOM_CITY_MIN - 0.01).detail).toBe('individual');
  });

  it('clamps outside the camera range', () => {
    expect(layerBlend('agents', 0.0001).opacity).toBeCloseTo(AGENT_SHIMMER_OPACITY);
    expect(layerBlend('flows', 100).opacity).toBeCloseTo(FLOW_MIN_OPACITY);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/render/layerBlend.test.ts`
Expected: FAIL — cannot resolve `../../src/render/layerBlend`

- [ ] **Step 3: Write the module**

```typescript
// src/render/layerBlend.ts
import {
  AGENT_SHIMMER_OPACITY,
  FLOW_MIN_OPACITY,
  ZOOM_CITY_MIN,
  ZOOM_ECONOMY_MAX,
} from './designTokens';

export type LayerKey = 'network' | 'markets' | 'agents' | 'flows';
export type LayerBlend = { opacity: number; detail: 'aggregate' | 'individual' };

/** 0 at/below the economy band, 1 at/above the city band, linear between. */
function cityness(scale: number): number {
  if (scale <= ZOOM_ECONOMY_MAX) return 0;
  if (scale >= ZOOM_CITY_MIN) return 1;
  return (scale - ZOOM_ECONOMY_MAX) / (ZOOM_CITY_MIN - ZOOM_ECONOMY_MAX);
}

export function layerBlend(layer: LayerKey, scale: number): LayerBlend {
  const t = cityness(scale);
  switch (layer) {
    case 'network':
    case 'markets':
      return { opacity: 1, detail: 'individual' };
    case 'agents':
      return {
        opacity: AGENT_SHIMMER_OPACITY + (1 - AGENT_SHIMMER_OPACITY) * t,
        detail: t > 0 ? 'individual' : 'aggregate',
      };
    case 'flows':
      return {
        opacity: 1 - (1 - FLOW_MIN_OPACITY) * t,
        detail: t < 1 ? 'individual' : 'aggregate',
      };
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/render/layerBlend.test.ts`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/render/layerBlend.ts tests/render/layerBlend.test.ts
git commit -m "feat(render): layerBlend — pure semantic-zoom cross-fade"
```

---

### Task 3: drawOrder gains `flow` and `marketNode`

**Files:**
- Modify: `src/render/drawOrder.ts`
- Test: `tests/render/drawOrder.test.ts` (exists — extend)

- [ ] **Step 1: Write the failing test (append to the existing describe block)**

```typescript
it('flows draw above roads but below actors; market nodes draw above everything', () => {
  const at = (type: Parameters<typeof compareDrawableOrder>[0]['type']) => ({ type, isoY: 10, x: 5 });
  expect(compareDrawableOrder(at('road'), at('flow'))).toBeLessThan(0);
  expect(compareDrawableOrder(at('flow'), at('car'))).toBeLessThan(0);
  expect(compareDrawableOrder(at('flow'), at('pedestrian'))).toBeLessThan(0);
  expect(compareDrawableOrder(at('pedestrian'), at('marketNode'))).toBeLessThan(0);
  expect(compareDrawableOrder(at('building'), at('marketNode'))).toBeLessThan(0);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/render/drawOrder.test.ts`
Expected: FAIL — TS error: `'flow'` not assignable to `DrawableType` (vitest transpiles loosely; if it runs, the priority assertions fail instead)

- [ ] **Step 3: Implement**

In `src/render/drawOrder.ts`:

```typescript
export type DrawableType =
  | 'rail' | 'road' | 'railStation' | 'flow' | 'detail' | 'tree' | 'building'
  | 'car' | 'pedestrian' | 'marketNode';
```

In `drawPriority`, renumber so the full order is: road 0, rail 1, railStation 2, **flow 3**, car 4, pedestrian 5, detail 6, tree 8, building 8 (default), **marketNode 9**:

```typescript
export function drawPriority(type: DrawableType): number {
  if (type === 'road') return 0;
  if (type === 'rail') return 1;
  if (type === 'railStation') return 2;
  if (type === 'flow') return 3;
  if (type === 'car') return 4;
  if (type === 'pedestrian') return 5;
  if (type === 'detail') return 6;
  if (type === 'marketNode') return 9;
  return 8; // tree, building
}
```

In `isFlatInfrastructure`, add `flow` (so flows sort with infrastructure, below actors); leave `isActor` unchanged but add `marketNode` to it so the flat-vs-actor rule lifts nodes above flat layers:

```typescript
function isFlatInfrastructure(type: DrawableType): boolean {
  return type === 'road' || type === 'rail' || type === 'railStation' || type === 'flow';
}

function isActor(type: DrawableType): boolean {
  return type === 'car' || type === 'pedestrian' || type === 'marketNode';
}
```

- [ ] **Step 4: Run the whole drawOrder test file**

Run: `npx vitest run tests/render/drawOrder.test.ts`
Expected: PASS (existing assertions must stay green — the renumbering preserves the old relative order)

- [ ] **Step 5: Commit**

```bash
git add src/render/drawOrder.ts tests/render/drawOrder.test.ts
git commit -m "feat(render): drawOrder kinds flow + marketNode"
```

---

### Task 4 (backend): `RealizedFlow.qty`

**Files:**
- Modify: `backend/crates/sim-core/src/economy/flow_shipments.rs:64-78` (struct)
- Modify: `backend/crates/sim-core/src/economy/macro_flow.rs:1055` (push site)
- Test: inline `#[cfg(test)]` tests in `macro_flow.rs` (existing module)

- [ ] **Step 1: Check for running cargo, then write the failing test**

Run first: `pgrep -f cargo` — if anything is running, wait for it to finish.

Add to the existing test module in `macro_flow.rs` (imports already present at its top):

```rust
#[test]
fn realized_flows_carry_shipped_quantity() {
    // Reuse the existing macro-flow test harness in this module: find the test
    // that asserts `realized.0` is non-empty after `run_macro_flow_at_tick`
    // (search for `RealizedFlows` in this file's test module) and extend a copy:
    // after the run, every realized flow must carry the planned positive qty.
    // ... (same setup as that test) ...
    // New assertion:
    for flow in &realized.0 {
        assert!(flow.qty > 0, "realized flow must carry shipped qty, got {}", flow.qty);
    }
}
```

(The setup is copied verbatim from the neighbouring test that already exercises `run_macro_flow_at_tick` and asserts on `realized.0` — copy it, rename, add the qty assertion.)

- [ ] **Step 2: Run to verify it fails to compile**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --lib economy::macro_flow`
Expected: COMPILE ERROR — `RealizedFlow` has no field `qty`

- [ ] **Step 3: Implement**

In `flow_shipments.rs`, add the field:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealizedFlow {
    pub src: MarketId,
    pub dst: MarketId,
    pub good: GoodId,
    pub qty: i64,
    pub p_src: Money,
    pub p_dst: Money,
    pub dist: i64,
}
```

In `macro_flow.rs:1055` (the `realized.0.push` inside `if eflow.q > 0`):

```rust
realized.0.push(crate::economy::RealizedFlow {
    src: eflow.src,
    dst: eflow.dst,
    good: eflow.good,
    qty: eflow.q,
    p_src: eflow.p_src,
    p_dst: eflow.p_dst,
    dist: eflow.dist,
});
```

Fix every other `RealizedFlow { ... }` construction the compiler reports (test fixtures) by adding a sensible `qty` (use the quantity the fixture ships, or `1`).

- [ ] **Step 4: Run scoped tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --lib economy::`
Expected: PASS (all economy tests, including the new one)

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/flow_shipments.rs backend/crates/sim-core/src/economy/macro_flow.rs
git commit -m "feat(economy): RealizedFlow carries shipped qty"
```

---

### Task 5 (backend): `FlowRateEwma` resource

**Files:**
- Create: `backend/crates/sim-core/src/economy/flow_telemetry.rs`
- Modify: `backend/crates/sim-core/src/economy/mod.rs` (module + re-export + plugin resource registration — register where `init_resource::<RealizedFlows>()` happens; find it with `grep -rn 'RealizedFlows' backend/crates/sim-core/src/economy/mod.rs`)
- Modify: `backend/crates/sim-core/src/economy/systems.rs:541-588` (`run_macro_flow_system`)

- [ ] **Step 1: Write the failing tests (inline in the new module)**

```rust
// backend/crates/sim-core/src/economy/flow_telemetry.rs
//! In-memory (never persisted) EWMA of realized macro-flow quantities per
//! (src, dst, good) edge. Powers the on-wire EconomySnapshot.flows field.
//! Deliberately NOT part of EconomyPersistSnapshot: a restart reconverges in
//! a few intervals, and persisting it would force an economy_snapshots wipe.

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;

use super::money::integer_ewma;
use super::{GoodId, MarketId, Money, RealizedFlows};

pub type FlowKey = (MarketId, MarketId, GoodId);

/// Smoothing weight for new observations, in basis points.
pub const FLOW_RATE_ALPHA_BPS: u16 = 3_000;

#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowRateEwma(pub BTreeMap<FlowKey, Money>);

/// Fold the current interval's realized flows into the EWMA. Edges that shipped
/// nothing decay toward zero and are dropped once they reach it.
pub fn update_flow_rate_ewma(ewma: &mut FlowRateEwma, realized: &RealizedFlows) {
    let mut current: BTreeMap<FlowKey, i64> = BTreeMap::new();
    for flow in &realized.0 {
        *current.entry((flow.src, flow.dst, flow.good)).or_insert(0) += flow.qty;
    }
    let keys: Vec<FlowKey> = ewma.0.keys().copied().chain(current.keys().copied()).collect();
    for key in keys {
        let old = ewma.0.get(&key).copied().unwrap_or(Money(0));
        let cur = Money(current.get(&key).copied().unwrap_or(0));
        let next = integer_ewma(old, cur, FLOW_RATE_ALPHA_BPS)
            .expect("flow ewma: alpha is a valid const and qty magnitudes cannot overflow i128");
        if next.0 <= 0 {
            ewma.0.remove(&key);
        } else {
            ewma.0.insert(key, next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::{GOOD_FOOD, RealizedFlow};

    fn realized(entries: &[(u32, u32, i64)]) -> RealizedFlows {
        RealizedFlows(
            entries
                .iter()
                .map(|&(src, dst, qty)| RealizedFlow {
                    src: MarketId(src),
                    dst: MarketId(dst),
                    good: GOOD_FOOD,
                    qty,
                    p_src: Money(0),
                    p_dst: Money(0),
                    dist: 1,
                })
                .collect(),
        )
    }

    #[test]
    fn first_observation_is_alpha_weighted() {
        let mut ewma = FlowRateEwma::default();
        update_flow_rate_ewma(&mut ewma, &realized(&[(1, 2, 1000)]));
        // 0.3 * 1000 = 300
        assert_eq!(ewma.0[&(MarketId(1), MarketId(2), GOOD_FOOD)], Money(300));
    }

    #[test]
    fn same_edge_entries_sum_before_smoothing() {
        let mut ewma = FlowRateEwma::default();
        update_flow_rate_ewma(&mut ewma, &realized(&[(1, 2, 600), (1, 2, 400)]));
        assert_eq!(ewma.0[&(MarketId(1), MarketId(2), GOOD_FOOD)], Money(300));
    }

    #[test]
    fn idle_edges_decay_and_are_eventually_dropped() {
        let mut ewma = FlowRateEwma::default();
        update_flow_rate_ewma(&mut ewma, &realized(&[(1, 2, 10)]));
        assert!(ewma.0.contains_key(&(MarketId(1), MarketId(2), GOOD_FOOD)));
        for _ in 0..64 {
            update_flow_rate_ewma(&mut ewma, &realized(&[]));
        }
        assert!(ewma.0.is_empty(), "decayed-to-zero edges must be dropped, got {:?}", ewma.0);
    }
}
```

(If `MarketId`'s constructor differs from `MarketId(u32)`, mirror how the neighbouring economy tests construct it.)

- [ ] **Step 2: Wire the module and run to verify the tests fail/compile**

In `mod.rs`: `pub mod flow_telemetry;` plus `pub use flow_telemetry::{update_flow_rate_ewma, FlowRateEwma};`, and register `.init_resource::<FlowRateEwma>()` directly next to the existing `RealizedFlows` registration.

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --lib economy::flow_telemetry`
Expected: PASS (the module above is written test-first as one unit; if any test fails, fix before proceeding)

- [ ] **Step 3: Call the update from the macro-flow system**

In `systems.rs` `run_macro_flow_system` (lines 541–588): add `mut flow_ewma: ResMut<FlowRateEwma>` to the system params and, immediately after the `run_macro_flow_at_tick(...)` call succeeds, gate on the same interval condition the macro flow itself uses:

```rust
if config.macro_flow_interval_ticks != 0
    && current_tick.is_multiple_of(config.macro_flow_interval_ticks)
{
    crate::economy::update_flow_rate_ewma(&mut flow_ewma, &realized);
}
```

(Use the same `current_tick` / `config` bindings already in scope in that system; `realized` is the `RealizedFlows` resource it already passes down.)

- [ ] **Step 4: Run scoped tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core --lib economy::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-core/src/economy/flow_telemetry.rs backend/crates/sim-core/src/economy/mod.rs backend/crates/sim-core/src/economy/systems.rs
git commit -m "feat(economy): in-memory FlowRateEwma over realized macro flows"
```

---

### Task 6: proto `EconomyFlow` + roundtrip + TS regen

**Files:**
- Modify: `backend/crates/protocol/proto/abutown.proto:66-72`
- Test: `backend/crates/protocol/src/lib.rs` (extend `roundtrip_economy_snapshot`, lines ~684-709)
- Regenerate: `src/backend/proto/abutown_pb.ts` via `node scripts/generate-proto-ts.mjs`

- [ ] **Step 1: Extend the proto**

```protobuf
message EconomySnapshot {
  uint32 protocol_version = 1;
  string world_id = 2;
  uint64 tick = 3;
  repeated EconomyMarket markets = 4;
  repeated EconomyMarketGood goods = 5;
  repeated EconomyFlow flows = 6;
}

message EconomyFlow {
  uint32 src_market_id = 1;
  uint32 dst_market_id = 2;
  uint32 good_id = 3;
  // EWMA of shipped quantity per macro-flow interval (raw good units, NOT money).
  int64 rate = 4;
}
```

- [ ] **Step 2: Extend the roundtrip test**

In `roundtrip_economy_snapshot`, add to the constructed message:

```rust
flows: vec![EconomyFlow {
    src_market_id: 9003,
    dst_market_id: 9004,
    good_id: 1,
    rate: 250,
}],
```

- [ ] **Step 3: Run protocol tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p abutown-protocol`
Expected: PASS (prost regenerates from build.rs; requires `protoc` in PATH)

- [ ] **Step 4: Regenerate the TS proto**

Run: `node scripts/generate-proto-ts.mjs`
Expected: `src/backend/proto/abutown_pb.ts` now exports `EconomyFlow` and `EconomySnapshot.flows`. Verify: `grep -n 'EconomyFlow' src/backend/proto/abutown_pb.ts`

- [ ] **Step 5: Commit**

```bash
git add backend/crates/protocol/proto/abutown.proto backend/crates/protocol/src/lib.rs src/backend/proto/
git commit -m "feat(protocol): additive EconomySnapshot.flows wire field"
```

---

### Task 7 (backend): ship flows in `build_economy_snapshot`

**Files:**
- Modify: `backend/crates/sim-server/src/app/mod.rs:275-325`
- Test: `backend/crates/sim-server/src/app/tests.rs`

- [ ] **Step 1: Write the failing test**

In `app/tests.rs`, find the existing test that exercises `build_economy_snapshot` (search `build_economy_snapshot`). Add a sibling test that (a) inserts a `FlowRateEwma` with one entry into the test world, (b) builds the snapshot, (c) asserts:

```rust
#[test]
fn economy_snapshot_includes_flow_rates() {
    // ...same world setup as the existing build_economy_snapshot test...
    world.insert_resource(sim_core::economy::FlowRateEwma(
        [((sim_core::economy::MarketId(9003), sim_core::economy::MarketId(9004), sim_core::economy::GOOD_FOOD), sim_core::economy::Money(250))]
            .into_iter()
            .collect(),
    ));
    let snapshot = build_economy_snapshot(&world, &world_id, 7);
    assert_eq!(snapshot.flows.len(), 1);
    let flow = &snapshot.flows[0];
    assert_eq!((flow.src_market_id, flow.dst_market_id, flow.good_id, flow.rate), (9003, 9004, 1, 250));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server economy_snapshot_includes_flow_rates`
Expected: COMPILE ERROR — `w::EconomySnapshot` has no field `flows` populated / missing field

- [ ] **Step 3: Implement**

In `build_economy_snapshot` (mod.rs:275-325), before the final struct literal add:

```rust
let flows = world
    .resource::<sim_core::economy::FlowRateEwma>()
    .0
    .iter()
    .map(|(&(src, dst, good), &rate)| w::EconomyFlow {
        src_market_id: src.0,
        dst_market_id: dst.0,
        good_id: good.0,
        rate: rate.0,
    })
    .collect();
```

and add `flows,` to the `w::EconomySnapshot { ... }` literal. (If `MarketId`/`GoodId` fields aren't pub-accessible as `.0`, use their existing accessor — check how `markets` mapping extracts ids a few lines above.)

- [ ] **Step 4: Run scoped tests**

Run: `scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add backend/crates/sim-server/src/app/mod.rs backend/crates/sim-server/src/app/tests.rs
git commit -m "feat(server): EconomySnapshot ships FlowRateEwma as flows"
```

---

### Task 8 (frontend wire): `EconomyFlowDto` + `economyState.flows`

**Files:**
- Modify: `src/backend/mobilityProtocol.ts` (economy DTO block, ~line 556)
- Modify: `src/backend/economyState.ts`
- Test: `tests/backend/economyState.test.ts` (extend existing)

- [ ] **Step 1: Write the failing test (extend the existing economyState suite)**

```typescript
it('stores flows from the snapshot', () => {
  const message = makeEconomySnapshotMessage({
    // extend the suite's existing snapshot factory with:
    flows: [{ srcMarketId: 9003, dstMarketId: 9004, goodId: 1, rate: 250n }],
  });
  const state = applyEconomyServerMessage(createEconomyOverlayState(), message);
  expect(state.flows).toEqual([{ srcMarketId: 9003, dstMarketId: 9004, goodId: 1, rate: 250 }]);
});

it('starts with no flows', () => {
  expect(createEconomyOverlayState().flows).toEqual([]);
});
```

(Adapt to the suite's existing message-construction helper — it builds proto `ServerMessage` objects with `create(ServerMessageSchema, ...)`; mirror how `markets`/`goods` are passed there. int64 proto fields surface as `bigint` on the proto object and are converted with `Number()` in the DTO.)

- [ ] **Step 2: Run to verify it fails**

Run: `npx vitest run tests/backend/economyState.test.ts`
Expected: FAIL — `state.flows` is `undefined`

- [ ] **Step 3: Implement**

`mobilityProtocol.ts` — extend the economy DTO block:

```typescript
export type EconomyFlowDto = { srcMarketId: number; dstMarketId: number; goodId: number; rate: number };
export type EconomySnapshotDto = { tick: number; markets: MarketLocationDto[]; goods: MarketGoodDto[]; flows: EconomyFlowDto[] };

export function economySnapshotFromProto(p: EconomySnapshot): EconomySnapshotDto {
  return {
    tick: Number(p.tick),
    markets: p.markets.map((m) => ({ marketId: m.marketId, name: m.name, tileX: m.tileX, tileY: m.tileY, wagePaidLastTick: Number(m.wagePaidLastTick) })),
    goods: p.goods.map((g) => ({ marketId: g.marketId, goodId: g.goodId, lastSettlementPrice: Number(g.lastSettlementPrice), ewmaReferencePrice: Number(g.ewmaReferencePrice), tradedQtyLastTick: Number(g.tradedQtyLastTick), unmetDemandLastTick: Number(g.unmetDemandLastTick), unsoldSupplyLastTick: Number(g.unsoldSupplyLastTick) })),
    flows: p.flows.map((f) => ({ srcMarketId: f.srcMarketId, dstMarketId: f.dstMarketId, goodId: f.goodId, rate: Number(f.rate) })),
  };
}
```

`economyState.ts` — full new contents (it is a 23-line file):

```typescript
import type { ServerMessage } from './proto/abutown_pb';
import {
  economySnapshotFromProto,
  type EconomyFlowDto,
  type MarketLocationDto,
  type MarketGoodDto,
} from './mobilityProtocol';

export type EconomyOverlayState = {
  tick: number;
  markets: Map<number, MarketLocationDto>; // by marketId
  goods: Map<string, MarketGoodDto>; // key `${marketId}:${goodId}`
  flows: EconomyFlowDto[];
};

export function createEconomyOverlayState(): EconomyOverlayState {
  return { tick: 0, markets: new Map(), goods: new Map(), flows: [] };
}

export function applyEconomyServerMessage(
  state: EconomyOverlayState,
  message: ServerMessage,
): EconomyOverlayState {
  if (message.body.case !== 'economySnapshot') return state;
  const dto = economySnapshotFromProto(message.body.value);
  const markets = new Map(dto.markets.map((m) => [m.marketId, m]));
  const goods = new Map(dto.goods.map((g) => [`${g.marketId}:${g.goodId}`, g]));
  return { tick: dto.tick, markets, goods, flows: dto.flows };
}
```

- [ ] **Step 4: Run the backend test directory**

Run: `npx vitest run tests/backend/`
Expected: PASS (including the untouched mobilityProtocol tests)

- [ ] **Step 5: Commit**

```bash
git add src/backend/mobilityProtocol.ts src/backend/economyState.ts tests/backend/economyState.test.ts
git commit -m "feat(frontend): EconomyFlowDto on the wire + economyState.flows"
```

---

### Task 9: drawFlows.ts

**Files:**
- Create: `src/render/drawFlows.ts`
- Test: `tests/render/drawFlows.test.ts`

- [ ] **Step 1: Write the failing tests**

```typescript
// tests/render/drawFlows.test.ts
import { describe, expect, it } from 'vitest';
import { drawFlows, flowCurveControlPoint, flowStrokeWidth, goodColor } from '../../src/render/drawFlows';
import { GOOD_COLORS, GOOD_COLOR_FALLBACK } from '../../src/render/designTokens';
import type { EconomyFlowDto, MarketLocationDto } from '../../src/backend/mobilityProtocol';

describe('flowCurveControlPoint', () => {
  it('bulges perpendicular to the segment midpoint', () => {
    const c = flowCurveControlPoint({ x: 0, y: 0 }, { x: 100, y: 0 });
    expect(c.x).toBeCloseTo(50);
    expect(c.y).not.toBeCloseTo(0); // displaced off the segment
  });
  it('is antisymmetric in direction (A→B bulges opposite to B→A)', () => {
    const ab = flowCurveControlPoint({ x: 0, y: 0 }, { x: 100, y: 0 });
    const ba = flowCurveControlPoint({ x: 100, y: 0 }, { x: 0, y: 0 });
    expect(ab.y).toBeCloseTo(-ba.y);
  });
});

describe('flowStrokeWidth', () => {
  it('is zero for non-positive rates', () => {
    expect(flowStrokeWidth(0)).toBe(0);
    expect(flowStrokeWidth(-5)).toBe(0);
  });
  it('grows monotonically and clamps at 10', () => {
    expect(flowStrokeWidth(10)).toBeGreaterThan(flowStrokeWidth(1));
    expect(flowStrokeWidth(1e9)).toBe(10);
  });
});

describe('goodColor', () => {
  it('maps known goods and falls back for unknown ids', () => {
    expect(goodColor(1)).toBe(GOOD_COLORS[1]);
    expect(goodColor(999)).toBe(GOOD_COLOR_FALLBACK);
  });
});

describe('drawFlows', () => {
  const market = (marketId: number, tileX: number, tileY: number): MarketLocationDto =>
    ({ marketId, name: `m${marketId}`, tileX, tileY, wagePaidLastTick: 0 });
  const flow = (src: number, dst: number, rate: number): EconomyFlowDto =>
    ({ srcMarketId: src, dstMarketId: dst, goodId: 1, rate });
  const fakeCtx = () => {
    const ops: string[] = [];
    return {
      ops,
      ctx: new Proxy({}, {
        get: (_t, prop: string) => (prop === 'ops' ? ops : (...args: unknown[]) => { ops.push(String(prop)); void args; }),
        set: () => true,
      }) as unknown as CanvasRenderingContext2D,
    };
  };
  const project = (c: { x: number; y: number }) => ({ x: c.x * 18 + 9, y: c.y * 18 + 9 });
  const markets = new Map([[9003, market(9003, 16, 48)], [9004, market(9004, 208, 48)]]);

  it('draws one curve per positive flow with known endpoints and reports the count', () => {
    const { ctx } = fakeCtx();
    const drawn = drawFlows(ctx, project, markets, [flow(9003, 9004, 250)], { opacity: 1, detail: 'individual' });
    expect(drawn).toBe(1);
  });

  it('skips zero-rate flows and flows with unknown markets', () => {
    const { ctx } = fakeCtx();
    expect(drawFlows(ctx, project, markets, [flow(9003, 9004, 0)], { opacity: 1, detail: 'individual' })).toBe(0);
    expect(drawFlows(ctx, project, markets, [flow(1, 9004, 50)], { opacity: 1, detail: 'individual' })).toBe(0);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npx vitest run tests/render/drawFlows.test.ts`
Expected: FAIL — cannot resolve `../../src/render/drawFlows`

- [ ] **Step 3: Write the module**

```typescript
// src/render/drawFlows.ts
import type { EconomyFlowDto, MarketLocationDto } from '../backend/mobilityProtocol';
import { GOOD_COLORS, GOOD_COLOR_FALLBACK } from './designTokens';
import type { LayerBlend } from './layerBlend';

type Point = { x: number; y: number };

/** Quadratic-curve control point: segment midpoint displaced perpendicular,
 *  bulge proportional to length but capped so long edges stay tame. */
export function flowCurveControlPoint(a: Point, b: Point): Point {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const bulge = Math.min(40, len * 0.18);
  return { x: mx - (dy / len) * bulge, y: my + (dx / len) * bulge };
}

/** World-unit stroke width from an EWMA rate. 0 means "do not draw". */
export function flowStrokeWidth(rate: number): number {
  if (rate <= 0) return 0;
  return Math.min(10, 2 + 2 * Math.log10(1 + rate));
}

export function goodColor(goodId: number): string {
  return GOOD_COLORS[goodId] ?? GOOD_COLOR_FALLBACK;
}

/** Draw all flows. Returns the number of curves drawn (for diagnostics/smoke). */
export function drawFlows(
  ctx: CanvasRenderingContext2D,
  project: (coord: Point) => Point,
  markets: ReadonlyMap<number, MarketLocationDto>,
  flows: readonly EconomyFlowDto[],
  blend: LayerBlend,
): number {
  if (blend.opacity <= 0) return 0;
  let drawn = 0;
  ctx.save();
  ctx.lineCap = 'round';
  for (const flow of flows) {
    const width = flowStrokeWidth(flow.rate);
    if (width === 0) continue;
    const src = markets.get(flow.srcMarketId);
    const dst = markets.get(flow.dstMarketId);
    if (!src || !dst) continue;
    const a = project({ x: src.tileX, y: src.tileY });
    const b = project({ x: dst.tileX, y: dst.tileY });
    const c = flowCurveControlPoint(a, b);
    ctx.globalAlpha = 0.85 * blend.opacity;
    ctx.strokeStyle = goodColor(flow.goodId);
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.quadraticCurveTo(c.x, c.y, b.x, b.y);
    ctx.stroke();
    if (blend.detail === 'individual') drawChevron(ctx, a, c, b);
    drawn += 1;
  }
  ctx.restore();
  return drawn;
}

/** Direction marker at the curve's t=0.5 point, oriented along the tangent. */
function drawChevron(ctx: CanvasRenderingContext2D, a: Point, c: Point, b: Point): void {
  const mid = { x: 0.25 * a.x + 0.5 * c.x + 0.25 * b.x, y: 0.25 * a.y + 0.5 * c.y + 0.25 * b.y };
  const tangent = { x: b.x - a.x, y: b.y - a.y };
  const len = Math.hypot(tangent.x, tangent.y) || 1;
  const tx = tangent.x / len;
  const ty = tangent.y / len;
  const size = 5;
  ctx.beginPath();
  ctx.moveTo(mid.x - size * tx - size * 0.7 * ty, mid.y - size * ty + size * 0.7 * tx);
  ctx.lineTo(mid.x + size * tx, mid.y + size * ty);
  ctx.lineTo(mid.x - size * tx + size * 0.7 * ty, mid.y - size * ty - size * 0.7 * tx);
  ctx.lineWidth = 2;
  ctx.stroke();
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `npx vitest run tests/render/drawFlows.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/render/drawFlows.ts tests/render/drawFlows.test.ts
git commit -m "feat(render): drawFlows — good-colored rate-width curves with chevrons"
```

---

### Task 10: drawMarkets.ts (market nodes)

**Files:**
- Create: `src/render/drawMarkets.ts`
- Test: `tests/render/drawMarkets.test.ts`

- [ ] **Step 1: Write the failing tests**

```typescript
// tests/render/drawMarkets.test.ts
import { describe, expect, it } from 'vitest';
import {
  marketActivity,
  marketNodeRadius,
  priceTrend,
  satisfiedDemandFraction,
} from '../../src/render/drawMarkets';
import type { MarketGoodDto } from '../../src/backend/mobilityProtocol';

const good = (overrides: Partial<MarketGoodDto>): MarketGoodDto => ({
  marketId: 1, goodId: 1, lastSettlementPrice: 0, ewmaReferencePrice: 0,
  tradedQtyLastTick: 0, unmetDemandLastTick: 0, unsoldSupplyLastTick: 0,
  ...overrides,
});

describe('marketActivity', () => {
  it('sums traded qty across goods', () => {
    expect(marketActivity([good({ tradedQtyLastTick: 3 }), good({ tradedQtyLastTick: 7 })])).toBe(10);
  });
});

describe('marketNodeRadius', () => {
  it('has a floor at 6, grows monotonically, clamps at 14', () => {
    expect(marketNodeRadius(0)).toBe(6);
    expect(marketNodeRadius(100)).toBeGreaterThan(marketNodeRadius(10));
    expect(marketNodeRadius(1e12)).toBe(14);
  });
});

describe('satisfiedDemandFraction', () => {
  it('is traded/(traded+unmet)', () => {
    expect(satisfiedDemandFraction([good({ tradedQtyLastTick: 75, unmetDemandLastTick: 25 })])).toBeCloseTo(0.75);
  });
  it('is 1 when there was no demand at all (0/0)', () => {
    expect(satisfiedDemandFraction([good({})])).toBe(1);
    expect(satisfiedDemandFraction([])).toBe(1);
  });
});

describe('priceTrend', () => {
  it('is up/down when settlement deviates >1% from the EWMA reference', () => {
    expect(priceTrend([good({ lastSettlementPrice: 1100, ewmaReferencePrice: 1000 })])).toBe('up');
    expect(priceTrend([good({ lastSettlementPrice: 900, ewmaReferencePrice: 1000 })])).toBe('down');
  });
  it('is flat inside the deadband and for empty/zero-reference data', () => {
    expect(priceTrend([good({ lastSettlementPrice: 1005, ewmaReferencePrice: 1000 })])).toBe('flat');
    expect(priceTrend([good({ lastSettlementPrice: 50, ewmaReferencePrice: 0 })])).toBe('flat');
    expect(priceTrend([])).toBe('flat');
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npx vitest run tests/render/drawMarkets.test.ts`
Expected: FAIL — cannot resolve module

- [ ] **Step 3: Write the module**

```typescript
// src/render/drawMarkets.ts
import type { MarketGoodDto, MarketLocationDto } from '../backend/mobilityProtocol';
import { GROUND, MARKET_ORANGE } from './designTokens';
import { screenStableWorldSize } from './minimalGlyphScale';

type Point = { x: number; y: number };
export type PriceTrend = 'up' | 'down' | 'flat';

const TREND_DEADBAND = 0.01; // ±1% of the EWMA reference counts as flat
const PULSE_DURATION_MS = 600;

export function marketActivity(goods: readonly MarketGoodDto[]): number {
  return goods.reduce((sum, g) => sum + g.tradedQtyLastTick, 0);
}

/** World-unit node radius: floor 6, log growth, ceiling 14. */
export function marketNodeRadius(activity: number): number {
  if (activity <= 0) return 6;
  return Math.min(14, 6 + 2 * Math.log10(1 + activity));
}

export function satisfiedDemandFraction(goods: readonly MarketGoodDto[]): number {
  const traded = goods.reduce((s, g) => s + g.tradedQtyLastTick, 0);
  const unmet = goods.reduce((s, g) => s + g.unmetDemandLastTick, 0);
  if (traded + unmet === 0) return 1;
  return traded / (traded + unmet);
}

export function priceTrend(goods: readonly MarketGoodDto[]): PriceTrend {
  let score = 0;
  for (const g of goods) {
    if (g.ewmaReferencePrice === 0) continue;
    const deviation = (g.lastSettlementPrice - g.ewmaReferencePrice) / g.ewmaReferencePrice;
    if (deviation > TREND_DEADBAND) score += 1;
    else if (deviation < -TREND_DEADBAND) score -= 1;
  }
  if (score > 0) return 'up';
  if (score < 0) return 'down';
  return 'flat';
}

// Settlement-pulse bookkeeping: render-only wall-clock animation state.
// Keyed by marketId; reset only when traded qty changes (a settlement happened).
const pulseState = new Map<number, { lastTradedQty: number; pulseStartMs: number }>();

export function pulseAlpha(nowMs: number, pulseStartMs: number): number {
  const t = (nowMs - pulseStartMs) / PULSE_DURATION_MS;
  if (t < 0 || t >= 1) return 0;
  return 0.5 * (1 - t);
}

export function drawMarketNodes(
  ctx: CanvasRenderingContext2D,
  project: (coord: Point) => Point,
  cameraScale: number,
  markets: ReadonlyMap<number, MarketLocationDto>,
  goodsByMarket: (marketId: number) => readonly MarketGoodDto[],
  nowMs: number,
): void {
  for (const market of markets.values()) {
    const goods = goodsByMarket(market.marketId);
    const point = project({ x: market.tileX, y: market.tileY });
    const radius = screenStableWorldSize(
      marketNodeRadius(marketActivity(goods)),
      cameraScale,
      { minWorld: 5, maxWorld: 16 },
    );

    const tracked = pulseState.get(market.marketId);
    const traded = marketActivity(goods);
    if (!tracked || tracked.lastTradedQty !== traded) {
      pulseState.set(market.marketId, {
        lastTradedQty: traded,
        pulseStartMs: tracked && traded > 0 ? nowMs : Number.NEGATIVE_INFINITY,
      });
    }
    const pulse = pulseAlpha(nowMs, pulseState.get(market.marketId)?.pulseStartMs ?? Number.NEGATIVE_INFINITY);

    ctx.save();
    // pulse halo
    if (pulse > 0) {
      ctx.globalAlpha = pulse;
      ctx.strokeStyle = MARKET_ORANGE;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(point.x, point.y, radius * 1.6, 0, Math.PI * 2);
      ctx.stroke();
    }
    // node disc with white casing (reads on any ground)
    ctx.globalAlpha = 1;
    ctx.fillStyle = MARKET_ORANGE;
    ctx.strokeStyle = GROUND;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(point.x, point.y, radius, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    // satisfied-demand ring: arc fraction starting at 12 o'clock
    const fraction = satisfiedDemandFraction(goods);
    if (fraction < 1) {
      ctx.strokeStyle = MARKET_ORANGE;
      ctx.lineWidth = 2.4;
      ctx.beginPath();
      ctx.arc(point.x, point.y, radius + 3, -Math.PI / 2, -Math.PI / 2 + fraction * Math.PI * 2);
      ctx.stroke();
    }
    // price-trend arrow above the node
    const trend = priceTrend(goods);
    if (trend !== 'flat') {
      const dir = trend === 'up' ? -1 : 1;
      ctx.fillStyle = GROUND;
      ctx.beginPath();
      ctx.moveTo(point.x, point.y + dir * (radius * 0.45) - dir * 2);
      ctx.lineTo(point.x - 3, point.y + dir * (radius * 0.45) + dir * 3);
      ctx.lineTo(point.x + 3, point.y + dir * (radius * 0.45) + dir * 3);
      ctx.closePath();
      ctx.fill();
    }
    ctx.restore();
  }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `npx vitest run tests/render/drawMarkets.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/render/drawMarkets.ts tests/render/drawMarkets.test.ts
git commit -m "feat(render): market nodes — activity radius, demand ring, trend, pulse"
```

---

### Task 11: drawAgents.ts + `BackendPedestrian.stateType`

**Files:**
- Modify: `src/render/backendMobilityDrawables.ts` (add `stateType` to `BackendPedestrian`)
- Create: `src/render/drawAgents.ts`
- Test: `tests/render/drawAgents.test.ts`; extend `tests/render/backendMobilityDrawables.test.ts`

- [ ] **Step 1: Write the failing tests**

```typescript
// tests/render/drawAgents.test.ts
import { describe, expect, it } from 'vitest';
import { agentGlyph } from '../../src/render/drawAgents';
import { AGENT_INK, TRADER_RED } from '../../src/render/designTokens';

describe('agentGlyph', () => {
  it('walking and in_vehicle render as filled ink dots', () => {
    expect(agentGlyph('walking', 'pedestrian')).toEqual({ shape: 'dot', color: AGENT_INK, radiusScale: 1 });
    expect(agentGlyph('in_vehicle', 'pedestrian')).toEqual({ shape: 'dot', color: AGENT_INK, radiusScale: 1 });
  });
  it('at_activity and waiting_at_stop render as rings', () => {
    expect(agentGlyph('at_activity', 'pedestrian').shape).toBe('ring');
    expect(agentGlyph('waiting_at_stop', 'pedestrian').shape).toBe('ring');
  });
  it('traders are larger red dots regardless of state', () => {
    expect(agentGlyph('at_activity', 'trader')).toEqual({ shape: 'dot', color: TRADER_RED, radiusScale: 1.5 });
  });
});
```

And in `tests/render/backendMobilityDrawables.test.ts`, extend an existing pedestrian-producing test with:

```typescript
expect(pedestrians[0].stateType).toBe('walking'); // matching the fixture's agent state
```

- [ ] **Step 2: Run to verify they fail**

Run: `npx vitest run tests/render/drawAgents.test.ts tests/render/backendMobilityDrawables.test.ts`
Expected: FAIL — module missing / `stateType` undefined

- [ ] **Step 3: Implement**

In `backendMobilityDrawables.ts`: add `stateType: AgentMobilityDto['state']['type'];` to `BackendPedestrian` and populate it in `pedestriansFromMobilityState` with `stateType: agent.state.type,` (next to the existing `kind:` mapping at ~line 97).

Create `src/render/drawAgents.ts` — move `drawPedestrian` and `drawCar` (and their private helper `vehicleVectorColor`) verbatim out of `minimalMapRenderer.ts:511-569`, then restyle the pedestrian path:

```typescript
// src/render/drawAgents.ts
import type { AgentMobilityDto } from '../backend/mobilityProtocol';
import type { BackendCar, BackendPedestrian } from './backendMobilityDrawables';
import {
  AGENT_INK,
  SELECTION_HALO_AGENT,
  SELECTION_HALO_VEHICLE,
  TRADER_RED,
  VEHICLE_COLORS,
} from './designTokens';
import type { LayerBlend } from './layerBlend';
import { drawCapsule } from './canvasPrimitives';
import { carRenderStyle, carVisualWorldPoint, pedestrianRenderStyle } from './entityRenderStyle';
import { stableHash as hash } from './gridMath';

export type AgentGlyph = { shape: 'dot' | 'ring'; color: string; radiusScale: number };

export function agentGlyph(
  stateType: AgentMobilityDto['state']['type'],
  kind: BackendPedestrian['kind'],
): AgentGlyph {
  if (kind === 'trader') return { shape: 'dot', color: TRADER_RED, radiusScale: 1.5 };
  if (stateType === 'at_activity' || stateType === 'waiting_at_stop') {
    return { shape: 'ring', color: AGENT_INK, radiusScale: 1 };
  }
  return { shape: 'dot', color: AGENT_INK, radiusScale: 1 };
}

// drawPedestrian(state, pedestrian, selected, blend): moved from minimalMapRenderer,
// with the body's fill section replaced by:
//
//   const glyph = blend.detail === 'aggregate'
//     ? { ...agentGlyph(pedestrian.stateType, pedestrian.kind), shape: 'dot' as const }
//     : agentGlyph(pedestrian.stateType, pedestrian.kind);
//   ctx.globalAlpha *= blend.opacity;
//   if (glyph.shape === 'ring') {
//     ctx.strokeStyle = glyph.color;
//     ctx.lineWidth = Math.max(1.2, style.radius * 0.45);
//     ctx.beginPath();
//     ctx.arc(0, 0, style.radius * glyph.radiusScale, 0, Math.PI * 2);
//     ctx.stroke();
//   } else {
//     ctx.fillStyle = glyph.color;
//     ctx.beginPath();
//     ctx.arc(0, 0, style.radius * glyph.radiusScale, 0, Math.PI * 2);
//     ctx.fill();
//   }
//
// The selection-halo block stays as-is but uses SELECTION_HALO_AGENT.
// drawCar moves verbatim; its hardcoded '#166c83' becomes SELECTION_HALO_VEHICLE
// and VEHICLE_COLORS imports from designTokens.
```

(Move the real function bodies — the comment above specifies exactly which section changes. `drawPedestrian`/`drawCar` keep taking the renderer state object; export both. The old `AGENT_COLOR`/`TRADER_COLOR` constants in `minimalMapRenderer.ts` are deleted in Task 13.)

- [ ] **Step 4: Run to verify they pass**

Run: `npx vitest run tests/render/drawAgents.test.ts tests/render/backendMobilityDrawables.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/render/drawAgents.ts src/render/backendMobilityDrawables.ts tests/render/drawAgents.test.ts tests/render/backendMobilityDrawables.test.ts
git commit -m "feat(render): drawAgents — state glyphs (dot/ring), trader red, blend-aware"
```

---

### Task 12: drawNetwork.ts extraction + palette swap

**Files:**
- Create: `src/render/drawNetwork.ts`
- Modify: `src/render/minimalMapRenderer.ts` (delete moved code + old constants)
- Modify: `tests/render/minimalMapRenderer.test.ts` (palette-pinned assertions)

- [ ] **Step 1: Move the L0 drawers**

Move verbatim from `minimalMapRenderer.ts` into `src/render/drawNetwork.ts` (exporting what the orchestrator needs):
`drawGrassBaseLayer`, `drawTerrainOverlayLayer`, `terrainOverlayStyle`, `drawRiverSurfaceLayer`, `riverSurfaceStyle`, `drawRoad`, `drawRoads`, `drawRoadRuns`, `drawRoadBand`, `appendGrouped`, `mergedRuns`, `drawRail`, `drawRailPath`, `drawPolyline`, `drawRailStation`, `drawDetail`, `drawBuilding`, `buildingJitter`, `drawTree`, `appendTileFillBatch`, `drawTileFillBatches`, `drawTileFillBatch`, `drawMaskLine`, `drawRoadPass`, `buildingVectorColor`, `drawEdgeConnections`, `drawFadingEdgeTile` (lines 200-509 + 579-606 + 663-669). They keep taking `MinimalMapRendererState` (import the type from `./minimalMapRenderer`).

- [ ] **Step 2: Apply the schematic restyle inside drawNetwork.ts**

All colors now import from `./designTokens`:
- `MAP_GRASS` → `GROUND`; `MAP_WATER` → `WATER`; `MAP_RIVERBANK` → `RIVERBANK`; `MAP_PARK` → `PARK`; `MAP_PLAZA` → `PLAZA`; `TREE_COLOR` → `TREE`; `DETAIL_COLOR` → `DETAIL`; building colors → the four `BUILDING_*` tokens.
- Roads collapse from five bands to two. Replace the `bands` array in `drawRoads` and the band calls in `drawRoad` with:

```typescript
const bands = [
  { color: ROAD_INK, width: screenStableWorldSize(14, state.camera.scale, { minWorld: 14, maxWorld: 24 }) },
  { color: ROAD_CENTER_DASH, width: screenStableWorldSize(2, state.camera.scale, { minWorld: 1.6, maxWorld: 3.4 }) },
];
```

(Same two bands in `drawRoad` for edge-fade tiles. The center line stays a solid contrasting line — it is drawn in `ROAD_CENTER_DASH` = ground color, which gives the schematic "cut-out" reading without per-run dash plumbing.)
- Buildings: in `drawBuilding`, drop the roof + window overlay block (the `if (building.sheet === 'oldhouses' ...)` branch) entirely; raise the corner radius to `2.6`.

- [ ] **Step 3: Update the palette-pinned test**

`tests/render/minimalMapRenderer.test.ts:150` pins `'#91c86f'`. Update it to import the token:

```typescript
import { GROUND } from '../../src/render/designTokens';
// ...
const grassFills = ctx.operations.filter(
  (operation): operation is FillRectOperation =>
    operation.type === 'fillRect' && operation.fillStyle === GROUND,
);
```

Run the file; update any further assertions the run flags (road band counts drop from 5 to 2 per run; building roof fills disappear). Every updated assertion must reference tokens, never hex literals.

- [ ] **Step 4: Run the render test directory**

Run: `npx vitest run tests/render/`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/render/drawNetwork.ts src/render/minimalMapRenderer.ts tests/render/minimalMapRenderer.test.ts
git commit -m "feat(render): drawNetwork extraction + schematic palette (paper ground, 2-band roads, flat buildings)"
```

---

### Task 13: Orchestration — renderer state, draw sequence, main.ts plumbing, diagnostics

**Files:**
- Modify: `src/render/minimalMapRenderer.ts`
- Modify: `src/main.ts` (render call ~line 276; diagnostics install ~line 485)
- Modify: `src/app/runtimeDiagnostics.ts` (~line 195)
- Test: `tests/render/minimalMapRenderer.test.ts`, `tests/render/economyMarkets.test.ts`

- [ ] **Step 1: Extend `MinimalMapRendererState`**

```typescript
import type { EconomyFlowDto, MarketGoodDto } from '../backend/mobilityProtocol';
// in MinimalMapRendererState:
  markets?: readonly MarketLocationDto[];
  goods?: readonly MarketGoodDto[];
  flows?: readonly EconomyFlowDto[];
```

- [ ] **Step 2: Rewrite `drawScene`'s layer sequence**

Replace the old `drawEconomyMarkets`/`drawMarketGlyph`/`MARKET_COLOR`/`MARKET_GLYPH_WORLD_SIZE` (delete them and the now-unused color constants 105-130 — they all live in tokens now) with the orchestrated sequence. Keep `visibleMarketGlyphs` exported (the economyMarkets test and culling use it):

```typescript
import { layerBlend } from './layerBlend';
import { drawFlows } from './drawFlows';
import { drawMarketNodes } from './drawMarkets';
import { drawCar, drawPedestrian } from './drawAgents';
import * as network from './drawNetwork';

// inside drawScene, after culling/sorting (drawables logic unchanged):
const agentsBlend = layerBlend('agents', state.camera.scale);
const flowsBlend = layerBlend('flows', state.camera.scale);

network.drawGrassBaseLayer(state);
network.drawTerrainOverlayLayer(state, visibleTerrainTiles);
network.drawRiverSurfaceLayer(state, visibleTerrainTiles);
network.drawRoads(state, [...state.roads.values()].filter((road) => isCoordVisible(road.coord, visibleGrid)));
for (const path of state.railPaths) network.drawRailPath(state, path);
network.drawEdgeConnections(state, visibleGrid);
for (const station of state.railStations) if (isCoordVisible(station.coord, visibleGrid)) network.drawRailStation(state, station);
for (const detail of state.details) if (isCoordVisible(detail.coord, visibleGrid)) network.drawDetail(state, detail);
for (const building of state.buildings) if (isCoordVisible(building.coord, visibleGrid)) network.drawBuilding(state, building);
for (const coord of state.trees) if (isCoordVisible(coord, visibleGrid)) network.drawTree(state, coord);

const marketsById = new Map((state.markets ?? []).map((m) => [m.marketId, m]));
lastFlowsDrawn = drawFlows(state.ctx, (c) => iso(state, c), marketsById, state.flows ?? [], flowsBlend);

for (const item of carDrawables) drawCar(state, item.car, item.vehicleId === state.selectedVehicleId);
for (const item of pedestrianDrawables) drawPedestrian(state, item.pedestrian, item.agentId === state.selectedAgentId, agentsBlend);

const visibleMarkets = new Map(
  visibleMarketGlyphs(state.markets, visibleGrid).map((m) => [m.marketId, m]),
);
drawMarketNodes(
  state.ctx,
  (c) => iso(state, c),
  state.camera.scale,
  visibleMarkets,
  (marketId) => (state.goods ?? []).filter((g) => g.marketId === marketId),
  state.now(),
);
```

Add at module scope and export for diagnostics:

```typescript
let lastFlowsDrawn = 0;
export function flowsDrawnLastFrame(): number {
  return lastFlowsDrawn;
}
```

- [ ] **Step 3: Plumb data + diagnostic in main.ts and runtimeDiagnostics.ts**

In the `renderMinimalMap({...})` call (main.ts:276): add

```typescript
goods: [...economyState.goods.values()],
flows: economyState.flows,
```

In `runtimeDiagnostics.ts`, mirror `economyMarketCount` (line 195): add `economyFlowCount: options.getEconomyFlowCount(),` plus the option type field `getEconomyFlowCount: () => number;`. In main.ts's `installRuntimeDiagnostics(...)` options add:

```typescript
getEconomyFlowCount: () => flowsDrawnLastFrame(),
```

(import `flowsDrawnLastFrame` from the renderer).

Spec §4 also requires a frame-time counter in the diagnostics. In main.ts's `frame(now)` callback, the delta is already computed (`dt`); keep the last raw frame duration in a module-level `let lastFrameMs = 0;` updated each frame (`lastFrameMs = now - previousTime;` before `previousTime` is reassigned), and expose it the same way: option `getFrameTimeMs: () => lastFrameMs` → diagnostics field `frameTimeMs: options.getFrameTimeMs(),`.

- [ ] **Step 4: Run full frontend checks**

Run: `npm run typecheck && npx vitest run`
Expected: PASS. Fix anything `tests/render/economyMarkets.test.ts` flags (it exercises `visibleMarketGlyphs`, which is unchanged; only its draw-path assertions may need the node call instead of the old glyph).

- [ ] **Step 5: Commit**

```bash
git add src/render/minimalMapRenderer.ts src/main.ts src/app/runtimeDiagnostics.ts tests/render/
git commit -m "feat(render): semantic-zoom orchestration — flows under agents, market nodes on top"
```

---

### Task 14: Browser smoke (acceptance gate)

**Files:**
- Create: `scripts/smoke-schematic.mjs` (modeled on `scripts/smoke-economy-markets.mjs` — same stack-launch scaffolding, ports 8083/5177)

- [ ] **Step 1: Write the smoke script**

Copy `scripts/smoke-economy-markets.mjs` to `scripts/smoke-schematic.mjs`, change ports (backend 8083, frontend 5177), and replace the assertion phase with:

```javascript
// 1) WIRE: within 120s, at least one economySnapshot frame carries flows
//    (reuse the existing ws frame decoding via fromBinary(ServerMessageSchema, ...)):
//    record `sawFlows = msg.body.value.flows.length > 0`.
// 2) ECONOMY VIEW: zoom far out (mouse wheel until camera.scale <= 0.5 — drive it
//    like smoke-economy-markets does), wait 3s, then assert via the page's
//    runtime diagnostics: economyFlowCount >= 1 and economyMarketCount >= 1.
// 3) CITY VIEW: zoom in over the corridor (scale >= 1.2), wait 3s, assert the
//    existing agent-count diagnostic reports > 0 agents.
// 4) Save screenshots of both zoom levels to smoke-schematic-economy.png /
//    smoke-schematic-city.png for human review.
// Fail (process.exit(1)) with a clear message if any assertion fails.
```

Implement these as real code following the existing script's idioms (`page.on('websocket')`, `page.mouse.wheel`, polling `await page.evaluate(() => window.__abutownDiagnostics?.economyFlowCount)` — check how smoke-economy-markets reads `runtimeDiagnostics.economyMarketCount` and mirror it exactly).

- [ ] **Step 2: Run it against the dev stack**

Precondition: no other cargo/sim-server/vite running (`pgrep -f 'cargo|sim-server|vite'`).
Run: `node scripts/smoke-schematic.mjs`
Expected: exits 0; both screenshots show (economy view) paper ground + orange nodes + at least one colored flow curve, (city view) the road with agent dots.

**Do not claim the feature complete while this fails.** If flows never arrive on the wire, debug the backend cadence (macro_flow_interval_ticks) before touching the renderer.

- [ ] **Step 3: Commit**

```bash
git add scripts/smoke-schematic.mjs
git commit -m "test(smoke): schematic renderer browser smoke — wire flows + both zoom narratives"
```

---

### Task 15: Full CI gate + PR

- [ ] **Step 1: Frontend gate**

Run: `npm run typecheck && npx vitest run && npm run build`
Expected: all green (typecheck covers src+tests+scripts via tsconfig.typecheck.json).

- [ ] **Step 2: Rust gate (serialized, scoped)**

```bash
pgrep -f cargo   # must be empty first
scripts/cargo-serial.sh fmt --manifest-path backend/Cargo.toml --all -- --check
scripts/cargo-serial.sh clippy --manifest-path backend/Cargo.toml -p sim-core -p sim-server -p abutown-protocol -- -D warnings
scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-core -p sim-server -p abutown-protocol
```

Expected: all green. If fmt is green locally but red in CI later: `rustup update stable`, re-run fmt (known toolchain-skew failure mode).

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin graphics/schematic-map-renderer
gh pr create --title "Schematic map renderer: Mini-Metro-school restyle, semantic zoom, economy flows on the wire" --body "$(cat <<'EOF'
Implements docs/superpowers/specs/2026-06-10-schematic-map-renderer-design.md.

- designTokens + layerBlend (pure semantic-zoom cross-fade)
- L0-L3 layer drawers extracted from minimalMapRenderer
- Additive wire field EconomySnapshot.flows=6 (in-memory FlowRateEwma, never persisted — no DB migration)
- Browser smoke scripts/smoke-schematic.mjs (wire + both zoom narratives) passing

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Wait for GREEN, then merge**

```bash
gh pr checks --watch --fail-fast
```

Expected: ALL checks pass (not merely "no failures yet"). Only then:

```bash
gh pr merge --squash --delete-branch
```

- [ ] **Step 5: Clean up**

Remove the worktree after merge (`git worktree remove .claude/worktrees/schematic-renderer`), verify `origin/main` contains the merge, and confirm no stray cargo/vite/sim-server processes remain.

---

## Self-review notes (kept for the executor)

- Spec §1 vocabulary → Tasks 1, 10, 11, 12. Spec §2 wire → Tasks 4-8. Spec §3 components → Tasks 1-3, 9-13. Spec §4 gates → Tasks 14-15.
- The settlement pulse uses the wall clock (`state.now()` / `nowMs`) — render-only, per spec §3.
- `RealizedFlow.qty` (Task 4) is the one backend change the spec implied but did not spell out: the realized-flow telemetry lacked quantities; the wire field needs them.
- Flow curves anchor at market tile coords through the real projection (`iso`/`mapProject`) — the Phase-7a lesson is covered by both the unit test (Task 9) and the smoke (Task 14).
