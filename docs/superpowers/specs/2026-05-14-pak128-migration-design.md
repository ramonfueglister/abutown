# Simutrans pak128 Migration Design

Date: 2026-05-14

## Status

Approved for specification. This document defines the design for migrating Abutown from OpenGFX/OpenTTD-style assets to Simutrans pak128 as the primary visual asset set. It does not authorize implementation yet; implementation planning follows after user review.

## Goal

Move Abutown to a complete Simutrans pak128 visual style without replacing the city model or simulation-oriented world data. The migration should make pak128 the primary asset pack for terrain, roads, rail, buildings, vegetation, details, pedestrians, vehicles, and trains.

The migration must avoid a brittle one-off sprite swap. The renderer should stop depending directly on OpenGFX file names, 64px tile assumptions, sheet-specific frame offsets, and hard-coded sprite dimensions. Instead, rendering should consume semantic asset roles through an asset-pack abstraction.

## Context

Abutown currently has an OpenGFX/OpenTTD-centered renderer. The main runtime uses 64px isometric tiles, OpenGFX file paths, and fixed draw constants for terrain, road, rail, buildings, vehicles, and details. The original graphics demo specification selected OpenGFX2 Classic 64px as the first visual target.

Simutrans pak128 is already present in a narrow form for pedestrian sprites. That integration proves the project can load pak128 PNG sheets, clean cyan/white transparent source pixels, map eight movement directions, and render 128px source frames in the existing canvas renderer.

The complete migration expands that narrow precedent into a first-class asset-pack pipeline.

## Chosen Approach

Use a pak128-native asset-pack migration in controlled slices.

The first implementation phase should introduce an asset-pack contract and pak128 catalog, then port the renderer to semantic asset lookups before broad visual replacement. OpenGFX can remain temporarily as an internal fallback during the transition, but it must not remain part of the final primary style.

This approach is preferred over direct replacement because the current renderer contains many OpenGFX-specific constants:

- 64x32 tile drawing dimensions.
- 42px OpenGFX terrain sprite height.
- OpenGFX road and rail frame steps.
- OpenGFX sheet names and frame grids.
- Hard-coded draw offsets for individual OpenGFX categories.
- Category-specific code paths tied to OpenGFX files.

Replacing paths alone would preserve those assumptions and produce fragile layout, draw-order, camera, and scaling problems.

## Alternatives Considered

### Direct Sprite Swap

Replace OpenGFX paths and constants in the current renderer with pak128 equivalents.

This would create a faster visual prototype, but it would encode pak128 assumptions into the same places where OpenGFX assumptions exist today. It would also make later fixes more expensive because every category would need manual recalibration.

### Hybrid OpenGFX And pak128

Use pak128 for major categories and keep OpenGFX for missing assets.

This would reduce short-term gaps, but it would weaken visual coherence. It is acceptable only as a temporary migration aid, not as a finished state.

### pak128-Native Asset Pack

Introduce an explicit asset-pack layer and migrate render categories through that layer.

This is the recommended path. It requires more initial structure, but it gives the project a stable way to load pak128 metadata, verify frames, tune anchors, and remove OpenGFX assumptions without rewriting the city model.

## Asset-Pack Contract

Create an asset-pack abstraction that describes render assets semantically. The renderer should request assets by role, not by file path.

Example semantic roles:

- `terrain.grass`
- `terrain.water`
- `terrain.riverbank`
- `road.straight`
- `road.curve`
- `road.intersection`
- `rail.straight`
- `rail.station`
- `building.residential.low`
- `building.commercial.mid`
- `building.civic`
- `vegetation.tree`
- `detail.park`
- `vehicle.bus`
- `vehicle.truck`
- `vehicle.train.engine`
- `agent.pedestrian`

Each resolved asset should carry render metadata:

- source image path,
- source rectangle,
- logical category,
- direction or variant metadata,
- source tile size,
- destination footprint,
- anchor point,
- y-sort baseline,
- default scale,
- transparency cleanup policy,
- license/provenance reference.

The renderer can still use category-specific drawing functions where useful, but those functions should consume this metadata instead of hard-coded OpenGFX sheet knowledge.

## pak128 Import And Catalog

The pak128 source should be imported into `public/simutrans-assets/pak128` or a similarly explicit location. The importer should preserve enough source structure and metadata to audit provenance.

The catalog should be generated from checked-in source assets and metadata where practical. For the first migration slice, manual curation is acceptable for a small set of known-good objects, but the catalog format should not depend on hand-coded draw functions.

Minimum catalog responsibilities:

