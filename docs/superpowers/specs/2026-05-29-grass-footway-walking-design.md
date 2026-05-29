# Grass Footway Walking Design

Date: 2026-05-29

## Status

Approved direction in chat. This design extends backend-authoritative walking so
pedestrians may walk on sidewalks and grass, while roads stay forbidden for
walking agents.

## Spec alignment

- Matches `2026-05-29-sidewalk-footway-simulation.md`: roads, buildings, and
  world tiles remain valid infrastructure. Sidewalk footways stay the authored
  near-road walking surface.
- Matches `2026-05-29-time-system-and-agent-aging-design.md`: birth, death,
  life stages, and demographics are untouched. This only changes walkability.
- Preserves the backend-authoritative model: the frontend renders backend
  mobility snapshots and does not synthesize pseudo-agents.

## Goal

Allow walking agents to move on green grass areas in addition to authored
sidewalk footways, while never creating pedestrian routes on road, building, or
water tiles.

## Architecture

Grass walkability is represented as normal routing `Footway` edges in the
backend graph. When a base-world bundle is available, the mobility seeder adds
deterministic grass footway links between adjacent grass-tile centers, plus
short connector links from authored sidewalk endpoints to nearby grass tiles.
Walking agents continue to use `AgentMobilityState::Walking { link_id, progress
}` and the existing backend snapshot/protobuf flow.

For simple wandering agents whose plan is an `Activity`, completing a walking
edge chooses the next connected footway deterministically from the graph instead
of resetting onto the same link. This keeps current Abutopia pedestrians moving
across the walkable surface without adding a frontend debug path.

## Constraints

- Do not emit grass footways on `TileKind::Road`, `TileKind::BuildingFootprint`,
  or `TileKind::Water`.
- Do not regenerate roads, buildings, terrain, or world layout.
- Do not add a fallback frontend renderer path.
- Keep movement deterministic from agent id, graph topology, and tick.
- Keep authored sidewalk corridors; grass expands the walkable network.

## Testing

- Base-world seeding creates grass footway links only for grass tiles and never
  for road/building/water tiles.
- Abutopia's routing graph contains grass footways and keeps the existing
  sidewalk footways.
- A walking agent with an `Activity` stage advances from a completed footway to
  a connected next footway instead of looping on the same sidewalk forever.
- Existing sidewalk and runtime tests remain green.
