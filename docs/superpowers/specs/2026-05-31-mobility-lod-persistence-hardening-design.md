# Mobility LOD Persistence Hardening Design

Date: 2026-05-31

## Status

Approved by the user's "lets go" after the Mobility Scale v1 merge.

## Goal

Keep Abutopia's 300 backend-authored base-world walking agents concrete,
simulated, health-gated, and persistable even after a browser viewport
unsubscribes from their chunk.

## Context

Mobility Scale v1 made the authored spawn layer the population contract:
Abutopia has 300 real backend-authored walking agents, and persistence refuses
to write a mobility snapshot with fewer concrete agents.

The current LOD path can violate that contract. After a viewport subscribes to a
chunk and then unsubscribes, the chunk can cool back to `Warm`; the LOD demotion
system then folds its walking agents into `flow_cells`. That is valid for later
large-scale aggregate simulation, but it is not valid for the current Abutopia
slice because the canonical Supabase row and `/health` require 300 concrete
base-world agents.

## Design

Add a server-owned active-chunk pin for authored base-world mobility corridors.
Pinned chunks are treated as at least `Active` by the chunk LOD reclassifier
without faking browser subscriber counts. Browser subscriptions can still raise
a chunk to `Hot`, and non-pinned chunks continue to use normal LOD behavior.

Runtime startup computes the pinned chunks from the loaded base-world transport
paths that author mobility population:

- pedestrian corridors referenced by `spawns.pedestrian_groups`,
- car arterial paths referenced by `spawns.car_groups`.

For the current Abutopia data this pins the south sidewalk chunk that owns the
300 walking agents. The agents stay concrete and keep moving even when no
browser is connected, so the persistence guard and Supabase freshness smoke
continue to validate the real authored population.

## Non-Goals

- No frontend fallback agents.
- No `flow_cells` reinterpretation as concrete people.
- No schema change to `mobility_snapshots`.
- No chunked mobility persistence in this slice.
- No Economy changes.

## Acceptance Criteria

- A runtime test proves Subscribe -> Unsubscribe -> cooldown does not demote
  Abutopia's 300 base-world agents out of the persist snapshot.
- A LOD classifier test proves pinned chunks stay `Active` without mutating
  `ChunkSubscriberCount`.
- Existing non-pinned LOD demotion tests still pass.
- Backend health and mobility persistence keep the existing concrete-agent
  integrity gate.
