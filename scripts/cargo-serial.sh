#!/usr/bin/env bash
# Serialize cargo invocations so two cargo builds never contend for the same
# target/ lock at once. Concurrent cargo runs in one target dir don't corrupt
# anything — they block on cargo's build lock — but that blocking shows up as
# multi-minute "hangs". This wrapper makes the second caller WAIT explicitly
# (with visible progress) instead of silently stalling, and reclaims the lock
# if the holding process died.
#
# Usage: scripts/cargo-serial.sh test --manifest-path backend/Cargo.toml -p sim-server
#
# Every cargo command in this repo (humans, scripts, subagents) should go
# through this wrapper. Tune the wait ceiling with CARGO_SERIAL_MAX_WAIT
# (seconds, default 1200).
set -euo pipefail

LOCKDIR="${TMPDIR:-/tmp}/abutown-cargo.lock.d"
PIDFILE="$LOCKDIR/pid"
MAX_WAIT="${CARGO_SERIAL_MAX_WAIT:-1200}"
waited=0

acquire() {
  while ! mkdir "$LOCKDIR" 2>/dev/null; do
    local holder=""
    [[ -f "$PIDFILE" ]] && holder="$(cat "$PIDFILE" 2>/dev/null || true)"
    if [[ -n "$holder" ]] && ! kill -0 "$holder" 2>/dev/null; then
      echo "cargo-serial: stale lock from dead pid $holder — reclaiming" >&2
      rm -rf "$LOCKDIR"
      continue
    fi
    if (( waited >= MAX_WAIT )); then
      echo "cargo-serial: gave up after ${MAX_WAIT}s waiting for cargo pid ${holder:-?}" >&2
      exit 1
    fi
    echo "cargo-serial: another cargo (pid ${holder:-?}) is running — waiting (${waited}s)…" >&2
    sleep 2
    waited=$((waited + 2))
  done
  echo "$$" > "$PIDFILE"
}

release() { rm -rf "$LOCKDIR"; }

acquire
trap release EXIT INT TERM
echo "cargo-serial: lock acquired (pid $$) — running: cargo $*" >&2
cargo "$@"
