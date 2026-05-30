import { readFileSync } from 'node:fs';

export interface MobilityPersistenceSmokeEnv {
  PGSSLROOTCERT?: string;
  DATABASE_SSL_CA_FILE?: string;
}

export interface PgClientConfig {
  connectionString: string;
  ssl: {
    ca: string;
    rejectUnauthorized: true;
    servername: string;
  };
}

export interface PersistedSnapshotCheck {
  healthPersistenceTick: number | null | undefined;
  mobilityTick: number;
  persistedTick: number;
  persistedUpdatedAgeMs: number;
  snapshotFreshnessMs: number;
}

export function createPgClientConfig(
  connectionString: string,
  env: MobilityPersistenceSmokeEnv = process.env,
): PgClientConfig {
  const url = parsePostgresUrl(connectionString);
  const sslMode = url.searchParams.get('sslmode');
  if (sslMode !== 'verify-full') {
    throw new Error('DATABASE_URL must use sslmode=verify-full for mobility persistence smoke');
  }

  const rootCertPath = url.searchParams.get('sslrootcert') ?? env.PGSSLROOTCERT ?? env.DATABASE_SSL_CA_FILE;
  if (!rootCertPath) {
    throw new Error('PGSSLROOTCERT or DATABASE_SSL_CA_FILE must point to the Supabase server root certificate');
  }

  const ca = readRootCertificate(rootCertPath);
  url.searchParams.delete('sslmode');
  url.searchParams.delete('sslrootcert');

  return {
    connectionString: url.toString(),
    ssl: {
      ca,
      rejectUnauthorized: true,
      servername: url.hostname,
    },
  };
}

export function countPersistedAgents(payload: unknown): number {
  if (!payload || typeof payload !== 'object') return -1;

  const agents = (payload as { agents?: unknown }).agents;
  if (Array.isArray(agents)) return agents.length;
  if (agents && typeof agents === 'object') return Object.keys(agents).length;

  return -1;
}

export function assertPersistedSnapshotMatchesHealth(check: PersistedSnapshotCheck): void {
  if (!Number.isFinite(check.persistedTick) || check.persistedTick < 0) {
    throw new Error(`invalid persisted tick ${check.persistedTick}`);
  }
  if (!Number.isFinite(check.healthPersistenceTick)) {
    throw new Error(`invalid health persistence tick ${check.healthPersistenceTick}`);
  }
  if (check.persistedTick !== check.healthPersistenceTick) {
    throw new Error(
      `persisted tick ${check.persistedTick} differs from health persistence tick ${check.healthPersistenceTick}`,
    );
  }
  if (check.mobilityTick < check.persistedTick) {
    throw new Error(`mobility tick ${check.mobilityTick} is behind persisted tick ${check.persistedTick}`);
  }
  if (
    !Number.isFinite(check.persistedUpdatedAgeMs) ||
    check.persistedUpdatedAgeMs < 0 ||
    check.persistedUpdatedAgeMs > check.snapshotFreshnessMs
  ) {
    throw new Error(
      `mobility_snapshots.updated_at age ${check.persistedUpdatedAgeMs}ms outside 0..${check.snapshotFreshnessMs}ms`,
    );
  }
}

function parsePostgresUrl(connectionString: string): URL {
  let url: URL;
  try {
    url = new URL(connectionString);
  } catch {
    throw new Error('DATABASE_URL must be a valid Postgres connection URL');
  }

  if (url.protocol !== 'postgres:' && url.protocol !== 'postgresql:') {
    throw new Error('DATABASE_URL must use postgres:// or postgresql://');
  }
  if (!url.hostname) {
    throw new Error('DATABASE_URL must include a database host');
  }

  return url;
}

function readRootCertificate(path: string): string {
  try {
    return readFileSync(path, 'utf8');
  } catch {
    throw new Error('Supabase server root certificate could not be read from PGSSLROOTCERT/DATABASE_SSL_CA_FILE');
  }
}
