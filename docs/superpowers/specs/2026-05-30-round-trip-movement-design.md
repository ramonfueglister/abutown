# Round-Trip / Cyclic Movement Design (minimal)

Date: 2026-05-30

## Status

Approved in brainstorming. A small, backend-only movement feature on the merged
abutopia world: replace the pedestrian's aimless random wander with a
**purposeful, cyclic A↔B routine**. It is the foundation for later time-of-day
routines (which will read the 8i `SimClock`). Implemented in its own worktree
(`plan/round-trip-movement`); near-disjoint from Codex's current frontend
agent-lifetime-display work.

## Current behaviour (verified)

The abutopia pedestrian does **not** idle. Its plan is a single
`PlanStage::Activity { activity_id: "activity:wander:N" }`, and the route system
(`mobility/systems/routing.rs`) special-cases the `Activity` stage to call
`next_wander_footway_link(...)` — i.e. the agent perpetually picks a **random**
adjacent footway link. So today's movement is an aimless random walk, not a
purposeful route.

`PlanStage::WalkToActivity { activity_id }` already routes purposefully: it
resolves `mobility_geometry::activity_geometry(activity_id).coord` → a
destination → HPA*/flow-field pathfinding. When the plan cursor advances past the
last stage (`plan.stages.get(cursor)` → `None`), the agent stops getting new
routes (does nothing).

## Goal

Make a pedestrian walk **purposefully between two waypoints (home ↔ destination)
and loop forever** — a visible, deterministic pendulum — instead of wandering
randomly. Keep the random wander as the fallback for agents without a routine.

## Architecture (minimal)

### 1. Cyclic plan
Add a `cyclic: bool` to `WalkPlan` (and a serde-default `cyclic` to `AgentRecord`,
round-tripping like `sex`/`birth_tick`). When the plan cursor would advance past
the last stage and `cyclic` is true, **wrap the cursor to 0** instead of
stalling. Apply the wrap at the cursor-advance sites (the route-advance in
`mobility/systems/walking.rs` and the stage handling in
`mobility/systems/routing.rs`) via one small shared helper so both paths behave
identically. Non-cyclic plans are unchanged (existing wander/idle behaviour).

### 2. Two waypoint activities at the abutopia houses
Two activities — `home` and `destination` — whose `activity_geometry` resolves to
the two house tiles (the buildings at the corridor ends, e.g. (2,3) and (13,3)).
How `activity_geometry` is populated (a hardcoded registry vs derived from the
base-world building layer) is the key planning question (§Open). The minimal
approach: register these two activity geometries so `WalkToActivity` resolves
them to the house coordinates.

### 3. abutopia seed plan
Seed the abutopia pedestrian with a **cyclic two-stage plan**:
`[WalkToActivity(home), WalkToActivity(destination)]`, `cyclic = true`. The
existing HPA*/flow-field routing pathfinds between the two houses along the
corridor. Result: the agent walks home → destination → home → … forever,
purposefully.

### Determinism & scope
- Routing is already deterministic; the cyclic wrap adds no randomness. Replay-safe.
- **Backend-only** (mobility plan/seed + cursor wrap). No wire/frontend change —
  the agent's streamed `world_coord` simply traces a purposeful path now.
- The random `wander` mechanism stays as the **fallback** for non-cyclic plans
  (additive change, nothing removed).

## What this is NOT (deferred)

- Dwell/pause at the endpoints (turn around immediately for now).
- Time-of-day scheduling (morning → work, night → home, via the 8i clock) — the
  natural next layer this enables.
- More than two waypoints / per-agent individual routines / activity selection.

## Testing

- **Cyclic cursor (unit):** a 2-stage cyclic plan, when the cursor advances past
  the end, wraps to 0; a non-cyclic plan does not wrap (cursor stays past end).
- **Activity resolution:** `home`/`destination` activities resolve to the
  expected house coordinates via `activity_geometry`.
- **Purposeful loop (integration):** seed the abutopia pedestrian with the cyclic
  plan; run the mobility schedule for enough ticks; assert the agent reaches the
  `destination` house tile, then heads back toward `home` (its `world_coord`
  oscillates between the two ends rather than drifting randomly), deterministically.
- Existing wander-based tests stay green (fallback unchanged).

## Open questions (resolve during planning, against real code)

1. How `mobility_geometry::activity_geometry` is populated — hardcoded table vs
   base-world-derived. Where to add the `home`/`destination` geometries, and the
   exact abutopia house coordinates (read `data/worlds/abutopia/layers/buildings.json`).
2. The exact cursor-advance sites to apply the wrap (`walking.rs` route advance +
   `routing.rs` stage handling) and whether one shared `advance_or_wrap(plan)`
   helper is cleanest.
3. Whether `cyclic` lives on `WalkPlan` + `AgentRecord` (serde-default) — confirm
   it round-trips through persistence like the other agent fields.
4. Whether the two house tiles are reachable as footway destinations from the
   corridor (the corridor connects them, so HPA* should route; verify a route is
   found, else extend the corridor/footway to the house tiles).
