export type CardDefinition = {
  id: string;
  name: string;
  type: string;
  mana_cost: number;
  description: string;
  rarity: string;
};

export type HandCard = {
  instance_id: number;
  card_id: string;
};

export type CardHandResponse = {
  user_id: string;
  cards: HandCard[];
};

export type VisibleHandCard = CardDefinition & {
  instance_id: number;
};

export type CardHandState = {
  status: 'loading' | 'ready' | 'error';
  cards: VisibleHandCard[];
  error: string | null;
};

export function createCardHandState(): CardHandState {
  return {
    status: 'loading',
    cards: [],
    error: null,
  };
}

export function mergeHandCards(handCards: HandCard[], definitions: CardDefinition[]): VisibleHandCard[] {
  const defs = new Map(definitions.map((definition) => [definition.id, definition]));
  return handCards.map((card) => {
    const definition = defs.get(card.card_id);
    if (!definition) throw new Error(`Missing card definition: ${card.card_id}`);
    return {
      ...definition,
      instance_id: card.instance_id,
    };
  });
}

export function isCardDefinitionList(value: unknown): value is CardDefinition[] {
  return Array.isArray(value) && value.every(isCardDefinition);
}

export function isCardHandResponse(value: unknown): value is CardHandResponse {
  return (
    isObject(value) &&
    isString(value.user_id) &&
    Array.isArray(value.cards) &&
    value.cards.every(isHandCard)
  );
}

function isCardDefinition(value: unknown): value is CardDefinition {
  return (
    isObject(value) &&
    isString(value.id) &&
    isString(value.name) &&
    isString(value.type) &&
    isNumber(value.mana_cost) &&
    isString(value.description) &&
    isString(value.rarity)
  );
}

function isHandCard(value: unknown): value is HandCard {
  return isObject(value) && isNumber(value.instance_id) && isString(value.card_id);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isString(value: unknown): value is string {
  return typeof value === 'string';
}

function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}
