// READ-ONLY diagnostic probe against prod: subscribes to all chunks covering the
// abutopia markets and reports, over ~75s, the three attribution inputs:
//   (A) chunk states (do subscribed market chunks reach ACTIVE/HOT?)
//   (B) per-market goods telemetry (traded qty) + per-market wage_paid_last_tick
//   (C) vitals.routed_citizens (the liveness gauge) + population
// Sends ONLY ChunkSubscribe (the normal viewer message); never mutates anything.
import { create, toBinary, fromBinary } from '@bufbuild/protobuf';
import { tsImport } from 'tsx/esm/api';

const proto = await tsImport('../src/backend/proto/abutown_pb.ts', import.meta.url);
const { ClientMessageSchema, ServerMessageSchema } = proto;

const URL = process.env.PROBE_URL ?? 'wss://abutown-abutopia.fly.dev/ws';
const DURATION_MS = 75_000;

const chunkStates = new Map(); // "x,y" -> latest state enum
const wageByMarket = new Map();
const tradedByMarketGood = new Map();
const vitalsSamples = [];
const caseCounts = new Map();
const serverErrors = [];
const priceByMarketGood = new Map();
let routedNonzeroSamples = 0;
let economyFrames = 0;
let flowsMax = 0;

const ws = new WebSocket(URL);
ws.binaryType = 'arraybuffer';

ws.addEventListener('open', () => {
  // Cover chunks (0..7)x(0..3): includes 9001(0,0-ish), 9002(3,2), 9003(0,1), 9004(6,1).
  const coords = [];
  for (let y = 0; y < 3; y += 1) for (let x = 0; x < 8; x += 1) coords.push({ x, y });
  const msg = create(ClientMessageSchema, {
    body: { case: 'chunkSubscribe', value: { protocolVersion: 1, coords } },
  });
  ws.send(toBinary(ClientMessageSchema, msg));
  console.error(`[probe] subscribed ${coords.length} chunks`);
});

ws.addEventListener('message', (ev) => {
  let m;
  try {
    m = fromBinary(ServerMessageSchema, new Uint8Array(ev.data));
  } catch {
    return;
  }
  caseCounts.set(m.body.case, (caseCounts.get(m.body.case) ?? 0) + 1);
  if (m.body.case === 'error') serverErrors.push(JSON.stringify({ code: m.body.value.code, msg: m.body.value.message }));
  if (m.body.case === 'mobilityChunkSnapshot' || m.body.case === 'mobilityChunkDelta') {
    const v = m.body.value;
    if (v.coord) chunkStates.set(`${v.coord.x},${v.coord.y}`, v.chunkState ?? v.state ?? 0);
  }
  if (m.body.case === 'economySnapshot') {
    economyFrames += 1;
    const v = m.body.value;
    for (const mk of v.markets ?? []) {
      wageByMarket.set(mk.marketId, Number(mk.wagePaidLastTick));
    }
    for (const g of v.goods ?? []) {
      const key = `${g.marketId}:${g.goodId}`;
      const prev = tradedByMarketGood.get(key) ?? { traded: 0, unmet: 0, unsold: 0, nonzeroTicks: 0 };
      const traded = Number(g.tradedQtyLastTick);
      priceByMarketGood.set(key, { last: Number(g.lastSettlementPrice), ewmaRef: Number(g.ewmaReferencePrice) });
      tradedByMarketGood.set(key, {
        traded,
        unmet: Number(g.unmetDemandLastTick),
        unsold: Number(g.unsoldSupplyLastTick),
        nonzeroTicks: prev.nonzeroTicks + (traded > 0 ? 1 : 0),
      });
    }
    if (v.vitals) {
      if (Number(v.vitals.routedCitizens) > 0) routedNonzeroSamples += 1;
      vitalsSamples.push({
        tick: Number(v.tick),
        routed: Number(v.vitals.routedCitizens),
        pop: Number(v.vitals.population),
        money: Number(v.vitals.totalMoney),
      });
    }
    flowsMax = Math.max(flowsMax, (v.flows ?? []).length);
  }
});

ws.addEventListener('error', (e) => {
  console.log(JSON.stringify({ status: 'ws-error', error: String(e?.message ?? e) }));
  process.exit(1);
});

setTimeout(() => {
  ws.close();
  const stateName = (s) => ['UNSPEC', 'ASLEEP', 'WARM', 'ACTIVE', 'HOT'][s] ?? s;
  const states = {};
  for (const [k, v] of chunkStates) states[k] = stateName(v);
  const lastVitals = vitalsSamples.at(-1);
  console.log(JSON.stringify({
    status: 'ok',
    economy_frames: economyFrames,
    flows_max: flowsMax,
    chunk_states: states,
    market_chunk_states: { m9003_c0_1: states['0,1'], m9002_c3_2: states['3,2'], m9004_c6_1: states['6,1'] },
    wage_by_market: Object.fromEntries(wageByMarket),
    traded_by_market_good: Object.fromEntries(tradedByMarketGood),
    vitals_first: vitalsSamples[0],
    vitals_last: lastVitals,
    routed_ever_nonzero: vitalsSamples.some((v) => v.routed > 0),
    routed_nonzero_samples: routedNonzeroSamples,
    total_vitals_samples: vitalsSamples.length,
    message_case_counts: Object.fromEntries(caseCounts),
    server_errors: serverErrors.slice(0, 5),
    price_by_market_good: Object.fromEntries(priceByMarketGood),
  }, null, 1));
  process.exit(0);
}, DURATION_MS);
