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

  it('merges hand cards with definitions and keeps unknown cards visible', () => {
    const merged = mergeHandCards(
      [{ instance_id: 1, card_id: 'strike' }, { instance_id: 2, card_id: 'missing' }],
      [{ id: 'strike', name: 'Strike', type: 'attack', mana_cost: 1, description: 'Deal damage.', rarity: 'starter' }],
    );

    expect(merged).toEqual([
      expect.objectContaining({ instance_id: 1, id: 'strike', name: 'Strike' }),
      expect.objectContaining({ instance_id: 2, id: 'missing', name: 'missing' }),
    ]);
  });

  it('starts in a loading state with no cards', () => {
    expect(createCardHandState()).toEqual({
      status: 'loading',
      cards: [],
      error: null,
    });
  });
});
