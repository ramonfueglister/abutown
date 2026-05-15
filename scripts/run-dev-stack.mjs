#!/usr/bin/env node
import { spawn } from 'node:child_process';

const mode = process.argv[2] ?? 'dev';
const frontendPort = mode === 'preview' ? '5173' : '5175';
const frontendScript = mode === 'preview' ? 'preview' : 'dev';

const commands = [
  {
    name: 'backend',
    command: 'cargo',
    args: ['run', '--manifest-path', 'backend/Cargo.toml', '-p', 'sim-server'],
  },
  {
    name: 'frontend',
    command: 'npm',
    args: ['run', frontendScript, '--', '--port', frontendPort],
  },
];

let shuttingDown = false;

const children = commands.map(({ name, command, args }) => {
  const child = spawn(command, args, {
    cwd: process.cwd(),
    env: process.env,
    stdio: 'inherit',
  });
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
});

process.on('SIGINT', () => shutdown(130));
process.on('SIGTERM', () => shutdown(143));

function shutdown(code) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) {
    if (!child.killed) child.kill('SIGTERM');
  }
  setTimeout(() => process.exit(code), 800).unref();
}
