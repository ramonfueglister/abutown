// tests/diorama/atParam.test.ts
//
// ?at= time-override parsing (shared by look.ts and ksw/main.ts boot).
// The documented full-ISO form (spec 2026-07-03-echtzeit-wetter-sonne §84,
// used by capture-env.mjs / smoke-environment.mjs) must keep working, and the
// bare `HH:MM` dev shorthand must resolve to TODAY at that local wall-clock
// time — `new Date('07:30')` is Invalid Date in V8, which used to throw at
// boot (`invalid ?at=07:30`) and take the whole app down.

import { describe, expect, it } from 'vitest';
import { parseAtParam } from '../../src/diorama/environment/atParam';

describe('parseAtParam', () => {
  it('returns null when the param is absent or empty (matches the old `atParam ?` guard)', () => {
    expect(parseAtParam(null)).toBeNull();
    expect(parseAtParam('')).toBeNull();
  });

  it('parses the full-ISO form (spec + capture/smoke harness contract)', () => {
    const d = parseAtParam('2026-07-03T11:00:00Z');
    expect(d).not.toBeNull();
    expect(d!.getTime()).toBe(Date.parse('2026-07-03T11:00:00Z'));
  });

  it('parses local ISO without zone exactly like new Date() did', () => {
    const d = parseAtParam('2026-07-04T07:30');
    expect(d!.getTime()).toBe(new Date('2026-07-04T07:30').getTime());
  });

  it('resolves bare HH:MM to today at that local wall-clock time', () => {
    const d = parseAtParam('07:30');
    const today = new Date();
    expect(d!.getFullYear()).toBe(today.getFullYear());
    expect(d!.getMonth()).toBe(today.getMonth());
    expect(d!.getDate()).toBe(today.getDate());
    expect(d!.getHours()).toBe(7);
    expect(d!.getMinutes()).toBe(30);
    expect(d!.getSeconds()).toBe(0);
  });

  it('accepts single-digit hours and 24h evening times', () => {
    expect(parseAtParam('7:05')!.getHours()).toBe(7);
    expect(parseAtParam('7:05')!.getMinutes()).toBe(5);
    expect(parseAtParam('23:59')!.getHours()).toBe(23);
  });

  it('rejects out-of-range wall-clock values', () => {
    expect(() => parseAtParam('24:00')).toThrow(/invalid \?at=/);
    expect(() => parseAtParam('12:60')).toThrow(/invalid \?at=/);
  });

  it('throws the boot guard error for garbage (unchanged behaviour)', () => {
    expect(() => parseAtParam('gestern')).toThrow(/invalid \?at=gestern/);
  });
});
