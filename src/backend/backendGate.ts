import { fromBinary } from '@bufbuild/protobuf';
import { HealthResponseSchema } from './proto/abutown_pb';
import { healthResponseFromProto, type BackendPersistenceHealthDto } from './mobilityProtocol';

export type BackendHealthResponseDto = {
  service: 'abutown-sim';
  world_id: string;
  ok: boolean;
  protocol_version: number;
  persistence?: BackendPersistenceHealthDto;
};

export type BackendHealthDto = BackendHealthResponseDto & { ok: true };

export type BackendGateOptions = {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
};

const DEFAULT_BACKEND_BASE_URL = 'http://127.0.0.1:8080';

export function resolveBackendBaseUrl(envUrl?: unknown): string {
  return typeof envUrl === 'string' && envUrl.length > 0 ? envUrl : DEFAULT_BACKEND_BASE_URL;
}

export function isBackendHealthResponseDto(value: unknown): value is BackendHealthResponseDto {
  if (!isObject(value)) return false;
  return (
    value.service === 'abutown-sim' &&
    isString(value.world_id) &&
    typeof value.ok === 'boolean' &&
    isNumber(value.protocol_version) &&
    (value.persistence === undefined || isBackendPersistenceHealthDto(value.persistence))
  );
}

export function isBackendHealthDto(value: unknown): value is BackendHealthDto {
  return (
    isBackendHealthResponseDto(value) &&
    value.ok === true &&
    isAcceptableBackendPersistenceHealth(value.persistence)
  );
}

export class BackendHealthError extends Error {
  constructor(readonly health: BackendHealthResponseDto) {
    super(formatBackendHealthError(health));
    this.name = 'BackendHealthError';
  }
}

export async function requireBackend(options: BackendGateOptions = {}): Promise<BackendHealthDto> {
  const baseUrl = options.baseUrl ?? resolveBackendBaseUrl();
  const fetchImpl = hasOption(options, 'fetchImpl') ? options.fetchImpl : globalThis.fetch?.bind(globalThis);
  if (!fetchImpl) throw new Error('Backend fetch transport unavailable');

  const response = await fetchImpl(new URL('/health', baseUrl).toString());
  if (!response.ok) throw new Error(`Backend health HTTP ${response.status}`);

  // Phase: binary wire — /health returns application/x-protobuf.
  const bytes = new Uint8Array(await response.arrayBuffer());
  const proto = fromBinary(HealthResponseSchema, bytes);
  const payload = healthResponseFromProto(proto);
  if (!isBackendHealthResponseDto(payload)) throw new Error('Invalid backend health payload');
  if (!isBackendHealthDto(payload)) throw new BackendHealthError(payload);
  return payload;
}

export function backendErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message.length > 0) return error.message;
  if (typeof error === 'string' && error.length > 0) return error;
  return 'Backend required';
}

function hasOption<T extends object, K extends PropertyKey>(value: T, key: K): value is T & Record<K, unknown> {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isString(value: unknown): value is string {
  return typeof value === 'string';
}

function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function formatBackendHealthError(health: BackendHealthResponseDto): string {
  const persistence = health.persistence;
  if (!persistence) return 'Backend health not OK';
  const base = `Backend health not OK: persistence ${persistence.status}`;
  return persistence.last_error ? `${base}: ${persistence.last_error}` : base;
}

function isAcceptableBackendPersistenceHealth(value: BackendPersistenceHealthDto | undefined): boolean {
  return value === undefined || value.status === 'starting' || value.status === 'healthy' || value.status === 'degraded';
}

function isBackendPersistenceHealthDto(value: unknown): value is BackendPersistenceHealthDto {
  if (!isObject(value)) return false;
  return (
    (value.status === 'starting' ||
      value.status === 'healthy' ||
      value.status === 'degraded' ||
      value.status === 'stale') &&
    isString(value.world_id) &&
    isNumber(value.mobility_tick) &&
    (value.last_attempt_unix_ms === null || isNumber(value.last_attempt_unix_ms)) &&
    (value.last_success_unix_ms === null || isNumber(value.last_success_unix_ms)) &&
    isNumber(value.consecutive_failures) &&
    (value.last_error === null || isString(value.last_error)) &&
    (value.freshness_ms === null || isNumber(value.freshness_ms))
  );
}
