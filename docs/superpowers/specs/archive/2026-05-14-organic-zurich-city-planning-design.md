# Organic Zurich City Planning Design

## Goal

Turn the current visually correct but scattered Zurich river map into a more believable grown European flat city without changing the existing mechanics, camera, asset pipeline, or render style.

## Visual Target

The map should read as a city that grew around a river and station:

- A dense old-town core near the river with compact blocks and short, irregular streets.
- A clear rail/station and industry band that is separated from residential districts.
- Residential districts that become looser toward the edges.
- A protected river corridor with quays, green banks, bridges, and fewer random waterfront buildings.
- Forests as dense irregular patches, not rows of individual trees.
- Open expansion land that still feels intentional rather than empty leftovers.

## Generator Rules

Roads should keep the existing OpenGFX road rendering and rail compatibility, but district streets should produce fewer long rectangular loops and fewer parallel grid runs. Arterials remain the connected backbone. District streets should be more compact near old town and less dense at the edges.

Buildings should be placed by district role:

- Old town: high density, oldhouses/townhouses/shops/church, close to the river but not on raw bank tiles.
- Rail center: shops/flats/office/townhouses, clustered around the station and rail line.
- Residential: houses/cottages/townhouses, lower density and stronger distance falloff.
- Industry/civic: fewer random residential-looking buildings, more controlled placement near their zone centers.
- Reserve: very sparse buildings.

Building placement should favor compact clusters. Isolated single buildings far from the zone center should be reduced. This can be implemented by density falloff, stricter frontage selection, and post-placement pruning.

The river should stay flat and remain the central visual feature. Buildings should generally stay at least two tiles from water, except in explicit old-town or waterfront corridors. Bridges must have connected road approaches on both banks.

Forests should be generated as clustered tree patches with irregular edges. Forest zones need higher local density and should avoid perfectly aligned rows.

## Non-Goals

- No hills yet.
- No new game mechanics.
- No UI changes.
- No change to camera behavior.
- No replacement of the current OpenGFX asset pipeline.
- No large renderer rewrite.

## Acceptance Criteria

- City validation remains clean: no road/rail overlap outside crossings, no invalid buildings, no tree/building overlap.
- At least three connected bridge spans remain.
- Old-town zones have materially higher building density than residential edge zones.
- The river corridor has fewer buildings directly adjacent to water.
- Forest zones contain dense local clusters instead of sparse rows.
- Building frames remain restricted to finished OpenGFX rows.
- E2E render smoke test passes.
