# Abutopia public deploy runbook

One shared abutopia `sim-server` on Fly.io (single instance) + static frontend on
Vercel + Supabase `:5432`. Design: `docs/superpowers/specs/2026-06-08-abutopia-remote-deploy-design.md`.

> **Single writer.** Never run more than one Fly machine — the world lives in memory.
> A second instance would double-write the same `world_id`.

## 0. One-time: fresh-seed abutopia on the remote DB (authorized)

Clears any old `home_market=0` records so the #86 rebind regenerates bindings on boot:

```bash
psql "$DATABASE_URL" \
  -c "DELETE FROM mobility_snapshots WHERE world_id='abutopia';" \
  -c "DELETE FROM economy_snapshots  WHERE world_id='abutopia';"
```

## 1. Backend (Fly.io)

```bash
fly auth login                      # interactive — run via `! fly auth login`
fly launch --no-deploy --copy-config --name abutown-abutopia --region lhr
fly secrets set \
  DATABASE_URL='postgresql://…@…pooler.supabase.com:5432/postgres?sslmode=verify-full' \
  CORS_ALLOWED_ORIGINS='https://PLACEHOLDER-set-after-vercel' \
  ABUTOWN_DB_MAX_CONNECTIONS='8'
fly deploy
# Verify:
curl -s https://abutown-abutopia.fly.dev/health -o /dev/null -w '%{http_code}\n'   # expect 200
```

`PGSSLROOTCERT`, `LISTEN_HOST`, `LISTEN_PORT`, `RUST_LOG` come from the image `ENV`.
Confirm the largest safe `ABUTOWN_DB_MAX_CONNECTIONS` against the Supabase dashboard
pooler ceiling (8 is conservative-safe).

## 2. Frontend (Vercel)

```bash
vercel login                        # interactive — run via `! vercel login`
VITE_ABUTOWN_BACKEND_URL='https://abutown-abutopia.fly.dev' vercel --prod
# Note the production URL it prints, e.g. https://abutown.vercel.app
```

For repeat builds, set `VITE_ABUTOWN_BACKEND_URL` as a Vercel project env var instead.

## 3. Wire CORS to the real frontend origin

```bash
fly secrets set CORS_ALLOWED_ORIGINS='https://abutown.vercel.app'   # restarts the machine
```

## 4. Verify (acceptance)

- `GET https://abutown-abutopia.fly.dev/health` → `ok=true`, persistence not `stale`, `world_id=abutopia`.
- Open the Vercel URL in a clean browser → abutopia renders over `wss://`, no
  "persistence stale" overlay, no CORS/mixed-content console errors.
- `fly logs` shows `economy::liveness … routed > 0`.
- Open the URL in a second browser → both see the live, ticking world.

## Rollback

`fly releases` then `fly deploy --image <previous>`; or `fly apps restart abutown-abutopia`.
