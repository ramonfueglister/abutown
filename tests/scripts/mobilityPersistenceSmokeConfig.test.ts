import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, test } from 'vitest';
import { createPgClientConfig } from '../../scripts/mobility-persistence-smoke-config';

let tempDir: string | null = null;

afterEach(() => {
  if (tempDir) {
    rmSync(tempDir, { recursive: true, force: true });
    tempDir = null;
  }
});

describe('createPgClientConfig', () => {
  test('requires verify-full and configures explicit CA verification', () => {
    tempDir = mkdtempSync(join(tmpdir(), 'abutown-smoke-'));
    const rootCertPath = join(tempDir, 'prod-supabase.cer');
    writeFileSync(rootCertPath, '-----BEGIN CERTIFICATE-----\ntest-ca\n-----END CERTIFICATE-----\n');

    const config = createPgClientConfig(
      'postgresql://user:pass@aws-0-eu-central-1.pooler.supabase.com:5432/postgres?sslmode=verify-full',
      { PGSSLROOTCERT: rootCertPath },
    );

    expect(config.connectionString).toBe(
      'postgresql://user:pass@aws-0-eu-central-1.pooler.supabase.com:5432/postgres',
    );
    expect(config.ssl).toEqual({
      ca: '-----BEGIN CERTIFICATE-----\ntest-ca\n-----END CERTIFICATE-----\n',
      rejectUnauthorized: true,
      servername: 'aws-0-eu-central-1.pooler.supabase.com',
    });
  });

  test('rejects weaker sslmode values instead of falling back', () => {
    expect(() =>
      createPgClientConfig('postgresql://user:pass@example.supabase.com/postgres?sslmode=require', {
        PGSSLROOTCERT: '/tmp/unused-cert.pem',
      }),
    ).toThrow('DATABASE_URL must use sslmode=verify-full');
  });
});
