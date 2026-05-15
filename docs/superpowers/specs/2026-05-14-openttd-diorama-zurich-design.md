# OpenTTD Diorama Zurich Design

## Goal

Shift the map from a broadly generated city into a composed OpenTTD-style screenshot scene. The first view should read as a transport diorama: station, rail yard, quay, docks, industry, compact town blocks, and landscape edge.

## Reference Pattern

Good OpenTTD screenshots are not mostly houses. They are built around strong transport setpieces:

- Large passenger stations and visible rail throat geometry.
- Industrial yards, depots, docks, ships, fences, fields, and station tiles.
- Dense urban blocks that frame infrastructure.
- Water edges with quay/dock objects, not bare blue water.
- Forests and fields as scene borders.

## Scope

Keep existing camera, mechanics, OpenGFX import, vehicle movement, and validation. Add a diorama layer using existing OpenGFX/OpenGFX2 assets already imported into `public/opengfx2/all`.

## Design

- Expand rail generation around the Zurich main-station zone with multiple parallel platform/yard tracks.
- Increase visible rail station tiles from a tiny cluster to a station complex.
- Promote `ZurichDetail` from data-only to rendered visual objects.
- Add deterministic detail categories for station, dock/quay, industry, fields, civic/plaza, and park.
- Reserve important setpiece detail tiles before building placement so houses do not overwrite docks, station areas, and yards.
- Keep buildings compact around roads but make infrastructure the primary visual anchor.

## Acceptance Criteria

- Rail tile count increases enough to visibly dominate the station area.
- Runtime exposes at least 14 rail station tiles.
- Placement has visible station, dock, industry, and field detail categories.
- Validation remains clean.
- E2E render smoke passes.
