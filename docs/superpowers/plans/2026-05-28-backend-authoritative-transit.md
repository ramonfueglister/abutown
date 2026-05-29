# Backend-Authoritative Transit Vehicles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Archived/closed in the 2026-05-29 documentation cleanup. This backend-transit slice was completed at the time, then superseded by Traffic Simulation Hardening, which removed runtime tram movement again.

**Goal:** Replace frontend-only moving trains with backend-authoritative tram vehicles seeded from the canonical Base World Bundle.

**Architecture:** Extend bundle validation and mobility seeding so `spawns.tram_lines` creates deterministic `VehicleKind::Tram` records on routing transit lines. Then remove local train offset state from the browser runtime and render backend tram vehicles from the existing mobility stream.

**Tech Stack:** Rust `sim-core` + `sim-server`, Bevy ECS resources, existing protobuf `VehicleMobility`, TypeScript/Vite frontend, Vitest, Playwright render smoke.

---

## File Structure

Create:

- `tests/render/backendTransitDrawables.test.ts` - frontend projection tests for backend tram vehicles.

Modify:

- `backend/crates/sim-core/src/base_world.rs` - validate spawn references against authored transport path ids.
- `backend/crates/sim-core/src/mobility/seed.rs` - add `from_base_world_bundle` and deterministic tram spawning.
- `backend/crates/sim-core/src/mobility/mod.rs` - add unit tests for bundle tram seeding.
- `backend/crates/sim-server/src/runtime.rs` - use bundle-aware seeding for startup/hydration and add runtime tram assertion.
- `src/render/backendMobilityDrawables.ts` - split car and tram projection from backend vehicle state.
- `src/render/minimalMapRenderer.ts` - draw backend tram drawables, not local `trains`.
- `src/app/runtimeDiagnostics.ts` - report backend tram entries.
- `src/main.ts` - remove local train construction and offset mutation.
- `tests/app/noProductionFallbacks.test.ts` - guard against local train movement paths.
- `tests/e2e/render-smoke.spec.ts` - assert backend tram diagnostics and movement.
- `docs/superpowers/plans/2026-05-28-backend-authoritative-transit.md` - mark progress.

## Progress

- [x] Task 1: Add failing backend bundle transit tests
- [x] Task 2: Implement bundle-aware mobility seeding
- [x] Task 3: Add failing frontend tram projection tests and guards
- [x] Task 4: Render backend trams and remove local train state
- [x] Task 5: Run full verification and commit

## Task 1: Add Failing Backend Bundle Transit Tests

**Files:**

- Modify: `backend/crates/sim-core/src/mobility/mod.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Write failing sim-core tests**

Add tests near the existing `from_network_*` tests:

```rust
#[test]
fn from_base_world_bundle_spawns_declared_trams() {
    let bundle = crate::base_world::BaseWorldBundle::load_from_dir(
        workspace_root().join("data/worlds/zurich-river-city-v1"),
    )
    .expect("base world fixture loads");

    let expected_trams: usize = bundle.spawns.tram_lines.iter().map(|line| line.trams as usize).sum();
    assert!(expected_trams > 0, "fixture declares backend tram spawns");

    let (world, _) = seed::from_base_world_bundle(&bundle).expect("bundle seeding succeeds");
    let trams = api::vehicles(&world)
        .into_iter()
        .filter(|vehicle| vehicle.kind == VehicleKind::Tram)
        .collect::<Vec<_>>();

    assert_eq!(trams.len(), expected_trams);
    assert!(trams.iter().all(|vehicle| vehicle.id.0.starts_with("vehicle:tram:")));
    assert!(trams.iter().all(|vehicle| vehicle.route_id.starts_with("tram:")));
}

