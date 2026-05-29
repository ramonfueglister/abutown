# Isometric City Graphics Demo Design

Date: 2026-05-11

## Status

Approved for specification. This document defines the first implementation scope only: a client-side isometric graphics demo for a realistic, simulation-ready city. Backend persistence, multiplayer, resident simulation, vehicle simulation, Supabase, and Vercel deployment are intentionally out of scope for this first demo.

## Goal

Build a beautiful isometric 2D city demo using OpenGFX2 Classic 64px assets. The first scene is a river-based polycentric city: a realistic landscape with a river, bridges, multiple neighborhood centers, roads, blocks, parcels, buildings, parks, trees, and landmarks.

The demo is visual first, but the city must not be a decorative tile painting. It must be generated from a structured city model that can later support individual residents, vehicles, routing, persistence, and multiplayer simulation.

## Chosen Approach

Use Vite, TypeScript, and PixiJS.

PixiJS provides modern WebGL-backed 2D rendering, efficient sprite layers, camera transforms, and enough control for a custom isometric renderer. TypeScript keeps the city model explicit and serializable. Vite keeps the app simple to run and later deploy.

The core pipeline is:

```text
research-informed city generator
  -> persistent city data model
  -> isometric projection
  -> PixiJS render layers
```

## Visual Style And Assets

The primary asset target is OpenGFX2 Classic, using 64px isometric tiles. This is preferred over the 256px high-definition set for the MVP because it gives a more complete asset base, more visible city area, lower rendering pressure, and a better foundation for later large-scale simulation.

OpenGFX2 is GPL-2.0 licensed. The project must keep license attribution and asset provenance visible in the repository documentation once assets are imported.

Asset usage rules:

- Use OpenGFX2 Classic as the primary visual style.
- Do not mix in high-definition sprites for the first demo unless explicitly revisited.
- Do not produce custom art in MVP 1 beyond simple technical placeholders where an asset is temporarily missing.
- Beauty should come from city composition, landscape, water, density, parks, landmarks, variation, and camera framing.

## City Form

The first demo city is a River Polycentric City.

The map should feel like a city that grew around a river, not a rectangular grid stamped over terrain. The river acts as both landscape feature and urban edge. Bridges are important movement constraints and visual anchors. Several neighborhood centers form along reachable parts of the river and are connected by primary roads.

The city should include:

- A river with banks and crossings.
- Multiple neighborhood centers.
- A small number of major roads connecting centers, bridges, and exits.
- Secondary streets that adapt to terrain and block shape.
- Parks, plazas, and landmark structures.
- Districts with visible differences in density and building mix.

## Research Basis

The generator should use practical heuristics derived from urban modeling and urban design research:

- Parish and Mueller, "Procedural Modeling of Cities" (2001): global goals and local constraints for street growth.
- Chen et al., "Interactive Procedural Street Modeling" (2008): tensor-field-like guidance for coherent street patterns.
- Vanegas et al., "Procedural Generation of Parcels in Urban Modeling" (2012): block subdivision into plausible parcels with street access.
- Hillier et al., "Natural Movement" (1993): street configuration influences movement patterns; central/integrated streets matter.
- Kevin Lynch, "The Image of the City" (1960): paths, edges, districts, nodes, and landmarks as readability checks.
- Bettencourt et al., "Growth, innovation, scaling, and the pace of life in cities" (2007): future population and infrastructure scale constraints.

The MVP does not need to implement academic models literally. It should encode a small, inspectable set of rules inspired by these sources.

The selected weighting is balanced:

- Morphology and movement are hard model constraints.
- Visual readability is a quality gate.

## City Generation

The demo uses one curated default seed. The city is deterministic: the same seed and generator version must produce the same city structure.

Generation stages:

1. Generate terrain and river path.
2. Select bridge candidate locations from terrain and center accessibility.
3. Place multiple neighborhood centers along reachable river-adjacent areas.
4. Create major roads between centers, bridges, and map exits.
5. Grow secondary streets from the major network using terrain, density, and block-size constraints.
6. Extract blocks from the road graph.
7. Subdivide blocks into parcels.
8. Assign districts, densities, land-use hints, parks, plazas, and landmarks.
9. Place buildings from parcel type, district, centrality, and available OpenGFX2 assets.
10. Validate that roads, parcels, and visible tiles agree.

The curated seed may be hand-tuned by changing generator parameters, but the final city must still be produced by the generator rather than hand-painted tile by tile.

## Simulation-Ready Data Model

The city model is the source of truth. Rendering reads from it.

Core entities:

