// tests/live/vitalsHud.test.ts
//
// Task 15: pure formatting helpers of the vitals HUD (the DOM card itself is
// covered by the browser smoke, not unit tests).

import { describe, expect, it } from 'vitest';
import { formatMoney, formatWorldClock } from '../../src/diorama/live/vitalsHud';

describe('formatWorldClock', () => {
  it('formats seconds-of-world-day as HH:MM', () => {
    expect(formatWorldClock(0)).toBe('00:00');
    expect(formatWorldClock(27_000)).toBe('07:30'); // 7*3600 + 30*60
    expect(formatWorldClock(3_600)).toBe('01:00');
    expect(formatWorldClock(86_340)).toBe('23:59');
  });

  it('ignores sub-minute seconds and wraps a full day', () => {
    expect(formatWorldClock(59)).toBe('00:00');
    expect(formatWorldClock(86_400)).toBe('00:00');
    expect(formatWorldClock(86_400 + 3_660)).toBe('01:01');
  });
});

describe('formatMoney', () => {
  it('divides the raw x1000 amount and groups thousands', () => {
    expect(formatMoney(123_456_789n)).toBe("123'457"); // rounds .789 up
    expect(formatMoney(1_000n)).toBe('1');
    expect(formatMoney(0n)).toBe('0');
    expect(formatMoney(999_999_000n)).toBe("999'999");
    expect(formatMoney(1_000_000_000n)).toBe("1'000'000");
  });

  it('rounds to the nearest unit and keeps the sign', () => {
    expect(formatMoney(1_499n)).toBe('1');
    expect(formatMoney(1_500n)).toBe('2');
    expect(formatMoney(-2_500_000n)).toBe("-2'500");
    expect(formatMoney(-1_499n)).toBe('-1');
  });
});
