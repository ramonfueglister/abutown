# Zurich-Inspired OpenGFX River City World Design

## Goal

Create a flat, grown European OpenTTD-style city world with a Zurich-like feeling: a river as the main spatial anchor, a dense historic core near the water, a strong rail and employment center, looser residential quarters, forests, parks, civic places, industry at the edge, and enough future development space for a persistent multiplayer city.

The target world size is 256 by 256 tiles. The current demo may still show a curated smaller viewport or initial area, but the design direction should stop treating the world as a 96-tile showcase map.

Existing mechanics, camera behavior, vehicle movement, rendering style, and OpenGFX visual language must remain intact. The work is about map composition, richer OpenGFX asset coverage, and better deterministic placement.

## Core Decisions

- The map is flat for now. No hills or height simulation are part of this design.
- The world should feel like a grown European river city, not a perfect grid or a fantasy showcase.
- The long-term map target is 256 by 256 tiles, using 32 by 32 chunks as the planning unit.
- The current visible experience may remain a curated subset until chunk-aware rendering and simulation are implemented.
- Import as much useful OpenGFX asset coverage as practical, but place assets through semantic categories instead of random sprite usage.
- The design must preserve existing gameplay and rendering mechanics unless a later implementation plan explicitly scopes a narrow change.

## World Scale

For a persistent city with up to 2000 players and families, a 96 by 78 map is too small. It is suitable for a demo, not for a city that many people can influence over time.

The recommended scale is 256 by 256 tiles:

- large enough for a recognizable city core, multiple quarters, river corridor, forests, railway area, industry, civic places, and future expansion;
- small enough to curate visually and reason about as one city rather than a whole region;
- compatible with a 32 by 32 chunk layout, giving 64 chunks total;
- suitable for representing families through buildings, apartments, parcels, or businesses instead of one visible house per family.

Families should not map one-to-one to individual visible house sprites. A block, flat, townhouse row, or mixed-use building can represent multiple households. This keeps the world dense and believable without filling every tile with housing.

## City Composition

The 256 by 256 world should be composed as a compact city region with deliberate empty space and green structure.

### River Axis

A broad, slightly meandering river runs through the city as the main organizing feature. It should not split the map into two disconnected halves. It should create edges, promenades, parks, bridge nodes, and dense waterfront areas.

The first target should include three to five meaningful bridge crossings. Bridges should connect important city centers or transport corridors, not appear at arbitrary intervals.

### Old Town

The old town sits near the river with dense, irregular streets. It uses older houses, townhouses, shops, civic buildings, churches, small plazas, and tight blocks. It should read as the historic heart of the city.

The street pattern should be organic and constrained by the river, not a square grid. Small open places are allowed, especially near civic or church assets.

### Rail And Central Business Area

A second dense center forms near the main rail line and station. This zone uses shops, offices, flats, station assets, road vehicles, rail details, and bus or truck activity. It should feel more modern and transport-driven than the old town.

Rail should be treated as a strong urban edge and growth driver. Industry and employment can sit near rail, but the rail corridor must not visually collide with normal streets except at intentional crossings.

### Residential Quarters

Residential quarters form looser rings around the old town and rail center. They use houses, cottages, old houses, townhouses, smaller shops, local streets, pocket parks, and tree-lined roads.

These areas should have local variation. Some quarters can be denser and older; others can feel suburban or village-like at the edge.

### Forests, Parks, And Green Corridors

Trees should be grouped into readable forests, parks, river-edge green corridors, and street avenues. Random single-tree scattering should be secondary.

The map needs meaningful green areas:

- forest belts or blocks near the map edge;
- parks along the river;
- civic parks near important buildings;
- green buffers around rail, industry, or reserve areas;
- small pocket parks inside residential areas.

### Industry And Edge Uses

Industrial or heavy commercial uses belong near the railway, map edge, or service roads. They should not dominate the old town or central riverfront.

Industry can use OpenGFX industrial, rail, fence, yard, truck, and utility details when available.

### Player Development Reserve

The city should include multiple future development reserves: empty, lightly built, or semi-rural areas that can later be claimed or transformed by players.

Reserve areas should not look broken or unfinished. They can appear as grassland, edge woods, small village fabric, rail-adjacent yards, or open parcels.

## OpenGFX Asset Strategy

The project should import broad OpenGFX asset coverage, preferably from the canonical OpenGFX source, while preserving license and provenance information.

Asset coverage should be organized by semantic categories:

