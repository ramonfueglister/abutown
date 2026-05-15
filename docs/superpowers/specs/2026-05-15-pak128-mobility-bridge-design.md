# Pak128 Mobility Bridge Design

## Goal

Run the existing backend mobility bridge on the Pak128 renderer so `http://127.0.0.1:5175/?mobility=1` shows the Pak128 world and the live demo mobility markers from the simulation backend.

## Scope

In scope:
- Reuse the existing mobility protocol, state reducer, backend client, and canvas overlay behavior from the merged mobility work.
- Keep Pak128 as the active renderer and asset pack.
- Keep mobility opt-in through `?mobility=1`, `?mobilityBackend=...`, or `localStorage["abutown:mobility"]="1"`.
- Preserve current uncommitted Pak128 visual tuning changes.
- Verify with unit tests, build, E2E smoke test, and a browser screenshot on the live `5175` server.

Out of scope:
- Changing the simulation backend model.
- Making the backend demo loop forever.
- Merging unrelated main-branch changes into the Pak128 worktree.

## Architecture

The frontend owns a small mobility bridge layer:
- `src/backend/mobilityProtocol.ts` validates snapshot and delta DTOs.
- `src/backend/mobilityState.ts` stores current server records and maps demo route ids to Pak128 world coordinates.
- `src/backend/mobilityClient.ts` fetches `/mobility`, subscribes to `/ws`, applies deltas, and reconnects after socket loss.
- `src/render/mobilityOverlay.ts` converts reducer markers into canvas draw items and draws the visible overlay.

`src/main.ts` remains the composition root. It detects whether mobility is enabled, starts the client bridge, exposes mobility diagnostics through `render_game_to_text`, and draws the overlay after the Pak128 scene so the agent marker remains visible.

## Data Flow

On boot, the Pak128 client loads assets and builds the world. If mobility is enabled, it connects to the backend, applies the initial `/mobility` snapshot, then applies `mobility_delta` messages from `/ws`. The render loop uses the latest immutable `mobilityState` and does not block on network activity.

The demo coordinate mapping is intentionally local and explicit. It maps backend demo ids to known visible Pak128 grid coordinates so the current seeded agent remains visible after the short backend demo has completed.

## Error Handling

Invalid websocket messages increment `invalidMessages` without dropping the last valid state. Failed snapshot or websocket connections mark the bridge disconnected with a readable `lastError`. Reconnects happen in the client bridge; the renderer continues to run with the latest known state.

## Testing

Tests must cover:
- DTO parsing and malformed server messages.
- Snapshot and delta reducer behavior.
- Completed agent coordinate visibility in the Pak128 startup view.
- Overlay draw item sizing and visibility filtering.
- E2E diagnostics showing Pak128 asset pack and disconnected default mobility when the opt-in flag is absent.

Browser verification must load `http://127.0.0.1:5175/?mobility=1` from the Pak128 worktree and confirm `assetPack.id === "simutrans-pak128"` plus connected mobility diagnostics.
