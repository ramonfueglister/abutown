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
