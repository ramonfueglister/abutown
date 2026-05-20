import { create, toBinary } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
  backendErrorMessage,
  isBackendHealthDto,
  requireBackend,
  resolveBackendBaseUrl,
} from '../../src/backend/backendGate';
import { HealthResponseSchema } from '../../src/backend/proto/abutown_pb';

function healthProtoResponse(payload: {
  service: string;
  world_id: string;
  ok: boolean;
  protocol_version: number;
}): Response {
  const message = create(HealthResponseSchema, {
    service: payload.service,
    worldId: payload.world_id,
    ok: payload.ok,
    protocolVersion: payload.protocol_version,
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
      world_id: 'abutown-main',
      ok: true,
      protocol_version: 1,
    })).toBe(true);
  });

  it('rejects invalid health payloads', () => {
    expect(isBackendHealthDto({
      service: 'abutown-sim',
      world_id: 'abutown-main',
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

  it('requires a valid healthy payload', async () => {
    await expect(requireBackend({
      fetchImpl: async () =>
        healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutown-main',
          ok: false,
          protocol_version: 1,
        }),
    })).rejects.toThrow('Invalid backend health payload');
  });

  it('returns the validated backend status', async () => {
    const status = await requireBackend({
      fetchImpl: async (input) => {
        expect(String(input)).toBe('http://127.0.0.1:8080/health');
        return healthProtoResponse({
          service: 'abutown-sim',
          world_id: 'abutown-main',
          ok: true,
          protocol_version: 1,
        });
      },
    });

    expect(status).toEqual({
      service: 'abutown-sim',
      world_id: 'abutown-main',
      ok: true,
      protocol_version: 1,
    });
  });

  it('normalizes unknown startup errors', () => {
    expect(backendErrorMessage(new Error('boom'))).toBe('boom');
    expect(backendErrorMessage('nope')).toBe('nope');
  });
});
