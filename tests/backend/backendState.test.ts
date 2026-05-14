import { describe, expect, it } from 'vitest';
import {
  applyChunkSnapshot,
  applyHealth,
  applyServerMessage,
  applyWorldSummary,
  createInitialBackendOverlayState,
} from '../../src/backend/backendState';

describe('backend overlay state', () => {
  it('loads HTTP snapshot state without requiring websocket data', () => {
    let state = createInitialBackendOverlayState();

    state = applyHealth(state, {
      service: 'abutown-sim',
      world_id: 'abutown-main',
      ok: true,
      protocol_version: 1,
    });
    state = applyWorldSummary(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });
    state = applyChunkSnapshot(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      coord: { x: 4, y: 4 },
      chunk_state: 'active',
      chunk_version: 1,
      tile_count: 1024,
      dirty_tiles: [{ local_index: 0, kind: 'road', version: 1 }],
    });

    expect(state.status).toBe('snapshot');
    expect(state.worldId).toBe('abutown-main');
    expect(state.chunkSize).toBe(32);
    expect(state.loadedChunk?.coord).toEqual({ x: 4, y: 4 });
  });

  it('applies websocket tile pulses only when protocol and world match', () => {
    let state = createInitialBackendOverlayState();
    state = applyWorldSummary(state, {
      protocol_version: 1,
      world_id: 'abutown-main',
      chunk_size: 32,
      loaded_chunks: [{ x: 4, y: 4 }],
    });

    state = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'abutown-main',
        tick: 4,
        version: 9,
        coord: { x: 4, y: 4 },
        local_index: 99,
      },
      1200,
    );

    expect(state.status).toBe('live');
    expect(state.latestTick).toBe(4);
    expect(state.latestVersion).toBe(9);
    expect(state.pulses).toHaveLength(1);
    expect(state.pulses[0]).toMatchObject({ localIndex: 99, receivedAtMs: 1200 });

    const afterWrongWorld = applyServerMessage(
      state,
      {
        type: 'tile_pulse',
        protocol_version: 1,
        world_id: 'other-world',
        tick: 5,
        version: 10,
        coord: { x: 4, y: 4 },
        local_index: 100,
      },
      1300,
    );

    expect(afterWrongWorld.pulses).toHaveLength(1);
    expect(afterWrongWorld.warning).toBe('Ignored websocket message for other-world');
  });
});
