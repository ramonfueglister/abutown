export type BackendHealthDto = {
  service: 'abutown-sim';
  world_id: string;
  ok: true;
  protocol_version: number;
};

export type BackendGateOptions = {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
};

const DEFAULT_BACKEND_BASE_URL = 'http://127.0.0.1:8080';

export function resolveBackendBaseUrl(envUrl?: unknown): string {
  return typeof envUrl === 'string' && envUrl.length > 0 ? envUrl : DEFAULT_BACKEND_BASE_URL;
}

export function isBackendHealthDto(value: unknown): value is BackendHealthDto {
  if (!isObject(value)) return false;
  return (
    value.service === 'abutown-sim' &&
    isString(value.world_id) &&
    value.ok === true &&
    isNumber(value.protocol_version)
  );
}

export async function requireBackend(options: BackendGateOptions = {}): Promise<BackendHealthDto> {
  const baseUrl = options.baseUrl ?? resolveBackendBaseUrl();
  const fetchImpl = hasOption(options, 'fetchImpl') ? options.fetchImpl : globalThis.fetch?.bind(globalThis);
  if (!fetchImpl) throw new Error('Backend fetch transport unavailable');

  const response = await fetchImpl(new URL('/health', baseUrl).toString());
  if (!response.ok) throw new Error(`Backend health HTTP ${response.status}`);

  const payload: unknown = await response.json();
  if (!isBackendHealthDto(payload)) throw new Error('Invalid backend health payload');
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