#[test]
fn from_base_world_bundle_rejects_missing_tram_rail_path() {
    let mut bundle = crate::base_world::BaseWorldBundle::load_from_dir(
        workspace_root().join("data/worlds/zurich-river-city-v1"),
    )
    .expect("base world fixture loads");
    bundle.spawns.tram_lines[0].rail_path_ids = vec!["rail:missing".to_string()];

    let err = seed::from_base_world_bundle(&bundle).expect_err("missing rail path is fatal");
    assert!(err.to_string().contains("rail:missing"));
}
```

Add a local `workspace_root()` helper if the test module does not already have one:

```rust
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("sim-core crate lives under backend/crates/sim-core")
        .to_path_buf()
}
```

- [x] **Step 2: Verify RED**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-core from_base_world_bundle -- --nocapture
```

Expected: FAIL to compile because `seed::from_base_world_bundle` does not exist.

- [x] **Step 3: Write failing sim-server runtime test**

Add near `runtime_materializes_base_world_instead_of_demo_chunks`:

```rust
#[test]
fn runtime_seeds_backend_trams_from_base_world() {
    let fixture_root = workspace_root().join("data/worlds/zurich-river-city-v1");
    let runtime = SimulationRuntime::new_from_base_world_dir(&fixture_root)
        .expect("base world fixture must load");
    let snapshot = runtime.mobility_snapshot();

    let trams = snapshot
        .vehicles
        .iter()
        .filter(|vehicle| vehicle.kind == abutown_protocol::VehicleKindDto::Tram)
        .collect::<Vec<_>>();

    assert_eq!(trams.len(), 4);
    assert!(trams.iter().all(|vehicle| vehicle.id.0.starts_with("vehicle:tram:")));
    assert!(trams.iter().all(|vehicle| vehicle.sprite_key.starts_with("tram:")));
}
```

- [x] **Step 4: Verify RED**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_seeds_backend_trams_from_base_world -- --nocapture
```

Expected: FAIL because the runtime currently exposes zero backend tram vehicles from the base-world bundle.

## Task 2: Implement Bundle-Aware Mobility Seeding

**Files:**

- Modify: `backend/crates/sim-core/src/base_world.rs`
- Modify: `backend/crates/sim-core/src/mobility/seed.rs`
- Modify: `backend/crates/sim-server/src/runtime.rs`

- [x] **Step 1: Add seed error type and bundle seeding function**

In `backend/crates/sim-core/src/mobility/seed.rs`, add:

```rust
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("base world tram line {line_id} references missing rail path {rail_path_id}")]
    MissingRailPath { line_id: String, rail_path_id: String },
    #[error("base world tram line {line_id} has no rail paths")]
    EmptyTramLine { line_id: String },
}

pub fn from_base_world_bundle(
    bundle: &crate::base_world::BaseWorldBundle,
) -> Result<(World, Schedule), SeedError> {
    let network = bundle.to_city_network();
    let (mut world, schedule) = empty_world_and_schedule_for_network(&network);

    seed_pedestrians_from_bundle(&mut world, bundle);
    seed_cars_from_bundle(&mut world, bundle);
    seed_trams_from_bundle(&mut world, bundle)?;

    Ok((world, schedule))
}
```

- [x] **Step 2: Add bundle pedestrian and car helpers**

Still in `seed.rs`, add helpers that preserve the existing deterministic ids:

```rust
fn seed_pedestrians_from_bundle(world: &mut World, bundle: &crate::base_world::BaseWorldBundle) {
    let mut agent_index = 0u32;
    for group in &bundle.spawns.pedestrian_groups {
        let Some(corridor_index) = bundle
            .transport
            .pedestrian_corridors
            .iter()
            .position(|path| path.id == group.corridor_id)
        else {
            continue;
        };
        for n in 0..group.agents_per_corridor {
            let agent_id = AgentId(format!("agent:walk:{agent_index}"));
            agent_index += 1;
            let link_id = format!("link:walk:corridor:{corridor_index}");
            let progress = if group.agents_per_corridor > 0 {
                (n as f32) / (group.agents_per_corridor as f32)
            } else {
                0.0
            };
            api::spawn_agent_from_record(
                world,
                AgentRecord::new(
                    agent_id,
                    AgentMobilityState::Walking { link_id, progress },
                    vec![PlanStage::Activity {
                        activity_id: format!("activity:wander:{corridor_index}"),
                    }],
                    0.05,
                ),
            );
        }
    }
}

