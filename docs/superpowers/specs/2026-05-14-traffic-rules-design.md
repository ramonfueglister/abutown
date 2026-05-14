# Traffic Rules Design

Date: 2026-05-14

## Status

Approved design direction for the first visible traffic-rule slice. This spec is intentionally aligned with the Rust authoritative simulation architecture and the visible backend slice. It must not become a permanent browser-authoritative traffic simulation.

## Goal

Add believable, invisible traffic rules for cars at road intersections while keeping the visible playfield clean. Cars should slow, yield, wait before conflict zones, reserve safe crossing windows, and continue smoothly without visible signs, traffic lights, HUD, or debug chrome.

The first implementation is a local deterministic reference model so the current graphics demo can show the behavior now. The model must be shaped so a later Rust ECS authority can replace it without rewriting rendering or vehicle presentation.

## Architecture Fit

Existing backend architecture rules remain binding:

- Rust is the only long-term simulation authority.
- Browser code is presentation, interpolation, and development-time reference behavior.
- Supabase/Postgres is durable state, not the hot simulation loop.
- Simulation uses stable IDs, tick/version counters, chunk ownership, and relevance-filtered deltas.
- Rendering must not own durable traffic decisions.

The traffic-rule slice therefore introduces an authority boundary:

```text
Road graph + vehicle route state
  -> Traffic authority interface
  -> per-vehicle movement decisions
  -> renderer/interpolator
```

For the demo this authority is `LocalTrafficAuthority`. Later it becomes Rust-owned ECS systems that publish snapshots and deltas through the backend bridge.

## Scope

In scope:

- Detect intersections from road graph connectivity.
- Assign deterministic stable `intersectionId` values.
- Assign stable `vehicleId` values for the visible demo vehicles.
- Reserve intersection conflict windows by tick.
- Let only compatible vehicles enter a conflict zone at the same time.
- Make blocked cars decelerate and wait before the intersection.
- Release reservations after the crossing window.
- Expose diagnostics through `render_game_to_text`, not through visible UI.

Out of scope for this slice:

- Visible traffic lights, signs, road markings, or HUD.
- Full Rust implementation.
- Supabase schema.
- Player actions.
- Lane changing, overtaking, parking, crashes, emergency vehicles, or police rules.
- Full 100,000-car load in the browser.
- Full global pathfinding replacement.

## Traffic Model

The first visible model is reservation-based intersection control.

Each intersection has a compact state:

- `intersectionId`
- grid coordinate
- connected approach edges
- current reservations
- tick/version metadata

Each reservation has:

- `reservationId`
- `intersectionId`
- `vehicleId`
- `enterTick`
- `exitTick`
- `approachEdge`
- `exitEdge`
- `conflictMask`
- `priority`

Each vehicle receives a `TrafficDecision` for the current tick:

- `go`: continue normally.
- `yield`: slow down because a conflict may happen soon.
- `stop`: stop before the reservation boundary.
- `blocked`: no slot is available yet.

The first priority rule is deterministic and simple:

1. Vehicles already inside the conflict zone keep their reservation until `exitTick`.
2. Vehicles with non-conflicting reservations may pass together.
3. Conflicting requests are ordered by predicted arrival tick, then stable `vehicleId`.
4. No vehicle may stop inside the intersection because of a denied reservation.

This is intentionally closer to autonomous/reservation intersection management than traffic lights, but tuned visually to feel like normal city traffic.

## ECS And LOD Fit

The local TypeScript model must map directly to future Rust ECS components and resources:

- `VehicleRoute`: route id, current path index/offset, next intersection candidate.
- `VehicleKinematics`: speed, desired speed, acceleration state.
- `TrafficAgent`: vehicle id, priority class, current decision.
- `IntersectionNode`: intersection id, chunk id, connected edges.
- `IntersectionReservationTable`: compact per-intersection reservation windows.
- `TrafficChunkState`: active/warm/asleep traffic LOD state.

The implementation must avoid global scans that would not scale. Even in TypeScript, the shape should be:

- index vehicles by path/intersection proximity,
- update only intersections near active vehicles,
- keep reservation tables per intersection,
- keep diagnostics as counters, not large logs.

For 100,000 future cars, the model is explicitly LOD-based:

- Visible/hot chunks: microscopic reservations and following behavior.
- Active nearby chunks: normal per-vehicle traffic decisions at a lower budget.
- Warm chunks: queue/flow approximation by road segment and intersection throughput.
- Asleep chunks: scheduled catch-up from aggregate events and elapsed time.

The browser may render many visible vehicles, but it must not become responsible for simulating all 100,000 cars every frame.

## Frontend Reference Implementation

The first implementation should add pure, testable TypeScript modules:

- `src/traffic/trafficTypes.ts`
  Serializable IDs, reservations, decisions, snapshots, and diagnostics.
- `src/traffic/intersections.ts`
  Road graph to intersection metadata. No rendering dependency.
- `src/traffic/localTrafficAuthority.ts`
  Deterministic reservation logic. No canvas dependency.

`src/main.ts` may integrate the reference model by passing route state into the authority during `advanceCars(dt)`, then applying the returned speed/stop decisions to existing vehicle motion.

The renderer must continue to draw only the final pose and sprite. It must not decide traffic priority.

## Backend Handoff

When the Rust backend traffic system exists:

- Rust owns `IntersectionReservationTable`.
- Rust owns `TrafficDecision` for authoritative vehicles.
- The browser receives vehicle snapshots/deltas with tick/version.
- The local authority can be disabled or kept only for offline demo mode.
- Client interpolation can continue using `vehicleRenderPose`.

The protocol does not need to be implemented in this slice, but data names and semantics should stay compatible with the visible backend slice: world id, tick, version, chunk, stable IDs, and resync-friendly snapshots.

## Error Handling And Diagnostics

The first diagnostics should appear in `render_game_to_text`:

- `reservedIntersections`
- `yieldingVehicles`
- `stoppedForTrafficRules`
- `blockedVehicles`
- `intersectionConflictsPrevented`
- `expiredReservations`
- `trafficRuleDecisionCount`

Hard errors should remain impossible by construction:

- no car may be moved off road by traffic rules,
- no car may move onto rails because of traffic rules,
- no car may enter a denied intersection slot,
- no denied car may roll past its stop boundary.

If the traffic authority cannot classify an intersection or route segment, it should return `go` and increment a diagnostic warning rather than freezing the whole demo.

## Testing

Unit tests:

- two conflicting cars request the same intersection and only one gets the first slot,
- non-conflicting movements can reserve the same intersection window,
- a denied car receives `yield` or `stop` before entering the conflict zone,
- reservations expire after `exitTick`,
- stable ids make tie-breaking deterministic,
- unknown route/intersection data degrades without throwing.

Runtime tests:

- existing vehicle diagnostics remain clean: off-road, rail, teleport, U-turn, and following gaps,
- `render_game_to_text` reports traffic rule counters,
- browser smoke test keeps rendering without visible UI.

## Success Criteria

The slice is successful when the running demo visibly shows cars yielding at intersections without UI clutter, while the code clearly separates traffic decisions from rendering and can be replaced by a Rust ECS authority later.

It is not successful if the browser-only implementation becomes the canonical simulation model, if it requires scanning every car against every intersection, or if it adds visible signs/controls to explain the behavior.
