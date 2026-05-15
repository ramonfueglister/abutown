import { describe, expect, it } from 'vitest';
import { resolveCardHandBaseUrl } from '../../src/cardHand/cardHandView';

describe('card hand view backend URL', () => {
  it('uses the live local backend by default', () => {
    expect(resolveCardHandBaseUrl()).toBe('http://127.0.0.1:8080');
  });

  it('allows an explicit backend URL override', () => {
    expect(resolveCardHandBaseUrl('https://backend.example.test')).toBe('https://backend.example.test');
  });
});
