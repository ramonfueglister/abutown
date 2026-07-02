# Abutopia Beautiful Minimal Metro City

Date: 2026-06-18
Status: Current product authority

## Goal

Make Abutown open on a beautiful minimal city-simulation screen, not a stale
fixture. The visual language is Mini-Metro-inspired: calm background, crisp
lines, named stations and quarters, visible moving agents, and readable economy
flows. The first viewport must look intentional without explanation.

## Non-Negotiables

- No visible retired fixture labels.
- No empty-field first screen.
- No hidden technical success in place of visible product success.
- Agents must be visible on first load and must move.
- Economy flows must read as city flows between meaningful places, not abstract
  debug lines.
- Debug HUD, auth state, and diagnostic UI must not dominate the first screen.
- Browser screenshot or smoke evidence is the acceptance gate for visual work.

## Product Shape

Abutopia is the canonical product world. It should be small enough to reason
about and rich enough to feel alive.

The first screen should show:

- 4 to 6 named places with city meaning.
- A compact network of clean lines between those places.
- Small building or landmark markers around the stations.
- Visible moving citizens and economy agents.
- At least two readable flow arcs or line movements.
- Enough whitespace to feel minimal, but not enough emptiness to read as
  unfinished.

Suggested place set:

- Central
- Homes
- Market
- Workshop
- Harbor
- Depot

Names can change during implementation, but they must be real city labels, not
test fixture labels.

## Data Direction

The renderer must draw authored world data, not fake frontend decoration.

Primary data/code targets:

- `data/worlds/abutopia/manifest.json`
- `data/worlds/abutopia/layers/markets.json`
- `data/worlds/abutopia/layers/transport.json`
- `data/worlds/abutopia/layers/buildings.json`
- `data/worlds/abutopia/layers/decorations.json`
- `data/worlds/abutopia/layers/spawns.json`
- `src/main.ts`
- `src/render/`
- `tests/`
- `scripts/smoke-schematic.mjs`

If the authored base-world data changes in a way that invalidates persisted
snapshots, bump the Abutopia schema version and clear only incompatible local
snapshots through the existing compatibility path.

## Agents

Agents are real simulation bodies. They should be visible because the world,
camera, subscription, and render projection line up, not because the frontend
draws fake people.

Expected behavior:

- Existing citizens appear near meaningful places on first load.
- Economy/trader agents are visually distinct enough to read as movement.
- Agents move along the authored network.
- The initial camera frames the active area where agents actually exist.
- Diagnostics may prove counts, but visual acceptance requires seeing movement.

## Economy

Keep the backend authoritative economy. The cleanup is to make the visible
economy legible and city-shaped.

Expected behavior:

- Markets have human city names.
- Flows connect real places.
- Goods movement is visible but not visually noisy.
- Labels explain places, not internal implementation fixtures.
- Existing backend economy mechanics remain authoritative unless a focused
  implementation task deliberately changes them.

## Renderer

Keep the schematic Canvas2D direction. Improve the first impression by making
the actual city data read clearly.

Expected behavior:

- Calm neutral map background.
- Crisp route lines.
- Small high-contrast station markers.
- Sparse labels with stable placement.
- Visible agents with readable motion.
- Minimal HUD footprint.
- No marketing page, no decorative hero, no fake overlay city.

## Acceptance Gates

A change that claims this spec must provide evidence for all relevant gates:

- `rg` finds no user-visible retired fixture labels in Abutopia data or render
  strings.
- Browser screenshot shows a composed minimal city view on first load.
- Browser smoke shows no console errors.
- At least 4 named places are visible or inspectable.
- Agents are visible and moving on first load.
- At least 2 economy or transit flows are visible.
- The camera opens on the live part of Abutopia.
- If frontend/backend/render wiring changes, run the mandatory browser smoke
  for real WebSocket and subscription behavior.

## Not Accepted

- Reintroducing deleted historical specs as active authority.
- Returning to old Zurich/OpenGFX/pak128 threads.
- Shipping visible retired fixture labels.
- Treating green unit tests as visual acceptance.
- Drawing fake frontend agents to hide backend/render alignment bugs.
- Adding broad new simulation theory before the first screen is visibly right.
