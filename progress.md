Original prompt: OpenTTD-like isometric persistent webgame graphics demo with realistic city, OpenGFX2 assets, no visible UI.


2026-05-14T08:04:30.446Z - Visual QA: replaced hand-drawn road strokes with OpenGFX2 road sprites, moved central river to a small edge creek, reduced road-grid density, increased building scale, and kept verification on port 5175 only.
2026-05-14T14:11:40.000Z - Camera UX: replaced map wrapping with a bounded fixed-map camera, added damped pan/zoom targets, added outskirts/edge-exit/mist rendering, and restored Vite preview support for Playwright smoke tests.
2026-05-14T14:25:20.000Z - Vehicle QA: fixed road/vehicle draw ordering so road tiles cannot overpaint moving vehicles, reduced right-lane offset to keep vehicles on the lane instead of the shoulder, and verified on isolated port 5176.
2026-05-14T14:35:10.000Z - Vehicle geometry: derived right-lane offset from OpenGFX road surface width, reduced vehicle scale, filtered out incomplete 8-direction vehicle sprites, and converted open vehicle corridors into ping-pong loops to avoid visual despawns.
