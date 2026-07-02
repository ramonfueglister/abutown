import { defineConfig } from 'vite';

// `npm run build` is wrapped by `scripts/build.mjs`, which copies `public/`
// to `dist/` via POSIX `cp -R` (a single fork) after vite finishes. vite's
// own per-file `copyFileSync` walk has been observed to hit transient
// `ETIMEDOUT` on the large `public/simutrans-assets/` tree. The wrapper
// sets `VITE_SKIP_PUBLIC_COPY=1` so we disable vite's copy in that mode
// only — `vite dev` continues to serve `public/` over HTTP.
const skipPublicCopy = process.env.VITE_SKIP_PUBLIC_COPY === '1';

export default defineConfig({
  publicDir: skipPublicCopy ? false : 'public',
  optimizeDeps: {
    entries: ['index.html', 'look.html', 'ksw.html', 'cards.html'],
  },
  server: {
    host: '127.0.0.1',
    port: 5175,
    strictPort: true,
  },
});
