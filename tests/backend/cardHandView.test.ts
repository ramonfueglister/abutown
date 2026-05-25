import { describe, expect, it, vi } from 'vitest';

vi.mock('@supabase/supabase-js', () => ({
  createClient: vi.fn(),
}));

import {
  buildOtpLoginPayload,
  cardHandStatusText,
  isCardHandStatusVisible,
  resolveCardHandBaseUrl,
} from '../../src/cardHand/cardHandView';

describe('card hand view backend URL', () => {
  it('uses the live local backend by default', () => {
    expect(resolveCardHandBaseUrl()).toBe('http://127.0.0.1:8080');
  });

  it('allows an explicit backend URL override', () => {
    expect(resolveCardHandBaseUrl('https://backend.example.test')).toBe('https://backend.example.test');
  });
});

describe('card hand view login state', () => {
  it('does not show a signed-out card hand status badge', () => {
    const signedOut = { status: 'signed_out' as const, cards: [], error: null };

    expect(cardHandStatusText(signedOut)).toBe('');
    expect(isCardHandStatusVisible(signedOut)).toBe(false);
  });

  it('builds OTP login payloads from inline email input', () => {
    expect(buildOtpLoginPayload(' player@example.test ', 'http://127.0.0.1:5175/')).toEqual({
      email: 'player@example.test',
      options: {
        emailRedirectTo: 'http://127.0.0.1:5175/',
      },
    });

    expect(buildOtpLoginPayload('   ', 'http://127.0.0.1:5175/')).toBeNull();
  });
});
