import { create, toBinary } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
  backendErrorMessage,
  isBackendHealthDto,
  requireBackend,
  resolveBackendBaseUrl,
} from '../../src/backend/backendGate';
import {
  HealthResponseSchema,
  PersistenceHealthStatus,
} from '../../src/backend/proto/abutown_pb';

function healthProtoResponse(payload: {
  service: string;
  world_id: string;
  ok: boolean;
  protocol_version: number;
  persistence?: {
    status: PersistenceHealthStatus;
    world_id?: string;
    mobility_tick?: number;
    last_attempt_unix_ms?: number;
    last_success_unix_ms?: number;
    consecutive_failures?: number;
    last_error?: string;
    freshness_ms?: number;
  };
}): Response {
  const message = create(HealthResponseSchema, {
    service: payload.service,
    worldId: payload.world_id,
    ok: payload.ok,
    protocolVersion: payload.protocol_version,
    persistence: payload.persistence
      ? {
          status: payload.persistence.status,
          worldId: payload.persistence.world_id ?? payload.world_id,
          mobilityTick: BigInt(payload.persistence.mobility_tick ?? 42),
          lastAttemptUnixMs: BigInt(payload.persistence.last_attempt_unix_ms ?? 0),
          lastSuccessUnixMs: BigInt(payload.persistence.last_success_unix_ms ?? 0),
          consecutiveFailures:
            payload.persistence.consecutive_failures ??
            (payload.persistence.status === PersistenceHealthStatus.HEALTHY ? 0 : 1),
          lastError: payload.persistence.last_error ?? '',
          freshnessMs: BigInt(payload.persistence.freshness_ms ?? 5_000),
        }
      : undefined,
  });
  return new Response(toBinary(HealthResponseSchema, message), {
    status: 200,
    headers: { 'content-type': 'application/x-protobuf' },
  });
}

describe('backend startup gate', () => {
  it('uses the live local backend by default', () => {
    expect(resolveBackendBaseUrl()).toBe('http://127.0.0.1:8080');
  });

  it('allows an explicit backend URL override', () => {
    expect(resolveBackendBaseUrl('https://backend.example.test')).toBe('https://backend.example.test');
  });

  it('accepts a healthy backend response', () => {
    expect(isBackendHealthDto({
      service: 'abutown-sim',
      world_id: 'abutopia',
      ok: true,
      protocol_version: 1,
    })).toBe(true);
  });

  it('rejects invalid health payloads', () => {
    expect(isBackendHealthDto({
      service: 'abutown-sim',
      world_id: 'abutopia',
      ok: false,
      protocol_version: 1,
    })).toBe(false);
    expect(isBackendHealthDto({ ok: true })).toBe(false);
  });

  it('requires fetch transport', async () => {
    await expect(requireBackend({ fetchImpl: undefined })).rejects.toThrow('Backend fetch transport unavailable');
  });

  it('requires HTTP success from health', async () => {
    await expect(requireBackend({
      fetchImpl: async () => new Response('{}', { status: 503 }),
    })).rejects.toThrow('Backend health HTTP 503');
  });

  it('fails closed when backend health is not OK without details', async () => {
    await expect(requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: false,
          protocol_version: 1,
        }),
    })).rejects.toThrow('Backend health not OK');
  });

  it('returns the validated backend status', async () => {
    const status = await requireBackend({
      fetchImpl: async (input) => {
        expect(String(input)).toBe('http://127.0.0.1:8080/health');
        return healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: true,
          protocol_version: 1,
        });
      },
    });

    expect(status).toEqual({
      service: 'abutown-sim',
      world_id: 'abutopia',
      ok: true,
      protocol_version: 1,
    });
  });

  it('returns healthy persistence details from health', async () => {
    const status = await requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: true,
          protocol_version: 1,
          persistence: {
            status: PersistenceHealthStatus.HEALTHY,
            world_id: 'abutopia',
            mobility_tick: 42,
            last_attempt_unix_ms: 1_712_000_000_100,
            last_success_unix_ms: 1_712_000_000_050,
            consecutive_failures: 0,
            last_error: '',
            freshness_ms: 5_000,
          },
        }),
    });

    expect(status.persistence).toEqual({
      status: 'healthy',
      world_id: 'abutopia',
      mobility_tick: 42,
      last_attempt_unix_ms: 1_712_000_000_100,
      last_success_unix_ms: 1_712_000_000_050,
      consecutive_failures: 0,
      last_error: null,
      freshness_ms: 5_000,
    });
  });

  it('returns starting persistence details from health', async () => {
    const status = await requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: true,
          protocol_version: 1,
          persistence: { status: PersistenceHealthStatus.STARTING },
        }),
    });

    expect(status.persistence?.status).toBe('starting');
  });

  it('accepts degraded persistence (non-blocking banner, map stays rendered)', async () => {
    const status = await requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: true,
          protocol_version: 1,
          persistence: {
            status: PersistenceHealthStatus.DEGRADED,
            last_error: 'mobility write failed',
          },
        }),
    });
    expect(status.persistence?.status).toBe('degraded');
  });

  it('accepts degraded persistence with error details (map stays rendered, banner shown)', async () => {
    const status = await requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: true,
          protocol_version: 1,
          persistence: {
            status: PersistenceHealthStatus.DEGRADED,
            consecutive_failures: 3,
            last_error: 'transient db error',
          },
        }),
    });
    expect(status.persistence?.status).toBe('degraded');
    expect(status.persistence?.consecutive_failures).toBe(3);
  });

  it('fails closed when persistence is stale', async () => {
    await expect(requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: false,
          protocol_version: 1,
          persistence: { status: PersistenceHealthStatus.STALE },
        }),
    })).rejects.toThrow('Backend health not OK: persistence stale');
  });

  it('fails closed when ok=true health reports stale persistence', async () => {
    await expect(requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutopia',
          ok: true,
          protocol_version: 1,
          persistence: { status: PersistenceHealthStatus.STALE },
        }),
    })).rejects.toThrow('Backend health not OK: persistence stale');
  });

  it('normalizes unknown startup errors', () => {
    expect(backendErrorMessage(new Error('boom'))).toBe('boom');
    expect(backendErrorMessage('nope')).toBe('nope');
  });
});
