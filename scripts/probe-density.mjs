// Throwaway density probe: opens the city page (proto + WS available in-page),
// subscribes to ALL AOI cells via a second raw WebSocket in page context, and
// reports the densest cells + their world centres. Stack must be up on :5187/:8790.
import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';

const ROOT = new URL('..', import.meta.url).pathname;
const net = JSON.parse(readFileSync(`${ROOT}/data/winterthur/trafficnet.json`, 'utf8'));
const CELL = 128;
let minX = Infinity, minZ = Infinity, maxX = -Infinity, maxZ = -Infinity;
for (const l of net.lanes) for (const p of l.pts) { if (p[0] < minX) minX = p[0]; if (p[1] < minZ) minZ = p[1]; if (p[0] > maxX) maxX = p[0]; if (p[1] > maxZ) maxZ = p[1]; }
const cols = Math.floor((maxX - minX) / CELL) + 1, rows = Math.floor((maxZ - minZ) / CELL) + 1;
const cellCenter = (id) => { const r = Math.floor(id / cols), c = id % cols; return [minX + (c + 0.5) * CELL, minZ + (r + 0.5) * CELL]; };

const browser = await chromium.launch({ headless: true, args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'] });
const page = await browser.newPage({ viewport: { width: 800, height: 600 } });
await page.goto(`http://127.0.0.1:5187/ksw.html?traffic=1&trafficWs=ws://127.0.0.1:8790/traffic&cam=bahnhof&at=2026-07-03T08:00:00Z&wx=clear`, { waitUntil: 'load', timeout: 30000 });
await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 40000 });

// In-page: open a raw WS, subscribe to all cells, accumulate per-cell membership.
await page.evaluate(async (nCells) => {
  const { fromBinary, toBinary, create } = await import('/node_modules/@bufbuild/protobuf/dist/esm/index.js').catch(() => import('@bufbuild/protobuf'));
  const pb = await import('/src/proto/traffic_pb.ts');
  const ws = new WebSocket('ws://127.0.0.1:8790/traffic');
  ws.binaryType = 'arraybuffer';
  window.__probe = { per: new Map() };
  ws.addEventListener('open', () => {
    const all = [...Array(nCells).keys()];
    ws.send(toBinary(pb.TrafficClientMsgSchema, create(pb.TrafficClientMsgSchema, { subscribeCells: all, unsubscribeCells: [] })));
  });
  ws.addEventListener('message', (ev) => {
    const server = fromBinary(pb.TrafficServerMsgSchema, new Uint8Array(ev.data));
    for (const f of server.cells) {
      let s = window.__probe.per.get(f.cell); if (!s) { s = new Set(); window.__probe.per.set(f.cell, s); }
      if (f.keyframe) s.clear();
      for (const v of f.vehicles) s.add(v.id);
      for (const id of f.departed) s.delete(id);
    }
  });
}, cols * rows);

await page.waitForTimeout(25000);

const per = await page.evaluate(() => [...window.__probe.per.entries()].map(([c, s]) => [c, s.size]));
await browser.close();

const ranked = per.map(([c, n]) => ({ cell: c, n, center: cellCenter(c) })).sort((a, b) => b.n - a.n);
console.log('cols', cols, 'rows', rows, 'total veh', ranked.reduce((a, b) => a + b.n, 0));
console.log('TOP 12 densest cells:');
for (const r of ranked.slice(0, 12)) console.log(`  cell ${r.cell}: ${r.n} veh at [${r.center[0].toFixed(0)}, ${r.center[1].toFixed(0)}]`);
