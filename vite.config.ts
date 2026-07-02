import { defineConfig } from 'vite';

// `npm run build` is wrapped by `scripts/build.mjs`, which copies `public/`
// to `dist/` via POSIX `cp -R` (a single fork) after vite finishes. vite's
// own per-file `copyFileSync` walk has been observed to hit transient
// `ETIMEDOUT` on the large `public/simutrans-assets/` tree. The wrapper
// sets `VITE_SKIP_PUBLIC_COPY=1` so we disable vite's copy in that mode
// only — `vite dev` continues to serve `public/` over HTTP.
const skipPublicCopy = process.env.VITE_SKIP_PUBLIC_COPY === '1';
const backendTarget = process.env.VITE_ABUTOWN_BACKEND_URL || 'http://127.0.0.1:8080';

export default defineConfig({
  publicDir: skipPublicCopy ? false : 'public',
  optimizeDeps: {
    entries: ['index.html'],
  },
  server: {
    host: '127.0.0.1',
    port: 5175,
    strictPort: true,
    proxy: {
      '/base-world': backendTarget,
      '/card-hand': backendTarget,
      '/cards': backendTarget,
      '/chunks': backendTarget,
      '/commands': backendTarget,
      '/economy': backendTarget,
      '/health': backendTarget,
      '/mobility': backendTarget,
      '/world': backendTarget,
      '/ws': {
        target: backendTarget,
        ws: true,
      },
    },
  },
});