fn seed_cars_from_bundle(world: &mut World, bundle: &crate::base_world::BaseWorldBundle) {
    let mut driver_index = 0u32;
    for group in &bundle.spawns.car_groups {
        let Some(arterial_index) = bundle
            .transport
            .arterial_paths
            .iter()
            .position(|path| path.id == group.arterial_id)
        else {
            continue;
        };
        for n in 0..group.cars_per_arterial {
            let vehicle_id = VehicleId(format!("vehicle:car:{arterial_index}:{n}"));
            let route_id = format!("route:arterial:{arterial_index}");
            let driver_id = AgentId(format!("agent:driver:{driver_index}"));
            driver_index += 1;
            api::spawn_vehicle_from_record(
                world,
                VehicleRecord {
                    id: vehicle_id.clone(),
                    kind: VehicleKind::Car,
                    route_id,
                    link_index: 0,
                    progress: if group.cars_per_arterial > 0 {
                        (n as f32) / (group.cars_per_arterial as f32)
                    } else {
                        0.0
                    },
                    speed_per_tick: 0.02,
                    capacity: 1,
                    occupants: vec![driver_id.clone()],
                    dwell_ticks_remaining: 0,
                },
            );
            api::spawn_agent_from_record(
                world,
                AgentRecord::new(
                    driver_id,
                    AgentMobilityState::InVehicle {
                        vehicle_id,
                        seat_index: 0,
                    },
                    vec![PlanStage::Activity {
                        activity_id: format!("activity:drive:{arterial_index}"),
                    }],
                    0.05,
                ),
            );
        }
    }
}
```

- [x] **Step 3: Add tram helper**

Add:

```rust
fn seed_trams_from_bundle(
    world: &mut World,
    bundle: &crate::base_world::BaseWorldBundle,
) -> Result<(), SeedError> {
    for (line_index, line) in bundle.spawns.tram_lines.iter().enumerate() {
        if line.rail_path_ids.is_empty() {
            return Err(SeedError::EmptyTramLine {
                line_id: line.id.clone(),
            });
        }
        let rail_path_id = &line.rail_path_ids[0];
        let Some(rail_index) = bundle
            .transport
            .rail_paths
            .iter()
            .position(|path| &path.id == rail_path_id)
        else {
            return Err(SeedError::MissingRailPath {
                line_id: line.id.clone(),
                rail_path_id: rail_path_id.clone(),
            });
        };
        for n in 0..line.trams {
            api::spawn_vehicle_from_record(
                world,
                VehicleRecord {
                    id: VehicleId(format!("vehicle:tram:{line_index}:{n}")),
                    kind: VehicleKind::Tram,
                    route_id: format!("tram:{rail_index}"),
                    link_index: 0,
                    progress: if line.trams > 0 {
                        (n as f32) / (line.trams as f32)
                    } else {
                        0.0
                    },
                    speed_per_tick: 0.03,
                    capacity: 80,
                    occupants: Vec::new(),
                    dwell_ticks_remaining: 0,
                },
            );
        }
    }
    Ok(())
}
```

- [x] **Step 4: Replace runtime seeding path**

In `backend/crates/sim-server/src/runtime.rs`, change `initial_mobility_snapshot_for_base_world` to:

```rust
fn initial_mobility_snapshot_for_base_world(bundle: &BaseWorldBundle) -> anyhow::Result<MobilityPersistSnapshot> {
    let (seeded_world, _) = sim_core::mobility::seed::from_base_world_bundle(bundle)?;
    Ok(extract_from_world(&seeded_world))
}
```

Update callers in `new_with_event_store_and_base_world` and `hydrate_from_stores` to propagate the error with `?` or map it into `HydrationError`.

- [x] **Step 5: Verify GREEN**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-core from_base_world_bundle -- --nocapture
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-server runtime_seeds_backend_trams_from_base_world -- --nocapture
```

Expected: both exit 0.

## Task 3: Add Failing Frontend Tram Projection Tests And Guards

