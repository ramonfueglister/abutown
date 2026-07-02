// Ad-hoc perf probe (not part of CI): measures the real rAF frame-time
// distribution per camera preset and agent count. Matrix {overview, er} x
// {default, 10000}: fps, p50/p95/max frame (ms) over >=600 frames after a
// warm-up (spans multiple 240-frame GI-probe periods — the old hitch cycle),
// plus drawCalls/triangles via the __KSW_INFO() debug hook.
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';

const HOST = '127.0.0.1';
const PORT = 5205;
const FRAMES = 700;

const portOpen = (h, p) =>
  new Promise((r) => {
    const s = net.createConnection({ host: h, port: p }, () => {
      s.end();
      r(true);
    });
    s.on('error', () => r(false));
    s.setTimeout(800, () => {
      s.destroy();
      r(false);
    });
  });
const dev = spawn('npm', ['run', 'dev', '--', '--port', String(PORT), '--strictPort'], {
  cwd: new URL('..', import.meta.url).pathname,
  stdio: 'ignore',
  detached: true,
});
process.on('exit', () => {
  try {
    process.kill(-dev.pid, 'SIGKILL');
  } catch {}
});
const t0 = Date.now();
while (Date.now() - t0 < 30000 && !(await portOpen(HOST, PORT))) await new Promise((r) => setTimeout(r, 200));
const browser = await chromium.launch({ headless: true, args: ['--enable-unsafe-webgpu', '--enable-gpu', '--use-angle=metal'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
console.log('cam        agents   fps    p50     p95     max     drawCalls  tris');
for (const agents of [undefined, 10000]) {
  for (const cam of ['overview', 'er']) {
    const q = agents === undefined ? '' : `&agents=${agents}`;
    await page.goto(`http://${HOST}:${PORT}/ksw.html?preset=morning&cam=${cam}${q}`, { waitUntil: 'load' });
    await page.waitForFunction(() => window.__LOOK_READY === true, { timeout: 30000 });
    await page.waitForTimeout(3000); // warm-up: shader compiles, TRAA settle
    const r = await page.evaluate(
      (frames) =>
        new Promise((res) => {
          const dts = [];
          let prev = performance.now();
          const loop = () => {
            const now = performance.now();
            dts.push(now - prev);
            prev = now;
            if (dts.length >= frames) {
              dts.sort((a, b) => a - b);
              const quantile = (p) => dts[Math.min(dts.length - 1, Math.floor(p * dts.length))];
              const total = dts.reduce((a, b) => a + b, 0);
              const info = window.__KSW_INFO();
              res({
                fps: ((dts.length / total) * 1000).toFixed(1),
                p50: quantile(0.5).toFixed(1),
                p95: quantile(0.95).toFixed(1),
                max: dts[dts.length - 1].toFixed(1),
                drawCalls: info.drawCalls,
                triangles: info.triangles,
              });
            } else requestAnimationFrame(loop);
          };
          requestAnimationFrame(loop);
        }),
      FRAMES,
    );
    const label = agents === undefined ? 'default' : String(agents);
    console.log(
      `${cam.padEnd(10)} ${label.padEnd(8)} ${r.fps.padEnd(6)} ${r.p50.padEnd(7)} ${r.p95.padEnd(7)} ${r.max.padEnd(7)} ${String(r.drawCalls).padEnd(10)} ${r.triangles}`,
    );
  }
}
await browser.close();
process.exit(0);
