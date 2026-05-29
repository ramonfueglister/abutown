# Pak128 Mobility Bridge Design

## Goal

Run mobility directly in the Pak128 game at `http://127.0.0.1:5175/` so the rendered pedestrians are available as local agents without a URL flag.

## Scope

In scope:
- Keep Pak128 as the active renderer and asset pack.
- Keep local pedestrian agents enabled by default in the game.
- Preserve current uncommitted Pak128 visual tuning changes.
- Verify with unit tests, build, E2E smoke test, and a browser screenshot on the live `5175` server.

Out of scope:
- Changing the simulation backend model.
- Making the backend demo loop forever.
- Merging unrelated main-branch changes into the Pak128 worktree.

## Architecture

The frontend owns a small local-agent layer:
- `src/render/pedestrianAgents.ts` projects rendered pedestrians into stable local agent records.
- `src/render/pedestrianAgentInspector.ts` formats selected-agent details.
- `src/main.ts` owns selection, hit testing, canvas feedback, and diagnostics.

`src/main.ts` remains the composition root. It exposes local pedestrian-agent diagnostics through `render_game_to_text` and draws the selected-agent feedback after the Pak128 scene.

## Data Flow

On boot, the Pak128 client loads assets, builds the world, creates pedestrians, and projects them into local agent records. The render loop updates pedestrian positions and keeps the selected-agent inspector in sync with the current projected agent state.

## Error Handling

No network path is required for the local pedestrian-agent layer. If the selected pedestrian disappears from the local projection, the inspector returns to the empty state and the renderer keeps running.

## Testing

Tests must cover:
- Local pedestrian agent projection.
- Click-hit selection.
- Selected-agent inspector payload.
- E2E diagnostics showing Pak128 asset pack and default local-pedestrian mobility.

Browser verification must load `http://127.0.0.1:5175/` from the Pak128 worktree and confirm `assetPack.id === "simutrans-pak128"` plus `mobility.status === "local-pedestrians"`.
