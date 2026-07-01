const DEFAULT_BACKEND_BASE_URL = 'http://127.0.0.1:8080';

/**
 * Resolve the backend base URL from the `VITE_ABUTOWN_BACKEND_URL` env value,
 * falling back to loopback for local dev.
 */
export function resolveBackendBaseUrl(envUrl?: unknown): string {
  return typeof envUrl === 'string' && envUrl.length > 0 ? envUrl : DEFAULT_BACKEND_BASE_URL;
}
