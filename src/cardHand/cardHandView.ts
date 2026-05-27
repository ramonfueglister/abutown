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
import { resolveBackendBaseUrl } from '../backend/backendGate';

export type CardHandViewOptions = {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  supabaseClient?: SupabaseClient;
};

export function mountCardHandView(options: CardHandViewOptions = {}): void {
  const supabase = options.supabaseClient ?? createConfiguredSupabaseClient();
  if (!supabase) return;

  const authRoot = document.createElement('div');
  authRoot.className = 'card-auth-shell';
  authRoot.innerHTML = `
    <span class="card-auth-user" data-card-auth-user></span>
    <button class="card-auth-button" type="button" data-card-auth-button>Login</button>
    <form class="card-auth-form" data-card-auth-form hidden>
      <input class="card-auth-input" type="email" autocomplete="email" placeholder="Email" aria-label="Email for login link" data-card-auth-email required />
      <button class="card-auth-button" type="submit" data-card-auth-submit>Send</button>
      <button class="card-auth-button card-auth-cancel" type="button" data-card-auth-cancel>Cancel</button>
    </form>
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
  const authForm = authRoot.querySelector<HTMLFormElement>('[data-card-auth-form]');
  const authEmail = authRoot.querySelector<HTMLInputElement>('[data-card-auth-email]');
  const authSubmit = authRoot.querySelector<HTMLButtonElement>('[data-card-auth-submit]');
  const authCancel = authRoot.querySelector<HTMLButtonElement>('[data-card-auth-cancel]');
  if (!handEl || !statusEl || !authButton || !authUser || !authForm || !authEmail || !authSubmit || !authCancel) return;
  const hand = handEl;
  const status = statusEl;
  const button = authButton;
  const user = authUser;
  const form = authForm;
  const email = authEmail;
  const submit = authSubmit;
  const cancel = authCancel;

  let state = createCardHandState();
  let activeSession: Session | null = null;
  renderCardHand(hand, status, state);

  button.addEventListener('click', () => {
    if (activeSession) {
      button.disabled = true;
      void supabase.auth.signOut()
        .catch((error: unknown) => {
          user.textContent = 'Logout failed';
          user.title = error instanceof Error ? error.message : String(error);
        })
        .finally(() => {
          button.disabled = false;
        });
      return;
    }

    showLoginForm(button, form, email);
  });

  form.addEventListener('submit', (event) => {
    event.preventDefault();
    void submitInlineEmailLogin(supabase, {
      button,
      form,
      email,
      submit,
      user,
    });
  });

  cancel.addEventListener('click', () => {
    hideLoginForm(button, form, email);
    user.textContent = '';
    user.title = '';
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
  statusEl.textContent = cardHandStatusText(state);
  statusEl.dataset.status = state.status;
  statusEl.title = state.error ?? '';
  statusEl.hidden = !isCardHandStatusVisible(state);
  handEl.replaceChildren(...state.cards.map(renderCard));
}

export function cardHandStatusText(state: CardHandState): string {
  if (state.status === 'ready') return 'Hand synced';
  if (state.status === 'signed_out') return '';
  if (state.status === 'error') return `Hand error: ${state.error ?? 'backend unavailable'}`;
  return 'Loading hand';
}

export function isCardHandStatusVisible(state: CardHandState): boolean {
  return state.status !== 'signed_out';
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
  return resolveCardHandBaseUrl(import.meta.env.VITE_ABUTOWN_BACKEND_URL);
}

export function resolveCardHandBaseUrl(envUrl?: unknown): string {
  return resolveBackendBaseUrl(envUrl);
}

function createConfiguredSupabaseClient(): SupabaseClient | null {
  const url = import.meta.env.VITE_SUPABASE_URL;
  const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY;
  if (typeof url !== 'string' || url.length === 0) return null;
  if (typeof anonKey !== 'string' || anonKey.length === 0) return null;
  return createClient(url, anonKey);
}

export function buildOtpLoginPayload(emailValue: string, redirectTo: string): {
  email: string;
  options: { emailRedirectTo: string };
} | null {
  const email = emailValue.trim();
  if (!email) return null;
  return {
    email,
    options: {
      emailRedirectTo: redirectTo,
    },
  };
}

type InlineLoginControls = {
  button: HTMLButtonElement;
  form: HTMLFormElement;
  email: HTMLInputElement;
  submit: HTMLButtonElement;
  user: HTMLElement;
};

async function submitInlineEmailLogin(supabase: SupabaseClient, controls: InlineLoginControls): Promise<void> {
  const payload = buildOtpLoginPayload(controls.email.value, globalThis.location.href);
  if (!payload) {
    controls.email.focus();
    return;
  }

  controls.email.disabled = true;
  controls.submit.disabled = true;
  controls.user.textContent = 'Sending link';
  controls.user.title = '';

  try {
    await supabase.auth.signInWithOtp(payload);
    controls.user.textContent = 'Check your email';
    controls.email.value = '';
    hideLoginForm(controls.button, controls.form, controls.email);
  } catch (error) {
    controls.user.textContent = 'Login failed';
    controls.user.title = error instanceof Error ? error.message : String(error);
  } finally {
    controls.email.disabled = false;
    controls.submit.disabled = false;
  }
}

function showLoginForm(button: HTMLButtonElement, form: HTMLFormElement, email: HTMLInputElement): void {
  button.hidden = true;
  form.hidden = false;
  email.focus();
}

function hideLoginForm(button: HTMLButtonElement, form: HTMLFormElement, email: HTMLInputElement): void {
  form.hidden = true;
  button.hidden = false;
  email.disabled = false;
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