- map pak128 object classes to Abutown semantic roles,
- expose frame rectangles for directional sprites,
- identify source PNG and `.dat` provenance,
- record license and copyright declarations,
- skip incomplete, mask-only, metadata-only, or unsuitable source frames,
- provide deterministic variant selection.

The project should keep license documentation close to imported pak128 sources. pak128 is distributed under Artistic License 2.0 according to the Simutrans project documentation and the existing local pedestrian import notes. The migration must retain attribution and source revision information.

## Renderer Changes

The renderer should be reworked in small stages:

1. Extract OpenGFX-specific constants into an OpenGFX asset-pack descriptor.
2. Add a pak128 asset-pack descriptor with native tile and sprite metadata.
3. Update terrain, road, rail, building, tree/detail, vehicle, train, and pedestrian draw functions to resolve semantic assets.
4. Tune camera defaults, min/max zoom, viewport padding, and culling for pak128 scale.
5. Remove OpenGFX fallback once pak128 covers the required visible categories.

The city model, road topology, Zurich placement, pedestrian corridors, and movement systems should remain mostly unchanged. They already operate on simulation-friendly grid coordinates and semantic categories. The renderer is the primary migration target.

## Visual And Gameplay Impact

pak128 is larger and more detailed than the current OpenGFX setup. The visible area at the same zoom will shrink unless camera defaults are adjusted. The result should feel more detailed and more cohesive, but it may also feel denser and visually busier.

Expected effects:

- fewer city blocks visible at default zoom,
- larger buildings and vehicles relative to streets,
- stronger need for accurate anchors and y-sort baselines,
- more visible overlap errors if draw order is wrong,
- likely camera recalibration for first-load composition,
- possible reduction in apparent city scale unless map framing changes.

The migration should prioritize coherence over showing the same amount of city at once. If the first viewport becomes too crowded, tune default zoom and city framing rather than shrinking pak128 sprites unnaturally.

## Performance Impact

pak128 source frames are larger than the current 64px OpenGFX tiles. Canvas rendering should remain viable, but the renderer must rely on viewport culling, deterministic sprite selection, and minimal per-frame image work.

Performance risks:

- larger texture memory footprint,
- larger draw rectangles,
- more expensive alpha-cleaned canvas sources,
- more overlap requiring additional draw calls,
- slower smoke tests if every source sheet is eagerly loaded.

Mitigations:

- keep image loading catalog-driven,
- load only assets used by the current scene,
- cache cleaned images once,
- preserve visible-grid culling,
- keep smoke-test scenes deterministic.

## Testing And Validation

The migration should add focused tests before broad replacement.

Required tests:

- pak128 catalog includes expected semantic roles,
- semantic roles resolve to valid source rectangles,
- directional pedestrian and vehicle frames map to expected directions,
- missing role lookups fail clearly or use an explicit temporary fallback,
- sprite cleanup handles pak128 transparent source colors,
- draw-order comparison still renders roads, rails, buildings, agents, and trains in stable order,
- render smoke test confirms the pak128 scene is non-empty and free of browser console errors.

Visual QA remains necessary after the technical tests because anchor tuning and scale correctness are perceptual.

## Acceptance Criteria

The migration is successful when:

- The default scene uses Simutrans pak128 as the primary visual style.
- OpenGFX is no longer required for terrain, roads, rail, buildings, vegetation, visible details, pedestrians, road vehicles, or trains.
- The renderer resolves semantic assets through an asset-pack layer.
- Camera framing, zoom, and culling are calibrated for pak128 scale.
- Existing city generation, transport topology, placement, and movement systems continue to work.
- License and source provenance for pak128 assets are documented in the repository.
- Automated tests cover catalog lookup, frame metadata, cleanup, draw order, and render smoke behavior.

## Explicit Non-Goals

This migration does not include:

- changing the city generator,
- replacing Zurich world data,
- adding new gameplay systems,
- implementing Simutrans economic rules,
- supporting multiple selectable paksets in the user interface,
- converting the renderer to WebGL or PixiJS,
- shipping a mixed OpenGFX/pak128 final style.

## Implementation Planning Decisions

- Pin a specific pak128 source revision before importing additional assets. The first candidate is the revision already documented by the local pedestrian import, unless implementation discovers missing required assets in that revision.
- Import curated source files for the first slice rather than the full pak128 repository. Expand the imported set only as semantic roles require it.
- Map buildings through Abutown roles first, then pak128 object names: residential low/mid, commercial mid, civic, industrial, station, and landmark.
- Keep any OpenGFX fallback explicit, tested, and temporary. The implementation plan should include a removal step once pak128 covers required visible categories.
- Split `src/main.ts` only as much as needed to isolate asset-pack lookup, catalog metadata, and draw helpers. Avoid a broad renderer rewrite before the asset migration proves itself.
