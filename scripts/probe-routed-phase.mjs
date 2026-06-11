// READ-ONLY diagnostic probe: subscribes to chunks covering the abutopia markets
// and buckets vitals.routed_citizens by tick % macro_flow_interval_ticks (10).
// This is the honest way to observe cadence-sensitive gauges — a fixed-cadence
// sample (like the 60-tick liveness log) aliases onto the pulse phase when the
// gauge period divides the sample period. Sends ONLY ChunkSubscribe.
//
// Usage: PROBE_URL=ws://127.0.0.1:8080/ws node scripts/probe-routed-phase.mjs
// Pass criterion (post target-hold): routed > 0 in EVERY observed phase bucket
// once the first delivery has occurred (zero-order hold ⇒ phase-invariant gauge).
import { create, toBinary, fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';

const proto = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ClientMessageSchema, ServerMessageSchema } = proto;

const URL = process.env.PROBE_URL ?? 'ws://127.0.0.1:8080/ws';
const DURATION_MS = Number(process.env.PROBE_DURATION_MS ?? 45_000);
const INTERVAL = Number(process.env.PROBE_FLOW_INTERVAL ?? 10);

const samples = []; // { tick, routed }
const seenTicks = new Set();

const ws = new WebSocket(URL);
ws.binaryType = 'arraybuffer';

ws.addEventListener('open', () => {
  const coords = [];
  for (let y = 0; y < 3; y += 1) for (let x = 0; x < 8; x += 1) coords.push({ x, y });
  const msg = create(ClientMessageSchema, {
    body: { case: 'chunkSubscribe', value: { protocolVersion: 1, coords } },
  });
  ws.send(toBinary(ClientMessageSchema, msg));
  console.error(`[probe] subscribed ${coords.length} chunks at ${URL}`);
});

ws.addEventListener('message', (ev) => {
  let m;
  try {
    m = fromBinary(ServerMessageSchema, new Uint8Array(ev.data));
  } catch {
    return;
  }
  if (m.body.case !== 'economySnapshot') return;
  const v = m.body.value;
  if (!v.vitals) return;
  const tick = Number(v.tick);
  if (seenTicks.has(tick)) return;
  seenTicks.add(tick);
  samples.push({ tick, routed: Number(v.vitals.routedCitizens) });
});

ws.addEventListener('error', (e) => {
  console.log(JSON.stringify({ status: 'ws-error', error: String(e?.message ?? e) }));
  process.exit(1);
});

setTimeout(() => {
  ws.close();
  const buckets = Array.from({ length: INTERVAL }, () => ({ n: 0, zero: 0, min: Infinity, max: 0 }));
  for (const s of samples) {
    const b = buckets[s.tick % INTERVAL];
    b.n += 1;
    if (s.routed === 0) b.zero += 1;
    b.min = Math.min(b.min, s.routed);
    b.max = Math.max(b.max, s.routed);
  }
  const phases = buckets.map((b, i) => ({
    phase: i,
    samples: b.n,
    zero_samples: b.zero,
    min: b.n ? b.min : null,
    max: b.max,
  }));
  // Honest pass criterion: ignore the warm-up before the first nonzero sample
  // (the hold re-arms within one interval after boot/resume).
  const firstNonzero = samples.findIndex((s) => s.routed > 0);
  const steady = firstNonzero >= 0 ? samples.slice(firstNonzero) : [];
  const steadyZero = steady.filter((s) => s.routed === 0).length;
  console.log(JSON.stringify({
    status: 'ok',
    total_tick_samples: samples.length,
    tick_range: samples.length ? [samples[0].tick, samples.at(-1).tick] : null,
    phases,
    steady_state: {
      from_tick: firstNonzero >= 0 ? samples[firstNonzero].tick : null,
      samples: steady.length,
      zero_samples: steadyZero,
      phase_stable: steady.length > INTERVAL && steadyZero === 0,
    },
  }, null, 1));
  process.exit(0);
}, DURATION_MS);
