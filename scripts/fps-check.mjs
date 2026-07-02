// Ad-hoc FPS probe (not part of CI): measures real rAF rate per camera preset.
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import net from 'node:net';
const HOST='127.0.0.1', PORT=5186;
const portOpen=(h,p)=>new Promise(r=>{const s=net.createConnection({host:h,port:p},()=>{s.end();r(true)});s.on('error',()=>r(false));s.setTimeout(800,()=>{s.destroy();r(false)})});
const dev=spawn('npm',['run','dev','--','--port','5186','--strictPort'],{cwd:new URL('..',import.meta.url).pathname,stdio:'ignore',detached:true});
process.on('exit',()=>{try{process.kill(-dev.pid,'SIGKILL')}catch{}});
const t0=Date.now(); while(Date.now()-t0<30000 && !(await portOpen(HOST,PORT))) await new Promise(r=>setTimeout(r,200));
const browser=await chromium.launch({headless:true,args:['--enable-unsafe-webgpu','--enable-gpu','--use-angle=metal']});
const page=await browser.newPage({viewport:{width:1280,height:800}});
for (const cam of ['overview','er']) {
  await page.goto(`http://${HOST}:${PORT}/ksw.html?preset=morning&cam=${cam}`,{waitUntil:'load'});
  await page.waitForFunction(()=>window.__LOOK_READY===true,{timeout:30000});
  await page.waitForTimeout(1500);
  const fps=await page.evaluate(()=>new Promise(res=>{let n=0;const s=performance.now();const loop=()=>{n++;const dt=performance.now()-s;if(dt>=3000)res((n/dt*1000).toFixed(1));else requestAnimationFrame(loop)};requestAnimationFrame(loop)}));
  console.log(`FPS ${cam}: ${fps}`);
}
await browser.close();
process.exit(0);
