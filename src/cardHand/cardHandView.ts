import {
  createCardHandState,
  isCardDefinitionList,
  isCardHandResponse,
  mergeHandCards,
  type CardDefinition,
  type CardHandState,
  type HandCard,
  type VisibleHandCard,
} from './cardHandState';

const LOCAL_USER_ID = '00000000-0000-0000-0000-000000000001';

export type CardHandViewOptions = {
  baseUrl?: string;
  token?: string;
  fetchImpl?: typeof fetch;
};

export function mountCardHandView(options: CardHandViewOptions = {}): void {
  const root = document.createElement('section');
  root.className = 'card-hand-shell';
  root.setAttribute('aria-label', 'Card hand');
  root.innerHTML = `
    <div class="card-hand-status" data-card-hand-status>Loading hand</div>
    <div class="card-hand" data-card-hand></div>
  `;
  document.body.appendChild(root);

  const handEl = root.querySelector<HTMLElement>('[data-card-hand]');
  const statusEl = root.querySelector<HTMLElement>('[data-card-hand-status]');
  if (!handEl || !statusEl) return;

  let state = createCardHandState();
  renderCardHand(handEl, statusEl, state);

  void loadPersistedHand(options)
    .then((cards) => {
      state = { status: 'ready', cards, error: null };
      renderCardHand(handEl, statusEl, state);
    })
    .catch((error) => {
      state = {
        status: 'error',
        cards: [],
        error: error instanceof Error ? error.message : String(error),
      };
      renderCardHand(handEl, statusEl, state);
    });
}

async function loadPersistedHand(options: CardHandViewOptions): Promise<VisibleHandCard[]> {
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  if (!fetchImpl) throw new Error('Fetch unavailable');

  const baseUrl = options.baseUrl ?? cardHandBaseUrl();
  const token = options.token ?? localCardHandToken();
  const headers = { authorization: `Bearer ${token}` };
  const [definitionsResponse, handResponse] = await Promise.all([
    fetchImpl(new URL('/cards', baseUrl), { headers }),
    fetchImpl(new URL('/card-hand', baseUrl), { headers }),
  ]);

  if (!definitionsResponse.ok) throw new Error(`Cards HTTP ${definitionsResponse.status}`);
  if (!handResponse.ok) throw new Error(`Hand HTTP ${handResponse.status}`);

  const definitionsPayload: unknown = await definitionsResponse.json();
  const handPayload: unknown = await handResponse.json();
  if (!isCardDefinitionList(definitionsPayload)) throw new Error('Invalid cards payload');
  if (!isCardHandResponse(handPayload)) throw new Error('Invalid hand payload');

  return mergeHandCards(handPayload.cards, definitionsPayload);
}

function renderCardHand(handEl: HTMLElement, statusEl: HTMLElement, state: CardHandState): void {
  statusEl.textContent = state.status === 'ready' ? 'Hand synced' : state.status === 'error' ? `Hand error: ${state.error ?? 'backend unavailable'}` : 'Loading hand';
  statusEl.dataset.status = state.status;
  statusEl.title = state.error ?? '';
  handEl.replaceChildren(...state.cards.map(renderCard));
}

function renderCard(card: VisibleHandCard): HTMLElement {
  const node = document.createElement('article');
  node.className = `card-hand-card card-hand-card-${card.type} card-hand-card-${card.rarity}`;
  node.dataset.cardId = card.id;
  const offset = card.instance_id - 3;
  node.style.setProperty('--hand-y', `${Math.abs(offset) * 2}px`);
  node.style.setProperty('--hand-rot', `${offset * 2.5}deg`);
  node.innerHTML = `
    <div class="card-hand-card-top">
      <span class="card-hand-cost">${escapeHtml(card.mana_cost)}</span>
      <span class="card-hand-name">${escapeHtml(card.name)}</span>
    </div>
    <div class="card-hand-card-body">${escapeHtml(card.description)}</div>
    <div class="card-hand-card-foot">${escapeHtml(card.type)} · ${escapeHtml(card.rarity)}</div>
  `;
  return node;
}

function cardHandBaseUrl(): string {
  const envUrl = import.meta.env.VITE_ABUTOWN_BACKEND_URL;
  return typeof envUrl === 'string' && envUrl.length > 0 ? envUrl : globalThis.location.origin;
}

function localCardHandToken(): string {
  const stored = globalThis.localStorage?.getItem('abutown.card_hand_user_id');
  if (stored) return stored;
  globalThis.localStorage?.setItem('abutown.card_hand_user_id', LOCAL_USER_ID);
  return LOCAL_USER_ID;
}

function escapeHtml(value: unknown): string {
  return String(value ?? '').replace(/[&<>"']/g, (char) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  })[char] ?? char);
}
