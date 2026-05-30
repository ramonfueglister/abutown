import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, test } from 'vitest';
import {
  assertPersistedSnapshotMatchesHealth,
  assertRuntimeAndPersistedAgentsMeetExpectation,
  countPersistedAgents,
  createPgClientConfig,
  expectedConcreteAgentsFromSpawns,
} from '../../scripts/mobility-persistence-smoke-config';

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

describe('mobility persistence smoke checks', () => {
  test('derives expected concrete agents from base-world spawns', () => {
    expect(
      expectedConcreteAgentsFromSpawns({
        pedestrian_groups: [
          { id: 'spawn:ped:1', corridor_id: 'corridor:1', agents_per_corridor: 2 },
          { id: 'spawn:ped:2', corridor_id: 'corridor:2', agents_per_corridor: 3 },
        ],
        car_groups: [{ id: 'spawn:car:1', arterial_id: 'arterial:1', cars_per_arterial: 4 }],
      }),
    ).toBe(9);
  });

  test('rejects runtime and persisted mobility below the expected base-world agents', () => {
    expect(() =>
      assertRuntimeAndPersistedAgentsMeetExpectation({
        expectedAgents: 1,
        runtimeAgents: 0,
        persistedAgents: 0,
      }),
    ).toThrow('runtime mobility has 0 agents, expected at least 1');

    expect(() =>
      assertRuntimeAndPersistedAgentsMeetExpectation({
        expectedAgents: 1,
        runtimeAgents: 1,
        persistedAgents: 0,
      }),
    ).toThrow('persisted mobility has 0 agents, expected at least 1');
  });

  test('counts persisted agents from the map-shaped mobility payload', () => {
    expect(
      countPersistedAgents({
        agents: {
          agent_1: { id: 'agent_1' },
          agent_2: { id: 'agent_2' },
        },
      }),
    ).toBe(2);
  });

  test('matches persisted tick to health persistence tick instead of live mobility tick', () => {
    expect(() =>
      assertPersistedSnapshotMatchesHealth({
        healthPersistenceTick: 100,
        mobilityTick: 108,
        persistedTick: 100,
        persistedUpdatedAgeMs: 500,
        snapshotFreshnessMs: 15_000,
      }),
    ).not.toThrow();
  });
});
