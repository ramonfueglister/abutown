# World-Drift Hardening — data-driven activity waypoints (Reliability stream ②)

Date: 2026-05-30

## Status

Approved scope (stream ② of the 3-stream reliability/refactor pass: ① startup
reliability [merged, PR #42], ② world-drift hardening, ③ god-file splits). Own
branch + PR (`plan/world-drift-hardening`, branched fresh from `origin/main`
152432a).

## Problem

`mobility_geometry::activity_geometry` returns **hardcoded** coordinates for the
round-trip waypoints: `"activity:home" => (106.0, 64.51)`, `"activity:destination"
=> (117.0, 64.51)`. These are the abutopia south-sidewalk corridor endpoints for
*the current* 224×128 world. When the world is regenerated (as it was on
2026-05-30, costing hours of debugging — see memory `local-green-ci-red-stale-base`),
these literals go stale: production pedestrians route toward coordinates that no
longer match the corridor, so they spawn/route wrong **in the running game**.
The round-trip integration test already derives endpoints from the bundle (so it
fails loudly), but the production routing path is still coordinate-hardcoded.

## Goal

Make the round-trip waypoints **data-driven**: derive `home`/`destination` from
the loaded world's south-sidewalk corridor at seed time, so regenerating the
world cannot leave production routing pointing at stale coordinates. No hardcoded
world coordinates in the routing path.

## Architecture

### `ActivityWaypoints` resource
A new ECS resource — `ActivityWaypoints(HashMap<String, (f32, f32)>)` (in
`mobility/resources.rs`) — maps an `activity_id` to its world coordinate. It is
the **authoritative** source for resolvable activities; `activity_geometry`
remains only for activities with no world-derived coordinate (`activity:work`,
the wander/default fallback).

### Populated at seed time
The seed paths that have bundle/network access populate it:
- `from_base_world_bundle` / `seed_pedestrians_from_bundle`: for the pedestrian
  group's corridor (`corridor:sidewalk:south`), insert `"activity:home" =>
  corridor.points.first()` and `"activity:destination" => corridor.points.last()`.
  Generalises naturally — the waypoints are whatever the seeded corridor's ends
  are, in whatever world is loaded.
- A **default empty** `ActivityWaypoints` is installed wherever a mobility
  schedule is built (the mobility plugin / `empty_world_and_schedule` /
  `from_network`) so the routing system's `Res<ActivityWaypoints>` is always
  present (no missing-resource panic).

### Consumed in routing
`destination_for_stage` (routing.rs) gains an `&ActivityWaypoints` lookup. For
`WalkToActivity { activity_id }` it resolves the coordinate as:
`waypoints.get(activity_id)` → else `activity_geometry(activity_id).coord` →
`spatial.nearest(coord)`. The calling systems (`route_assignment_system`, and the
re-route comparison) add a `Res<ActivityWaypoints>` param and pass it down.

### `activity_geometry` cleanup
Remove the hardcoded `"activity:home"`/`"activity:destination"` arms. Keep
`"activity:work"` and the default. (The resource is authoritative for the
round-trip waypoints; the static fn no longer encodes any abutopia-specific
coordinate.)

## Determinism & scope

- Routing stays deterministic; the resource is built once at seed from ordered
  corridor points. Replay-safe.
- **Backend-only**, no wire/frontend change.
- Round-trip behaviour is unchanged on the current world (the resource yields the
  same `(106,64.51)`/`(117,64.51)` the literals did) — but now it tracks the data.

## Testing

- **Resource population (unit/integration):** after `from_base_world_bundle`, the
  world holds an `ActivityWaypoints` with `activity:home == south.first` and
  `activity:destination == south.last`, read from the bundle (assert equality
  against the bundle's corridor points — not hardcoded numbers).
- **Routing uses the resource:** with an `ActivityWaypoints` overriding
  `activity:home` to a custom coord, `destination_for_stage`/route assignment
  targets that coord (proves the resource is consulted, not the static fn).
- **Fallback:** an activity not in the resource (`activity:work`) still resolves
  via `activity_geometry`.
- **Regression:** the existing `round_trip_movement` tests stay green (the
  pedestrian still oscillates between the corridor ends) — now end-to-end
  data-driven.
- **Drift guard:** a unit test that fails if `activity_geometry` ever re-encodes
  `activity:home`/`activity:destination` (i.e., asserts those return the default,
  not a bespoke coord), so the hardcoding cannot silently come back.
- Full gate: `cargo test --workspace`, `clippy --workspace --all-targets -D
  warnings`, `fmt --check`, `build -p sim-server`.

## What this is NOT

- Not CHUNK_SIZE parametrization (the literal `32`) — separate, deferred.
- Not the god-file splits (stream ③).
- Not a new waypoint/activity system — only the existing home/destination become
  data-driven.

## Open questions (resolve in planning, against real code)

1. The exact mobility-plugin/schedule-build site(s) where the default
   `ActivityWaypoints` must be inserted so `Res<ActivityWaypoints>` is always
   present (empty_world_and_schedule, from_network, from_base_world_bundle).
2. All call sites of `destination_for_stage` (route_assignment_system + the
   re-route comparison at routing.rs:~409) that must thread the resource.
3. Whether to key the resource by the literal `"activity:home"`/`"activity:destination"`
   strings (simplest, matches the seeded plan) — yes, keep the existing ids.
