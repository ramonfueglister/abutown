#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const mode = process.argv[2] ?? 'dev';
const frontendPort = mode === 'preview' ? '5173' : '5175';
const frontendScript = mode === 'preview' ? 'preview' : 'dev';
const killProcessGroup = process.platform !== 'win32';
const viteBin = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const backendBinaryName = process.platform === 'win32' ? 'sim-server.exe' : 'sim-server';
const backendBinary = join(resolveCargoTargetDir(), 'debug', backendBinaryName);
const backendHealthUrl = 'http://127.0.0.1:8080/health';
const backendStartupTimeoutMs = 120_000;

const backendBuildCommand = {
  name: 'backend-build',
  command: 'cargo',
  args: ['build', '--manifest-path', 'backend/Cargo.toml', '-p', 'sim-server'],
};
const backendCommand = {
  name: 'backend',
  command: backendBinary,
  args: [],
  env: { RUST_LOG: process.env.RUST_LOG ?? 'error' },
};
const frontendCommand = {
  name: 'frontend',
  command: process.execPath,
  args: [viteBin, frontendScript, '--host', '127.0.0.1', '--port', frontendPort],
};

let shuttingDown = false;
const children = [];

process.on('SIGINT', () => shutdown(130));
process.on('SIGTERM', () => shutdown(143));

startStack().catch((error) => {
  console.error(`[stack] ${error.message}`);
  shutdown(1);
});

async function startStack() {
  await runCommand(backendBuildCommand);
  if (shuttingDown) return;
  spawnService(backendCommand);
  await waitForHttpOk(backendHealthUrl, backendStartupTimeoutMs);
  if (shuttingDown) return;
  spawnService(frontendCommand);
  setInterval(() => {}, 60_000);
}

function resolveCargoTargetDir() {
  const configured = process.env.CARGO_TARGET_DIR;
  if (configured) return isAbsolute(configured) ? configured : resolve(process.cwd(), configured);
  return resolve(process.cwd(), 'backend/target');
}

function runCommand({ name, command, args }) {
  return new Promise((resolveCommand, rejectCommand) => {
    const child = spawn(command, args, {
      cwd: process.cwd(),
      env: process.env,
      stdio: 'inherit',
    });
    child.on('error', rejectCommand);
    child.on('exit', (code, signal) => {
      if (signal) {
        rejectCommand(new Error(`[${name}] exited with ${signal}`));
        return;
      }
      if ((code ?? 0) !== 0) {
        rejectCommand(new Error(`[${name}] exited with ${code}`));
        return;
      }
      resolveCommand();
    });
  });
}

function spawnService({ name, command, args, env = {} }) {
  const child = spawn(command, args, {
    cwd: process.cwd(),
    env: { ...process.env, ...env },
    detached: killProcessGroup,
    stdio: 'inherit',
  });
  children.push(child);
  child.on('error', (error) => {
    console.error(`[${name}] ${error.message}`);
    shutdown(1);
  });
  child.on('exit', (code, signal) => {
    if (shuttingDown) return;
    const status = signal ? 1 : code ?? 0;
    if (status !== 0) console.error(`[${name}] exited with ${signal ?? status}`);
    shutdown(status);
  });
  return child;
}

async function waitForHttpOk(url, timeoutMs) {
  const start = Date.now();
  while (!shuttingDown) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // Retry until timeout; backend compilation and startup take a few seconds.
    }
    if (Date.now() - start > timeoutMs) {
      throw new Error(`timed out waiting for ${url}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}

function shutdown(code) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) {
    terminate(child);
  }
  setTimeout(() => process.exit(code), 800).unref();
}

function terminate(child) {
  if (child.killed) return;
  if (killProcessGroup && child.pid) {
    try {
      process.kill(-child.pid, 'SIGTERM');
      return;
    } catch {
      // The process may already have exited; fall through to the direct child.
    }
  }
  child.kill('SIGTERM');
}