- terrain and ground;
- water, river, shore, and bridge details;
- roads, road overlays, street crossings, and street furniture;
- rail, stations, yards, fences, crossings, and rail details;
- old town buildings;
- residential buildings;
- commercial and office buildings;
- civic and landmark buildings;
- industrial buildings and details;
- parks, trees, forest objects, and green details;
- vehicles and service objects;
- decorative objects that fit the OpenGFX style.

The goal is to make as many OpenGFX assets available as practical, but not to force every sprite onto the first map. Asset use should follow zone rules, density rules, and visual quality rules.

If a specific object such as a fountain is not available as a clean OpenGFX sprite, it should not be faked in a different art style. Instead, the place should be made readable using available OpenGFX plaza, water, civic, or park assets.

## Architecture

This should be a composition-driven extension, not a renderer rewrite.

### World Layout

World layout defines the 256 by 256 target map: river, forests, quarter centers, road and rail corridors, industry, parks, civic nodes, and reserve zones.

This layer is responsible for city intent and large-scale readability.

### Generated Placement

Generated placement turns the layout into concrete tiles and objects: terrain, water, roads, rails, bridges, buildings, trees, plazas, parks, stations, and decorative details.

Placement should be deterministic. Given the same seed and inputs, it should produce the same world. This is important for tests, screenshots, debugging, and future persistence.

### Existing Rendering

The existing Canvas/OpenGFX rendering approach remains the presentation layer. It should receive richer data and more asset choices, not a new rendering architecture as part of this design.

Camera, vehicles, current sprite drawing, and interaction behavior should remain stable.

### Chunk Planning

The world should be planned around 32 by 32 chunks. Full chunk streaming is not required for the first implementation, but the data model should avoid assumptions that everything will always be rendered or simulated as one small demo map.

## Data Flow

1. Import OpenGFX assets and preserve license/provenance files.
2. Build or extend an asset manifest so rendering code can use semantic asset categories instead of hard-coded sprite coordinates everywhere.
3. Generate the 256 by 256 world layout.
4. Generate concrete placement from zone rules and deterministic seed values.
5. Validate conflicts, density, coverage, and reserved areas.
6. Render the current visible area using existing Canvas/OpenGFX rendering.
7. Use browser screenshots and diagnostics for visual QA.

## Quality Rules

- No normal building on water.
- No building directly on road or rail tiles.
- Roads and rails may overlap only at intentional crossings.
- Bridges must correspond to river crossings.
- Dense areas should have more buildings and fewer random trees.
- Forest areas should use grouped trees and readable edges.
- Old town streets should be irregular and compact.
- Residential quarters should be looser than the old town.
- Industrial zones should be near rail, service roads, or map edges.
- Reserve areas must remain available for later player influence.
- Asset variety should increase visual richness without turning the city into a sprite catalog.

## Error Handling And Diagnostics

- Missing asset frame: fall back to a compatible category and report the missing key in diagnostics.
- Invalid overlap: count it and fail validation when it violates a hard rule.
- Excessive visible density: enforce per-zone caps for details, trees, and buildings.
- Missing asset category: use a lower-detail category and keep the map valid.
- Large world performance risk: render and simulate by visible area or chunk in later implementation phases.

## Testing And Verification

The implementation plan should include tests for:

- deterministic world generation;
- target 256 by 256 dimensions;
- chunk boundary sanity;
- no invalid road, rail, water, and building overlaps;
- intentional bridge and rail crossing counts;
- minimum forest and green-space coverage;
- reserve area coverage;
- minimum asset category diversity;
- existing vehicle and camera behavior still working;
- browser screenshot verification of the rendered city.

Visual QA should compare the result against the official OpenTTD visual standard rather than low-quality community screenshots. The city should look like a believable OpenGFX/OpenTTD city first and a technical demo second.

## Out Of Scope

- Hill or height simulation.
- Replacing the renderer.
- Changing vehicle mechanics.
- Adding visible UI.
- Simulating all 2000 players and families.
- Implementing full multiplayer persistence.
- Forcing every imported sprite to appear in the first visible map.

## Acceptance Criteria

- A deterministic 256 by 256 flat river city layout exists or is planned in implementation detail.
- The map reads as a grown European river city with old town, rail center, residential quarters, forests, parks, industry, and development reserves.
- OpenGFX asset import is broadened and organized by semantic categories.
- Existing mechanics and rendering behavior remain stable.
- Tests and diagnostics protect against invalid overlaps and map composition regressions.
- The rendered result is visually closer to official OpenTTD screenshots than to the current sparse demo composition.
