# Winterthur MMORPG M1 deploy runbook

One `sim-server` process on Fly.io (single instance) ticks the whole world
(traffic + citizens + economy + persistence + card-hand + `/live`/`/traffic`
WebSockets) + static frontend on Vercel + Supabase Postgres `:5432`.
Design: `docs/superpowers/specs/2026-07-05-mmorpg-m1-persistent-world-design.md`.

> **Single writer.** Never run more than one Fly machine — the world lives in
> memory and upserts `world_core_snapshots` for its `world_id`. A second
> instance would double-write the same world.

> **No world wipes.** Snapshots are versioned (`schema_version` +
> `migrate_snapshot` chain in `world-core/src/persist.rs`). Schema changes ship
> a migration arm — the old `DELETE FROM …_snapshots` ritual is forbidden once
> the world is live.

## 1. Backend (Fly.io)

The image bundles `data/winterthur/{trafficnet.json,trips.bin,simworld.json,economy.json}`.

```bash
fly auth login                      # interactive — run via `! fly auth login`
fly launch --no-deploy --copy-config --region lhr
fly secrets set \
  DATABASE_URL='postgresql://…@…pooler.supabase.com:5432/postgres?sslmode=verify-full' \
  SUPABASE_URL='https://<project>.supabase.co' \
  CORS_ALLOWED_ORIGINS='https://PLACEHOLDER-set-after-vercel' \
  ABUTOWN_DB_MAX_CONNECTIONS='8'
fly deploy
# Verify:
curl -s https://<app>.fly.dev/health   # expect ok=true, world_tick, audit_ok, resumed
```

- `DATABASE_URL` MUST use `:5432` (session pooler). `:6543` crashes sqlx.
- Without `DATABASE_URL` the server runs in-memory (dev mode) and logs a
  warning — never deploy that way.
- `ABUTOWN_WORLD_ID=winterthur` comes from the image `ENV`.
- **Resume proof** is the boot-log line
  `resuming world-core from persisted snapshot tick=…` (check `fly logs` after
  every deploy; a fresh world logs `seeding fresh world-core state` instead —
  on an established world that line means something is wrong).

## 2. Frontend (Vercel)

Build locally, deploy static `dist/` (Vercel rebuild skips the buf wrapper).
Project env vars needed:

```
VITE_SUPABASE_URL=https://<project>.supabase.co
VITE_SUPABASE_ANON_KEY=<anon key>
VITE_LIVE_WS=wss://<app>.fly.dev/live
VITE_TRAFFIC_WS=wss://<app>.fly.dev/traffic
```

The city view needs the world tile pyramid (`data/winterthur/world/`, ~77 MB,
gitignored). Bake it before `npm run build` (`npm run geo:fetch` +
`npm run geo:bake-world`) so it ships inside `dist/`.

```bash
vercel login                        # interactive — run via `! vercel login`
vercel --prod
```

## 3. Wire CORS to the real frontend origin

```bash
fly secrets set CORS_ALLOWED_ORIGINS='https://<frontend>.vercel.app'   # restarts the machine
```

## 4. Verify (acceptance)

- `GET /health` → `ok=true`, `world_tick` increasing between two calls,
  `audit_ok=true`, `resumed=true` after the first restart.
- Open the Vercel URL with `?live=1` in TWO clean browsers → both show the
  same population/world clock in the vitals HUD; world time runs 6× real time.
- `fly logs` shows the resume line after a restart, never a conservation panic.
- Anonymous (not logged in) visitors can watch; login is only for the card hand.

## Rollback

`fly releases` then `fly deploy --image <previous>`; or `fly apps restart <app>`.
Snapshots are upserts — rolling back the binary never loses the world, but an
OLDER binary cannot read a NEWER snapshot schema version (it fails loud). Roll
forward with a fix instead of wiping.
