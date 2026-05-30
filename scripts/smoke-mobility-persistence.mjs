#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fromBinary } from '@bufbuild/protobuf';
import pg from 'pg';
import {
  HealthResponseSchema,
  MobilitySnapshotSchema,
} from '../src/backend/proto/abutown_pb.ts';
import {
  healthResponseFromProto,
  mobilitySnapshotFromProto,
} from '../src/backend/mobilityProtocol.ts';
import {
  assertPersistedSnapshotMatchesHealth,
  assertRuntimeAndPersistedAgentsMeetExpectation,
  countPersistedAgents,
  createPgClientConfig,
  expectedConcreteAgentsFromSpawns,
} from './mobility-persistence-smoke-config.ts';

let backendBaseUrl = process.env.VITE_ABUTOWN_BACKEND_URL ?? 'http://127.0.0.1:8080';
const snapshotFreshnessMs = 15_000;

main().catch((error) => {
  console.error(`[mobility-persistence-smoke] ${redact(String(error?.message ?? error))}`);
  process.exit(1);
});

async function main() {
  loadEnvFile(resolve(process.cwd(), '.env'));
  backendBaseUrl = process.env.VITE_ABUTOWN_BACKEND_URL ?? backendBaseUrl;
  const databaseUrl = process.env.DATABASE_URL;
  if (!databaseUrl) {
    throw new Error('DATABASE_URL missing in ignored .env');
  }
  const baseWorldPath = process.env.ABUTOWN_BASE_WORLD_PATH ?? 'data/worlds/abutopia';
  const expectedAgents = expectedConcreteAgentsFromSpawns(
    readJsonFile(resolve(process.cwd(), baseWorldPath, 'layers/spawns.json')),
  );

  const health = await readProto(`${backendBaseUrl}/health`, HealthResponseSchema, healthResponseFromProto);
  if (health.world_id !== 'abutopia') {
    throw new Error(`unexpected health world_id ${health.world_id}`);
  }
  if (!health.ok) {
    throw new Error(`backend health not OK: persistence ${health.persistence?.status ?? 'missing'}`);
  }

  const mobility = await readProto(
    `${backendBaseUrl}/mobility`,
    MobilitySnapshotSchema,
    mobilitySnapshotFromProto,
  );
  if (mobility.world_id !== health.world_id) {
    throw new Error(`mobility world_id ${mobility.world_id} differs from health ${health.world_id}`);
  }

  const client = new pg.Client(createPgClientConfig(databaseUrl));
  try {
    await client.connect();
    const result = await client.query(
      `
        SELECT tick, updated_at, payload
        FROM mobility_snapshots
        WHERE world_id = $1
      `,
      [health.world_id],
    );

    if (result.rowCount !== 1) {
      throw new Error(`expected exactly one mobility_snapshots row for ${health.world_id}, found ${result.rowCount}`);
    }

    const row = result.rows[0];
    const persistedTick = Number(row.tick);
    const updatedAtMs = new Date(row.updated_at).getTime();
    const persistedUpdatedAgeMs = Date.now() - updatedAtMs;
    const payloadAgentCount = countPersistedAgents(row.payload);

    assertPersistedSnapshotMatchesHealth({
      healthPersistenceTick: health.persistence?.mobility_tick,
      mobilityTick: mobility.tick,
      persistedTick,
      persistedUpdatedAgeMs,
      snapshotFreshnessMs,
    });
    if (payloadAgentCount !== mobility.agents.length) {
      throw new Error(`payload agent count ${payloadAgentCount} differs from /mobility ${mobility.agents.length}`);
    }
    assertRuntimeAndPersistedAgentsMeetExpectation({
      expectedAgents,
      runtimeAgents: mobility.agents.length,
      persistedAgents: payloadAgentCount,
    });

    console.log(
      JSON.stringify({
        ok: true,
        world_id: health.world_id,
        expected_agents: expectedAgents,
        health_persistence: health.persistence?.status ?? null,
        health_tick: health.persistence?.mobility_tick ?? null,
        mobility_tick: mobility.tick,
        persisted_tick: persistedTick,
        persisted_updated_age_ms: persistedUpdatedAgeMs,
        agents: mobility.agents.length,
      }),
    );
  } finally {
    await client.end();
  }
}

function readJsonFile(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    throw new Error(`could not read base-world spawns from ${path}: ${error?.message ?? error}`);
  }
}

async function readProto(url, schema, convert) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}`);
  }

  const bytes = new Uint8Array(await response.arrayBuffer());
  return convert(fromBinary(schema, bytes));
}

function loadEnvFile(path) {
  let text;
  try {
    text = readFileSync(path, 'utf8');
  } catch {
    return;
  }

  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;

    const declaration = trimmed.startsWith('export ') ? trimmed.slice('export '.length).trimStart() : trimmed;
    const eq = declaration.indexOf('=');
    if (eq <= 0) continue;

    const key = declaration.slice(0, eq).trim();
    const rawValue = declaration.slice(eq + 1).trim();
    if (!key || process.env[key] !== undefined) continue;

    process.env[key] = unquoteEnvValue(rawValue);
  }
}

function unquoteEnvValue(value) {
  if (value.length < 2) return value;

  const quote = value[0];
  if ((quote !== '"' && quote !== "'") || value[value.length - 1] !== quote) {
    return value;
  }

  return value.slice(1, -1);
}

function redact(value) {
  return value
    .replace(/\b(postgres(?:ql)?:\/\/)([^@\s/]+@)/gi, '$1<redacted>@')
    .replace(/\bsb_secret_[^\s'"`]+/g, '<redacted>')
    .replace(/\bsb_publishable_[^\s'"`]+/g, '<redacted>');
}
