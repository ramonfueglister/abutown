# Minimal Motorways Renderer Design

Date: 2026-05-26

## Status

Approved by user after web research. This replaces the default pak128/isometric visual presentation with an ultraminimal Mini Motorways-inspired vector map while preserving the existing backend-authoritative mobility model.

## Goal

Make Abutown look like a calm, beautiful, diagrammatic city map: roads, water, parks, buildings, cars, agents, rail, and trains remain visible, but sprites, texture detail, isometric tile art, night modes, theme modes, and decorative UI are removed from the primary renderer.

## Research Basis

Mini Motorways is the primary reference: a growing city expressed through map surfaces, clean roads, small moving vehicles, and restrained depth cues. Mini Metro contributes transit-map clarity: colored rail lines, simple nodes, and route-first readability. Cartographic hierarchy guidance supports the same direction: important layers should be visually prominent, secondary layers should recede, and line weights should encode hierarchy.

RimWorld is not a visual target. Its relevance is limited to simulation readability: individual actors must stay selectable and inspectable.

## Scope

In scope:

- Replace the default visible renderer with a vector, top-down map style.
- Keep backend-required boot, backend mobility state, interpolation, selection, and inspectors.
- Keep the existing city generation, Zurich world, road/rail/building placement, and backend protocol.
- Draw roads, bridges, rail, train, cars, pedestrians, parks, water, buildings, trees, and sparse details using canvas primitives.
- Switch projection from isometric tile pixels to a flat map projection with a reversible screen-to-tile transform for viewport subscriptions and hit testing.
- Update render smoke expectations to assert the new style and canvas nonblank behavior.

Out of scope:

- Night mode, color themes, palette picker, creative/edit mode, minimap, labels, new UI, route editing, backend changes, asset imports, WebGL, PixiJS, and new simulation systems.

## Visual Direction

Use a warm light map background. Water is a soft blue surface. Parks and forests are quiet green shapes. Roads are calm, rounded strokes with a subtle casing and a clear bridge treatment. Rail is a stronger colored transit line with small train capsules. Buildings are tiny abstract blocks, colored by deterministic district/category choices. Cars are small moving capsules. Pedestrians are smaller moving dots. Selection rings remain, but use the same restrained graphic language.

The first read should be mobility and road structure. The second read should be city mass and terrain. Decorative detail should be almost invisible.

## Architecture

Add a small projection helper for flat map coordinates. `src/main.ts` keeps the existing runtime and data wiring but routes all draw functions through vector canvas primitives instead of pak128 image frames. The old asset catalog can remain present for sprite-key compatibility and tests, but runtime image loading is no longer required for the primary visual style.

The backend viewport contract remains intact because `screenToWorld -> worldToGrid` continues to return backend tile coordinates. Only the visual projection changes.

## Testing

Unit tests cover flat map projection round-tripping. Existing backend mobility projection tests remain valid because they operate on tile coordinates and sprite-key catalogs. The browser smoke test is updated to assert:

- `visualStyle.id === "minimal-motorways"`,
- top-down coordinate-system metadata,
- vector tile size instead of pak128 tile size,
- backend mobility counts still match rendered cars/agents,
- selection still works for agents and vehicles,
- canvas is not near-black and has meaningful colored pixels.

## Acceptance Criteria

- The app boots with the backend and renders a light, vector, Mini Motorways-inspired map by default.
- No pak128 sprites are drawn in the main scene.
- Cars and agents come from backend mobility state and remain animated/selectable.
- Camera pan/zoom, viewport chunk subscription, and render smoke still work.
- No extra visual modes or UI controls are introduced.