**Files:**

- Create: `tests/render/backendTransitDrawables.test.ts`
- Modify: `tests/app/noProductionFallbacks.test.ts`

- [x] **Step 1: Add projection tests**

Create `tests/render/backendTransitDrawables.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { createMobilityOverlayState, applyMobilitySnapshot } from '../../src/backend/mobilityState';
import { carsFromMobilityState, tramsFromMobilityState } from '../../src/render/backendMobilityDrawables';

const carSprite = { sheet: 'city-bus', role: 'vehicle.bus' };
const tramSprite = { sheet: 'metro-line', role: 'vehicle.tram' };

describe('backend transit drawables', () => {
  it('projects backend trams separately from backend cars', () => {
    const state = applyMobilitySnapshot(createMobilityOverlayState(), {
      protocol_version: 1,
      world_id: 'zurich-river-city-v1',
      tick: 5,
      agents: [],
      stops: [],
      vehicles: [
        {
          id: 'vehicle:car:0:0',
          kind: 'car',
          route_id: 'route:arterial:0',
          link_index: 0,
          progress: 0.5,
          capacity: 1,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 20, y: 30 },
          direction: 's',
          sprite_key: 'vehicle:0',
        },
        {
          id: 'vehicle:tram:0:0',
          kind: 'tram',
          route_id: 'tram:0',
          link_index: 0,
          progress: 0.25,
          capacity: 80,
          occupants: [],
          dwell_ticks_remaining: 0,
          world_coord: { x: 150, y: 64 },
          direction: 'n',
          sprite_key: 'tram:0',
        },
      ],
    }, 1000);

    const cars = carsFromMobilityState(state, [carSprite], 1000, 100);
    const trams = tramsFromMobilityState(state, [tramSprite], 1000, 100);

    expect(cars).toHaveLength(1);
    expect(cars[0].id).toBe('vehicle:car:0:0');
    expect(trams).toHaveLength(1);
    expect(trams[0]).toMatchObject({
      id: 'vehicle:tram:0:0',
      path: [{ x: 150, y: 64 }, { x: 150, y: 63 }],
      sprite: tramSprite,
      direction: 'n',
    });
  });
});
```

- [x] **Step 2: Add local train fallback guard**

Extend `tests/app/noProductionFallbacks.test.ts` with forbidden runtime patterns in `src/main.ts`:

```ts
"buildNorthboundTrainPath(",
"trainWrappedOffset(",
"for (const train of trains)",
```

- [x] **Step 3: Verify RED**

Run:

```bash
npx vitest run tests/render/backendTransitDrawables.test.ts tests/app/noProductionFallbacks.test.ts --passWithNoTests
```

Expected: FAIL because `tramsFromMobilityState` does not exist and `src/main.ts` still advances local trains.

## Task 4: Render Backend Trams And Remove Local Train State

**Files:**

- Modify: `src/render/backendMobilityDrawables.ts`
- Modify: `src/render/minimalMapRenderer.ts`
- Modify: `src/app/runtimeDiagnostics.ts`
- Modify: `src/main.ts`
- Modify: `tests/e2e/render-smoke.spec.ts`

- [x] **Step 1: Add tram drawable projection**

In `backendMobilityDrawables.ts`, add a tram type and projection:

```ts
export type BackendTram = {
  id: string;
  path: Coord[];
  offset: number;
  speed: number;
  sprite: VehicleSpriteLike;
  direction: DirectionDto;
};

export function tramsFromMobilityState(
  state: MobilityOverlayState,
  sprites: readonly VehicleSpriteLike[],
  now: number,
  tickPeriodMs: number,
): BackendTram[] {
  if (sprites.length === 0) return [];
  return interpolatedVehicles(state, now, tickPeriodMs)
    .filter((vehicle) => vehicle.kind === 'tram')
    .sort((a, b) => a.id.localeCompare(b.id))
    .map((vehicle) => ({
      id: vehicle.id,
      path: syntheticPath(vehicle.world_coord, vehicle.direction),
      offset: 0,
      speed: 0,
      sprite: sprites[spriteIndexFromKey(vehicle.sprite_key, sprites.length)],
      direction: vehicle.direction,
    }));
}
```

