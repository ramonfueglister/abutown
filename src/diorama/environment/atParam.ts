// src/diorama/environment/atParam.ts
//
// Parsing for the `?at=` time-override boot param, shared by look.ts and
// ksw/main.ts. Two forms:
//   * full date-time, e.g. `2026-07-03T11:00:00Z` or `2026-07-04T07:30`
//     (the spec'd form; capture-env.mjs / smoke-environment.mjs rely on it) —
//     handed to `new Date()` unchanged;
//   * bare `HH:MM` wall-clock shorthand, e.g. `07:30` — resolved to TODAY at
//     that local time. `new Date('07:30')` is Invalid Date in V8, so without
//     this branch the shorthand hit the invalid-guard and broke boot.

const HH_MM = /^(\d{1,2}):(\d{2})$/;

/** Parse `?at=` — null/empty means "no override". Throws on anything that
 * doesn't resolve to a valid instant (same boot-guard error as before). */
export function parseAtParam(raw: string | null): Date | null {
  if (!raw) return null;

  const hhmm = HH_MM.exec(raw);
  if (hhmm) {
    const hours = Number(hhmm[1]);
    const minutes = Number(hhmm[2]);
    if (hours > 23 || minutes > 59) throw new Error(`invalid ?at=${raw}`);
    const d = new Date();
    d.setHours(hours, minutes, 0, 0);
    return d;
  }

  const d = new Date(raw);
  if (Number.isNaN(d.getTime())) throw new Error(`invalid ?at=${raw}`);
  return d;
}
