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

export interface ExpectedAgentCheck {
  expectedAgents: number;
  runtimeAgents: number;
  persistedAgents: number;
}

interface BaseWorldSpawns {
  pedestrian_groups?: Array<{ agents_per_corridor?: unknown }>;
  car_groups?: Array<{ cars_per_arterial?: unknown }>;
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

export function expectedConcreteAgentsFromSpawns(spawns: unknown): number {
  if (!spawns || typeof spawns !== 'object') {
    throw new Error('base-world spawns must be an object');
  }

  const typed = spawns as BaseWorldSpawns;
  const pedestrianAgents = sumNonNegativeIntegerField(
    typed.pedestrian_groups,
    'agents_per_corridor',
    'pedestrian_groups',
  );
  const driverAgents = sumNonNegativeIntegerField(typed.car_groups, 'cars_per_arterial', 'car_groups');

  return pedestrianAgents + driverAgents;
}

export function assertRuntimeAndPersistedAgentsMeetExpectation(check: ExpectedAgentCheck): void {
  if (!Number.isInteger(check.expectedAgents) || check.expectedAgents < 0) {
    throw new Error(`invalid expected agent count ${check.expectedAgents}`);
  }
  if (check.runtimeAgents < check.expectedAgents) {
    throw new Error(`runtime mobility has ${check.runtimeAgents} agents, expected at least ${check.expectedAgents}`);
  }
  if (check.persistedAgents < check.expectedAgents) {
    throw new Error(`persisted mobility has ${check.persistedAgents} agents, expected at least ${check.expectedAgents}`);
  }
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

function sumNonNegativeIntegerField(
  rows: Array<Record<string, unknown>> | undefined,
  field: string,
  groupName: string,
): number {
  if (rows === undefined) return 0;
  if (!Array.isArray(rows)) {
    throw new Error(`base-world spawns ${groupName} must be an array`);
  }

  return rows.reduce((sum, row, index) => {
    const value = row[field];
    if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) {
      throw new Error(`base-world spawns ${groupName}[${index}].${field} must be a non-negative integer`);
    }
    return sum + value;
  }, 0);
}
