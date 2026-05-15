import { describe, expect, it } from 'vitest';
import {
  createCardHandState,
  isCardDefinitionList,
  isCardHandResponse,
  mergeHandCards,
} from '../../src/cardHand/cardHandState';

describe('card hand state', () => {
  it('accepts valid card definitions and hand responses', () => {
    expect(isCardDefinitionList([
      {
        id: 'strike',
        name: 'Strike',
        type: 'attack',
        mana_cost: 1,
        description: 'Deal damage.',
        rarity: 'starter',
      },
    ])).toBe(true);

    expect(isCardHandResponse({
      user_id: '00000000-0000-0000-0000-000000000001',
      cards: [{ instance_id: 1, card_id: 'strike' }],
    })).toBe(true);
  });

  it('rejects hand cards without authoritative definitions', () => {
    expect(() => mergeHandCards(
      [{ instance_id: 1, card_id: 'strike' }, { instance_id: 2, card_id: 'missing' }],
      [{ id: 'strike', name: 'Strike', type: 'attack', mana_cost: 1, description: 'Deal damage.', rarity: 'starter' }],
    )).toThrow('Missing card definition: missing');
  });

  it('starts in a loading state with no cards', () => {
    expect(createCardHandState()).toEqual({
      status: 'loading',
      cards: [],
      error: null,
    });
  });
});
