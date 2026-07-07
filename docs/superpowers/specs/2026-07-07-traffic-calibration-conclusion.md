# Traffic SOTA S2 — Calibration conclusion (2026-07-07)

Calibrating the Winterthur microsim against real Stadt-Winterthur MIV count
stations (hourly, per SWISS10 class, K501/K606/K611) did what calibration is
supposed to do: it surfaced four real gridlock bugs that all the unit tests
had missed, and then pinned the residual mismatch to an architectural cause
rather than a model defect.

## What the calibration harness found and fixed

Running one full **world day** (`--bin calibrate`, 144 000 ticks, the same
deterministic `build_sim` chain the server uses) and comparing simulated vs
observed hourly cross-section flows exposed, in order:

1. **Mutual-yield deadlock.** Gap acceptance vetoed against *standing*
   conflicting vehicles; at a right-before-left node (cyclic by
   construction) two vehicles waited on each other forever. First run:
   26.6 k of 26.7 k vehicles frozen by world midnight. Fix: the critical
   gap is judged only against an *approaching* stream (≥ 0.5 m/s); phase-2
   conflict-point occupancy remains the physical crossing authority.
2. **Missed-turn stranding.** A MOBIL lane change onto a turnless secondary
   lane left a vehicle walled at the lane end with no onward turn — 9.4 k
   dead-end removals/day. Fix: a SUMO-style *strategic mandatory* lane change
   inside the 150 m urgent zone (comfort threshold waived, only the safety
   veto applies), plus a kernel `stranded_last_tick` seam and a shell
   `rescue_stranded` system (re-route from the current lane; side-reseat off
   turnless lanes; loud counted removal only as a last resort).
3. **Entry drop.** A blocked spawn was *dropped* permanently — 73 % of
   weekday demand never entered. Fix: a SUMO-style insertion queue (route
   cached at release, retry every tick, give up after 15 world-minutes).
4. **Demand-blind signal splits.** The bake's fixed-time green splits
   (14 / 60 s for the main movement at the worst node) starved main flows to
   ~420 veh/h. Fix (**S3**): vehicle-actuated control (VSS/RiLSA practice) —
   min-green 5 s, gap-out 3 s, max-green 40 s, empty phases skipped, idle
   green when nobody else is waiting; deterministic, updated once per tick
   from the previous snapshot.

Each fix is covered by a regression test (mutual-yield, wrong-lane-merge,
insertion-queue, actuated idle-green / cross-demand) and moved the world-day
metrics monotonically toward health.

## The residual mismatch is the 6× world clock, not the model

After the four fixes, the network still saturates during the world day at
`demand_scale = 1.0`. Three further capacity levers were tested and **ruled
out** as the cause: relaxing gap-acceptance headways (6→4.5 s), shortening
conflict-point clearance (2→1.2 s), and adding "don't-block-the-box"
spillback prevention each changed the outcome by noise. The levers were
reverted (no cruft).

The actual cause is structural. The world runs at **6× real time**
(`WORLD_TIME_SCALE = 6`), so census demand is *released* on world-time while
vehicles *move* at real m/s through the kernel. The compressed rush-hour peak
therefore arrives ~6× faster than vehicles can physically clear the network —
Little's-law inflow exceeds drainage, congestion feeds back, and the network
gridlocks regardless of node parameters.

A demand-scale sweep confirms it. At `demand_scale ≈ 0.15` (≈ 1/6, matching
the time compression) the network is **fluid all day**: alive count stays
1000–2100, mean speed 5–13 m/s, and it empties overnight (749 alive at end,
no permanent gridlock). At that fluid operating point the demand-normalized
cross-section flows jump from a total ratio of **0.08 → 0.30** of observed,
with the busiest measured corridor (Seenerstrasse → Seen) matching almost
exactly (**0.97**) and several others at 0.36–0.47.

| Station-direction | sim/0.15 (veh/day) | observed | ratio |
|---|---|---|---|
| Seenerstrasse → Seen | 3720 | 3838 | **0.97** |
| Steigstrasse → Winterthur Zentrum | 967 | 2072 | 0.47 |
| Seenerstrasse → Oberwinterthur | 2393 | 6530 | 0.37 |
| Untere Vogelsangstr. → A1-Anschluss | 2487 | 6936 | 0.36 |
| Breitestrasse → Seen (Grüze) | 100 | 1923 | 0.05 |

## Recommendations

1. **Operate the live traffic at `demand_scale ≈ 0.15`.** The current
   `WORLD_BG_DEMAND_SCALE = 0.5` (world-sim active) is ~3× above the fluid
   point and will gridlock the city under sustained census demand. This is
   the single highest-value change and needs a product decision (it trades
   absolute vehicle counts for a network that actually flows).
2. **Residual under-count is demand-bake calibration, not physics.** At the
   fluid point the 0.30 overall ratio (with spot-on and near-zero corridors
   side by side) points at the census-gravity OD + gateway volumes routing
   too little through specific streets — a `demand-gen` retuning effort,
   separate from the dynamics core.
3. **GEH is not meaningful at reduced demand** (it compares absolute veh/h);
   revisit it once the demand bake is retuned to run fluidly at scale 1.0, or
   add a per-station demand-normalisation to the report.

## Reproduce

```
# fluid operating point
CALIBRATION_OUT=scratch/calibration/sim.json DEMAND_SCALE=0.15 \
  scripts/cargo-serial.sh run --manifest-path backend/Cargo.toml \
  --release -p winterthur-traffic --bin calibrate
node scripts/traffic/calibration-report.mjs --simulated scratch/calibration/sim.json
# single-node micro watch (why a junction backs up)
CALIBRATE_WATCH_NODE=2819 scripts/cargo-serial.sh run … --bin calibrate
```
