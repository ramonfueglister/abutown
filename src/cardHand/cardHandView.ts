import { createClient, type Session, type SupabaseClient } from '@supabase/supabase-js';
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

export type CardHandViewOptions = {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  supabaseClient?: SupabaseClient;
};

export function mountCardHandView(options: CardHandViewOptions = {}): void {
  const authRoot = document.createElement('div');
  authRoot.className = 'card-auth-shell';
  authRoot.innerHTML = `
    <span class="card-auth-user" data-card-auth-user></span>
    <button class="card-auth-button" type="button" data-card-auth-button>Login</button>
  `;
  document.body.appendChild(authRoot);

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
  const authButton = authRoot.querySelector<HTMLButtonElement>('[data-card-auth-button]');
  const authUser = authRoot.querySelector<HTMLElement>('[data-card-auth-user]');
  if (!handEl || !statusEl || !authButton || !authUser) return;
  const hand = handEl;
  const status = statusEl;
  const button = authButton;
  const user = authUser;

  let state = createCardHandState();
  let activeSession: Session | null = null;
  const supabase = options.supabaseClient ?? createConfiguredSupabaseClient();
  renderCardHand(hand, status, state);

  if (!supabase) {
    button.disabled = true;
    button.textContent = 'Login unavailable';
    state = { status: 'error', cards: [], error: 'Supabase env missing' };
    renderCardHand(hand, status, state);
    return;
  }

  button.addEventListener('click', () => {
    void handleAuthClick(supabase, activeSession);
  });

  void supabase.auth.getSession().then(({ data }) => {
    void applySession(data.session);
  });
  supabase.auth.onAuthStateChange((_event, session) => {
    void applySession(session);
  });

  async function applySession(session: Session | null): Promise<void> {
    activeSession = session;
    renderAuth(button, user, session);
    if (!session?.access_token) {
      state = { status: 'signed_out', cards: [], error: null };
      renderCardHand(hand, status, state);
      return;
    }

    state = createCardHandState();
    renderCardHand(hand, status, state);
    try {
      const cards = await loadPersistedHand(session.access_token, options);
      state = { status: 'ready', cards, error: null };
    } catch (error) {
      state = {
        status: 'error',
        cards: [],
        error: error instanceof Error ? error.message : String(error),
      };
    }
    renderCardHand(hand, status, state);
  }
}

async function loadPersistedHand(token: string, options: CardHandViewOptions): Promise<VisibleHandCard[]> {
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  if (!fetchImpl) throw new Error('Fetch unavailable');

  const baseUrl = options.baseUrl ?? cardHandBaseUrl();
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
  statusEl.textContent = statusText(state);
  statusEl.dataset.status = state.status;
  statusEl.title = state.error ?? '';
  handEl.replaceChildren(...state.cards.map(renderCard));
}

function statusText(state: CardHandState): string {
  if (state.status === 'ready') return 'Hand synced';
  if (state.status === 'signed_out') return 'Login required';
  if (state.status === 'error') return `Hand error: ${state.error ?? 'backend unavailable'}`;
  return 'Loading hand';
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

function createConfiguredSupabaseClient(): SupabaseClient | null {
  const url = import.meta.env.VITE_SUPABASE_URL;
  const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY;
  if (typeof url !== 'string' || url.length === 0) return null;
  if (typeof anonKey !== 'string' || anonKey.length === 0) return null;
  return createClient(url, anonKey);
}

async function handleAuthClick(supabase: SupabaseClient, session: Session | null): Promise<void> {
  if (session) {
    await supabase.auth.signOut();
    return;
  }

  const email = globalThis.prompt('Email for login link');
  if (!email) return;
  await supabase.auth.signInWithOtp({
    email,
    options: {
      emailRedirectTo: globalThis.location.href,
    },
  });
}

function renderAuth(button: HTMLButtonElement, userEl: HTMLElement, session: Session | null): void {
  if (session) {
    button.disabled = false;
    button.textContent = 'Logout';
    userEl.textContent = session.user.email ?? 'Logged in';
    return;
  }

  button.disabled = false;
  button.textContent = 'Login';
  userEl.textContent = '';
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