- [x] **Step 2: Draw backend trams in minimal renderer**

In `minimalMapRenderer.ts`:

- import `tramsFromMobilityState` and `BackendTram`
- remove `MinimalMapRendererTrain`
- replace `state.trains` with `tramSprites`
- build `tramDrawables` from `tramsFromMobilityState`
- call the existing train glyph drawing function with backend tram coordinates

Keep the visual glyph compact and selected state separate from cars. Do not add a local fallback when the list is empty.

- [x] **Step 3: Remove local train state from main runtime**

In `src/main.ts`:

- delete `buildTrains`
- delete local `trains` array
- delete per-frame train offset advancement
- delete `advanceTime` train mutation
- pass tram sprites to the renderer instead of `trains`

- [x] **Step 4: Update diagnostics**

In `runtimeDiagnostics.ts`, derive:

```ts
const backendTrams = tramsFromMobilityState(mobilityState, tramSprites, now, mobilityTickPeriodMs);
```

Report:

- `city.trains = backendTrams.length`
- `city.train = first backend tram diagnostic or null`
- `city.mobilityVehicles.vehicles` remains car-only for existing vehicle selection
- add `city.mobilityTrams.trams` for backend tram diagnostics

- [x] **Step 5: Update E2E expectations**

In `tests/e2e/render-smoke.spec.ts`:

- keep `expect(state.city.trains).toBe(4)`
- update train movement assertions to compare `state.city.mobilityTrams.trams[0]` between samples
- remove reliance on `city.train.position` if it is no longer the primary contract

- [x] **Step 6: Verify GREEN**

Run:

```bash
npx vitest run tests/render/backendTransitDrawables.test.ts tests/app/noProductionFallbacks.test.ts --passWithNoTests
npm test
npm run build
```

Expected: all exit 0.

## Task 5: Run Full Verification And Commit

**Files:**

- Modify: `progress.md`
- Modify: `docs/superpowers/plans/2026-05-28-backend-authoritative-transit.md`

- [x] **Step 1: Run Rust verification**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo fmt --manifest-path backend/Cargo.toml --all
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-core
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo test --manifest-path backend/Cargo.toml -p sim-server
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets -- -D warnings
```

Expected: all exit 0.

- [x] **Step 2: Run frontend and browser verification**

Run:

```bash
npm test
npm run build
npm run test:e2e -- tests/e2e/render-smoke.spec.ts
```

Expected: all exit 0.

- [x] **Step 3: Run forbidden fallback sweep**

Run:

```bash
rg -n "buildNorthboundTrainPath\\(|trainWrappedOffset\\(|fallback|legacy_seeded|trams_total|empty_for_world|tiny_world\\(|initial_world\\(" backend/crates/sim-core/src backend/crates/sim-server/src src tests -g '!src/backend/proto/**' -g '!backend/target/**'
```

Expected: no production fallback hits. Test names and explicit guard patterns are acceptable only when they assert absence.

- [x] **Step 4: Update progress**

Add a `progress.md` entry with the exact verification commands and results.

- [x] **Step 5: Commit**

Run:

```bash
git add backend/crates/sim-core/src/base_world.rs backend/crates/sim-core/src/mobility/seed.rs backend/crates/sim-core/src/mobility/mod.rs backend/crates/sim-server/src/runtime.rs src/render/backendMobilityDrawables.ts src/render/minimalMapRenderer.ts src/app/runtimeDiagnostics.ts src/main.ts tests/render/backendTransitDrawables.test.ts tests/app/noProductionFallbacks.test.ts tests/e2e/render-smoke.spec.ts docs/superpowers/specs/2026-05-28-backend-authoritative-transit-design.md docs/superpowers/plans/2026-05-28-backend-authoritative-transit.md progress.md
git commit -m "Make transit vehicles backend authoritative"
```

Expected: commit succeeds.
