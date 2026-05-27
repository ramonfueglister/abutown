# Phase 8g Seed Slice — Layered Terrain Seed

**Date:** 2026-05-27
**Status:** Design
**Phase in roadmap:** 8g seed slice, before domain tiles and before economy

## Goal

Make the physical map backend-authoritative and persistent.

The current visible Zurich map is assembled mostly in frontend TypeScript from `buildZurichWorld`, `buildZurichTransport`, and `buildZurichPlacement`. That was useful for fast visual iteration, but it is the wrong long-term authority. This phase creates a clean backend terrain seed so the runtime owns every physical tile layer for the 256x256 world.

This phase is deliberately **terrain only**. It does not add companies, homes, workplaces, owners, jobs, storage, production, money, markets, or ledger logic.

## Non-Negotiables

- No legacy data model as the target state.
- No large one-dimensional `TileKind` enum as the durable model.
- No duplicate frontend/backend runtime truth.
- No economy/domain semantics in this phase.
- No "forestry", "workplace", "company", "housing", or ownership fields.
- Existing frontend procedural city code may be used as a seed generator, but not as runtime authority after this phase.

## Current State

The world is 256x256 tiles, chunked into 64 chunks of 32x32 tiles.

Existing backend persistence stores dense chunk tile arrays, but the tile model is too coarse:

```rust
TileKind::Grass
TileKind::Water
TileKind::Road
TileKind::BuildingFootprint
```

The frontend has richer physical layers already:

- base terrain: `grass`, `water`, `riverbank`, `forest`, `park`, `reserve`, `plaza`
- transport: `street`, `bridge`, `rail`, `rail_crossing`
- cover: buildings, trees, details
- metadata: zone id, road mask, rail mask

The problem is that these richer layers live outside the authoritative backend runtime.

## Design

Introduce a clean layered physical tile record as the target runtime/persistence shape.

```text
LayeredTileRecord
- base: Grass | Water | Riverbank | Forest | Park | Reserve | Plaza
- surface: None | Street | Bridge | Rail | RailCrossing
- cover: None | Building | Tree | Detail
- display: optional visual hint, renderer-only meaning
- zone_id: optional stable zone identifier
- road_mask: optional u8
- rail_mask: optional u8
- version: u64
```

The layers are intentionally separate. A tile can be `Riverbank + Bridge`, `Grass + Rail`, or `Grass + Building` only where validation allows it. The model should describe the physical map without assigning economic meaning to it.

`display` is visual only. For example, a building may carry a visual hint such as `oldhouses`, `shops`, `office`, or `church`, but that does not mean the tile is a residence, company, workplace, civic institution, or economic actor.

## Seed Source

The first terrain seed is generated from the existing Zurich frontend world builders:

- `buildZurichWorld` for base terrain and zones
- `buildZurichTransport` for roads, bridges, rails, rail crossings, and masks
- `buildZurichPlacement` only for physical cover such as building footprint, tree, and visible detail display hints

The generated artifact becomes an explicit seed input for the backend. The frontend builders stop being runtime truth once backend tile data is exposed to the renderer.

The seed artifact should include:

```json
{
  "version": 1,
  "world_id": "zurich-river-city-v1",
  "width": 256,
  "height": 256,
  "chunk_size": 32,
  "tiles": [
    {
      "x": 0,
      "y": 0,
      "base": "Grass",
      "surface": "None",
      "cover": "None",
      "display": null,
      "zone_id": "zone:north-forest",
      "road_mask": null,
      "rail_mask": null
    }
  ]
}
```

The artifact can be dense. Runtime can still optimize snapshots later. Correctness and single source of truth matter more than prematurely minimizing a 65k-tile seed.

## Backend Runtime Ownership

Backend startup loads the terrain seed when creating a fresh world. It populates chunk tile arrays with `LayeredTileRecord` values. The runtime then serves physical tile data from its ECS world and persists mutations through the existing snapshot pipeline or a replacement terrain snapshot provider.

The target state is:

- backend owns all physical tile layers
- frontend asks backend for tile/chunk data
- renderer draws the map from backend data
- frontend procedural Zurich builders are no longer used by runtime rendering

Legacy `TileKind` may temporarily exist only as an adapter boundary while code is migrated. It must not be the target model, and the implementation plan must include removal or isolation criteria for that adapter.

## Validation Rules

The seed generator and backend loader both validate the same physical invariants:

- every tile coordinate is inside the world bounds
- exactly one base layer is present
- surface and cover values are known enum variants
- road masks are present only for `Street`, `Bridge`, or `RailCrossing`
- rail masks are present only for `Rail` or `RailCrossing`
- `Bridge` may only sit on `Water` or `Riverbank`
- `Building` cover may not sit on `Water`
- `Building` cover may not coexist with `Street`, `Bridge`, `Rail`, or `RailCrossing`
- `Tree` cover may not coexist with `Street`, `Bridge`, `Rail`, or `RailCrossing`
- road/rail overlap is allowed only as `RailCrossing`
- generated tile count equals `width * height`
- chunk sizing exactly partitions the 256x256 world into 32x32 chunks

These are physical-map rules only. No economic capacity or ownership validation belongs here.

## Frontend Rendering Impact

The Mini Motorways renderer keeps its visual style, but changes its data source.

Instead of deriving visible roads, rails, buildings, parks, water, trees, and details from frontend-local Zurich builders, it maps backend `LayeredTileRecord` data into draw calls.

This should improve building placement work because renderer and backend will share one physical truth. Later visual improvements can transform or aggregate backend tiles for display, but they must not invent a different map.

## Persistence

The clean target is a terrain/chunk snapshot provider that serializes layered tiles, not the old `TileKind` shape.

Acceptable transition:

- add new layered terrain payload version
- load old rows only through a narrow migration adapter if needed for tests/local dev
- write only the new layered format once the provider is active

Unacceptable transition:

- extending the old `TileKind` enum until it becomes the permanent model
- keeping frontend placement as a runtime fallback
- storing economic/domain fields in terrain records

## Out Of Scope

- companies
- homes
- apartments
- workplaces
- forestry or resource extraction
- storage/economy inventory
- ownership
- jobs
- production recipes
- money
- ledger
- demand simulation
- UI for editing terrain
- procedural map generation beyond the Zurich seed export

## Acceptance Criteria

- A deterministic Zurich layered terrain seed artifact exists.
- Backend fresh-world startup loads that artifact.
- Backend runtime exposes layered tile data per chunk.
- Backend persistence writes layered tile data, not the old one-dimensional tile kind as the target format.
- Frontend renderer draws physical map layers from backend tile/chunk data.
- Runtime no longer depends on frontend Zurich builders for visible map truth.
- Validation rejects invalid physical layer combinations.
- Existing mobility still works on the same 256x256 world and road network.
- No economy/domain terms are introduced into runtime tile records.

## Follow-Up Phases

After this seed slice, a later 8g domain-tile spec can add sparse ECS tile entities for domain semantics. Phase 8h can then introduce economy and ledger. Those phases must build on this terrain truth instead of replacing it.
