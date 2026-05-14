Original prompt: OpenTTD-like isometric persistent webgame graphics demo with realistic city, OpenGFX2 assets, no visible UI.


2026-05-14T08:04:30.446Z - Visual QA: replaced hand-drawn road strokes with OpenGFX2 road sprites, moved central river to a small edge creek, reduced road-grid density, increased building scale, and kept verification on port 5175 only.
2026-05-14T14:11:40.000Z - Camera UX: replaced map wrapping with a bounded fixed-map camera, added damped pan/zoom targets, added outskirts/edge-exit/mist rendering, and restored Vite preview support for Playwright smoke tests.
2026-05-14T14:25:20.000Z - Vehicle QA: fixed road/vehicle draw ordering so road tiles cannot overpaint moving vehicles, reduced right-lane offset to keep vehicles on the lane instead of the shoulder, and verified on isolated port 5176.
2026-05-14T14:35:10.000Z - Vehicle geometry: derived right-lane offset from OpenGFX road surface width, reduced vehicle scale, filtered out incomplete 8-direction vehicle sprites, and converted open vehicle corridors into ping-pong loops to avoid visual despawns.
2026-05-14T14:43:30.000Z - Vehicle routing: built car loops from final road tiles only, split paths at grass/non-road and rail tiles, excluded all rail tiles from vehicle routes, and added render diagnostics for off-road, rail, and teleporting vehicle paths.
2026-05-14T14:47:45.000Z - Vehicle lane fit: inset top-to-bottom screen travel by 0.75px from the right-lane offset so vehicles no longer graze the road edge while preserving other travel directions.
2026-05-14T14:50:45.000Z - Vehicle lane fit: made the 0.75px inset symmetric for all isometric screen-vertical travel, including bottom-to-top up-left and up-right movement, while preserving the right-hand normal.
2026-05-14T14:55:05.000Z - Vehicle lane fit: corrected the inset to apply only to screen down-left travel; east/down-right, west/up-left, and north/up-right now keep the full right-lane offset and are regression-tested together.
2026-05-14T15:07:05.000Z - Vehicle motion: researched SUMO/Pure-Pursuit style separation of lane geometry, turn speed, and rendering; added a render-only vehicle motion profile with 90-degree curve easing, diagonal sprite frames, and curve/junction speed factors that can later be replaced by Rust ECS state.