- `Tile`: terrain, water, slope/edge hints, and render material.
- `RoadNode`: graph node with world coordinate, intersection metadata, bridge metadata, and future traffic role.
- `RoadEdge`: directed or bidirectional edge with geometry, orientation, lane/mode metadata, cost, and connected nodes.
- `Block`: polygon or tile-region enclosed by roads or natural boundaries.
- `Parcel`: buildable subdivision with access road, district, land-use hint, and future capacity.
- `Building`: asset choice, footprint, parcel, district, role hints, and future resident/workplace capacity.
- `District`: named or typed area with density, use mix, centrality, and readability role.
- `Landmark`: orientation anchor for the player and future destination anchor for agents.

Every entity must have a stable ID. IDs should be deterministic where practical.

## Road Direction And Correct Sprites

Roads are not decorative tiles. They are graph geometry first.

Each road segment must store enough information for future routing and correct rendering:

- connected start/end road nodes,
- directionality,
- lane/mode hints,
- orientation such as north-east, north-west, east-west, curve, intersection, bridge, or dead-end,
- cost metadata for future pathfinding.

The renderer must choose road sprites from the road graph, not from manual tile placement. A road tile is valid only if its visible OpenGFX2 sprite matches the model geometry. Intersections, curves, bridges, and dead-ends must come from node/edge connectivity.

Debug mode should be able to show road nodes, road edges, directions, and selected road sprite IDs.

## Rendering

Rendering uses PixiJS and a custom isometric projection.

World coordinates remain simulation-friendly. The renderer maps them into isometric screen coordinates. Rendering must not mutate city model state.

Layer order:

1. Terrain
2. Water and river banks
3. Roads and bridges
4. Parcels, plazas, and parks
5. Buildings
6. Trees and small details
7. Optional debug overlays

Depth sorting must handle buildings and taller objects correctly. The first viewport should show only the beautiful playfield: no visible menus, panels, buttons, onboarding text, or persistent HUD.

Camera requirements:

- Smooth continuous zoom with mouse wheel and trackpad.
- Smooth panning/scrolling by drag or trackpad.
- Sensible minimum and maximum zoom.
- Stable camera behavior without jitter.
- Optional developer overlays only through keyboard toggles.

## User Interface

The default experience has no visible interface. The screen is the game world.

Developer-only controls can exist behind keyboard shortcuts:

- road graph overlay,
- parcel overlay,
- district overlay,
- tile/sprite ID overlay,
- performance HUD.

These overlays are off by default and must not be presented as in-world UI.

## Future Simulation Fit

MVP 1 does not simulate residents or vehicles, but it must avoid choices that would block them.

Future-compatible constraints:

- City data structures must be serializable.
- Generator output should be reproducible from seed plus generator version.
- Render state must remain separate from simulation state.
- Roads must be navigable graph data, not only tile coordinates.
- Parcels and buildings must expose future capacity and role hints.
- Districts and centers must expose centrality and density metadata.
- The model must be independent of browser-only APIs where practical.

Supabase and Vercel are later platform targets. MVP 1 should not implement them, but the model should be designed so that persistence and server-side simulation can be added without replacing the city representation.

## Testing And Validation

The MVP should include focused tests and debug checks:

- Coordinate transform tests: world/grid to isometric screen coordinates.
- Determinism tests: same seed and generator version produce the same structure.
- Road graph checks: no broken edges, invalid node references, or impossible bridges.
- Road sprite validation: graph-derived orientation matches selected sprite category.
- Parcel validation: each buildable parcel has road access.
- Render smoke test: app loads, scene is non-empty, camera pans and zooms, no console errors.
- Performance visibility: developer HUD or measurable render stats available behind a toggle.

## Explicit Non-Goals

Not included in MVP 1:

- resident simulation,
- vehicle simulation,
- multiplayer,
- Supabase schema or Realtime implementation,
- Vercel deployment automation,
- authentication,
- economy or gameplay mechanics,
- custom production art,
- visible user interface,
- procedural generation UI.

## Acceptance Criteria

The first implementation is successful when:

- The app displays a beautiful isometric river city using OpenGFX2 Classic 64px assets.
- The visible city is generated from a structured model, not hand-painted as static sprites.
- The road network is represented as a navigable graph with correct road orientations.
- Road sprites match the graph-derived direction and connectivity.
- The default screen shows only the city view.
- Continuous zoom and panning work smoothly.
- Debug overlays can show road graph, parcels, districts, and sprite/tile metadata.
- The default seed produces a deterministic city.
- Basic tests validate transforms, generation determinism, road graph integrity, parcel access, and render smoke behavior.

## References

- OpenGFX2 repository: https://github.com/OpenTTD/OpenGFX2
- Hillier et al., "Natural Movement": https://discovery.ucl.ac.uk/1398/
- Chen et al., "Interactive Procedural Street Modeling": https://asu.elsevierpure.com/en/publications/interactive-procedural-street-modeling/
- Vanegas et al., "Procedural Generation of Parcels in Urban Modeling": https://twak.org/project/parcels/
- Bettencourt et al., "Growth, innovation, scaling, and the pace of life in cities": https://pubmed.ncbi.nlm.nih.gov/17438298/
