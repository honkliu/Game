// Application state and the actions that change it. One transcript holds both kinds of
// turn: with the Agent switch on a message runs the tool loop (`agent.run`); off, it goes
// through the cited-answer pipeline (`shell_ai` ask). The shell owns the facts (page, tabs,
// selection, content); this module only mirrors them and decides what the user sees.

import { batch, computed, signal } from '@preact/signals';
import { bridge, createAgentCalls } from './ipc';
import { contextSignature, echoOf, reconcileEdit, type ContextEdit } from './context';
import { modelCommand } from './model_commands';
import {
  agentStatusLine, failureLine, fromMessages, newAgentTurn, newAskTurn, reduceAgent, reduceAnswer, toMessages,
} from './reducers';
import type {
  AgentContext, AgentEvent, AgentState, AiStatus, AskTurn, Candidate, ConversationRow, ListedModel, ModelInfo, Turn,
} from './types';

export const native = bridge();
export const call = createAgentCalls(native);

export type View = 'chat' | 'settings' | 'history';
export type SettingsTab = 'tools' | 'skills' | 'mcp' | 'prompts' | 'instructions' | 'model' | 'general' | 'history' | 'downloads' | 'chats';
export type Intent = 'ask' | 'summarize' | 'explain' | 'verify' | 'translate' | 'rewrite' | 'act';

const MAX_TURNS = 100;

const store = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

export const agentOn = signal(store()?.getItem('agentOn') !== '0');
/** auto: the assistant navigates the page itself. manual: it only offers a link. */
export const navAuto = signal(store()?.getItem('navAuto') !== '0');
export const view = signal<View>('chat');
export const settingsTab = signal<SettingsTab>('general');
export const turns = signal<Turn[]>([]);
export const status = signal<{ text: string; kind?: 'warn' | 'error' }>({ text: '' });
export const ctx = signal<AgentContext | null>(null);
export const aiStatus = signal<AiStatus | null>(null);
export const models = signal<ModelInfo[]>([]);
export const candidates = signal<Candidate[]>([]);
export const agentState = signal<AgentState | null>(null);
export const focusTick = signal(0);
export const promptHistory = signal<string[]>((() => {
  try {
    const saved: unknown = JSON.parse(store()?.getItem('promptHistory') ?? '[]');
    return Array.isArray(saved) ? saved.filter((text): text is string => typeof text === 'string' && !!text.trim()).slice(-MAX_TURNS) : [];
  } catch {
    return [];
  }
})());

export const intent = signal<Intent>('ask');
export const model = signal(store()?.getItem('model') ?? '');
export const modelMenu = signal(false);
export const modelFilter = signal('');
export function setModel(id: string) {
  model.value = id;
  store()?.setItem('model', id);
}
window.addEventListener('storage', event => { if (event.key === 'model') model.value = event.newValue ?? ''; });
export const useSelection = signal(false);
export const localOnly = signal(false);
export const extraTabs = signal<number[]>([]);

export interface ContextPanel {
  open: boolean;
  loading: boolean;
  context: string;
  system: string;
  error?: string;
  /** Signature of the page state the preview was rendered from. */
  baseSig: string;
}
export const ctxPanel = signal<ContextPanel>({ open: false, loading: false, context: '', system: '', baseSig: '' });
export const ctxEdit = signal<ContextEdit | null>(null);
export const ctxNotice = signal('');
/** When the shell last reported new page content (for the "updated just now" label). */
export const contentUpdatedAt = signal(0);

export const busy = computed(() => turns.value.some((t) => (t.kind === 'agent' ? t.status === 'running' : t.status === 'pending')));

let loadedSkills: string[] = [];
let convId: string | null = null;
let seq = 0;
const nextId = () => ++seq;
let sessionSeq = 0;

function observeSessionEvent(payload: { state_seq?: number }) {
  if (typeof payload.state_seq === 'number') sessionSeq = Math.max(sessionSeq, payload.state_seq);
}

export async function syncSession() {
  try {
    const snapshot = await native.invoke<{ sequence: number; turns: Turn[] }>('shell_ai', { operation: 'session_get' });
    if (snapshot.sequence < sessionSeq) return;
    sessionSeq = snapshot.sequence;
    const previous = turns.value;
    const restored = snapshot.turns.map((turn) => {
      const old = previous.find((t) => t.kind === turn.kind && (
        t.kind === 'agent' && turn.kind === 'agent' ? t.runId === turn.runId && t.runId !== 0 :
          t.kind === 'ask' && turn.kind === 'ask' && t.turnId === turn.turnId
      ));
      return { ...turn, id: old?.id ?? nextId() };
    });
    const saved = previous.filter((t) => t.kind === 'agent' && !t.runId && t.status !== 'running');
    turns.value = [...saved, ...restored].slice(-MAX_TURNS);
    if (!busy.value) setStatus('');
  } catch {
    /* A closing host cannot provide a new session snapshot. */
  }
}

export const setStatus = (text: string, kind?: 'warn' | 'error') => {
  status.value = { text, kind };
};
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));
const focusInput = () => {
  focusTick.value++;
};

function updateTurn(id: number, f: (t: Turn) => Turn) {
  turns.value = turns.value.map((t) => (t.id === id ? f(t) : t));
}
function pushTurn(t: Turn) {
  turns.value = [...turns.value, t].slice(-MAX_TURNS);
}

export const messages = () => toMessages(turns.value);

// ── context ──────────────────────────────────────────────────────────────

let previewTimer = 0;
export async function loadPreview(depth = 0) {
  const sig = contextSignature(ctx.value);
  ctxPanel.value = { ...ctxPanel.value, open: true, loading: true, error: undefined };
  try {
    const r = await call<{ context: string; system: string }>('shell_agent', 'context_preview', { loaded_skills: loadedSkills });
    ctxPanel.value = { open: ctxPanel.value.open, loading: false, context: r.context, system: r.system, baseSig: sig };
    // The page moved while this was rendering; render again (a few times at most).
    if (ctxPanel.value.open && depth < 3 && contextSignature(ctx.value) !== sig) void loadPreview(depth + 1);
  } catch (e) {
    ctxPanel.value = { ...ctxPanel.value, loading: false, error: 'Could not render the context: ' + errText(e) };
  }
}

export function toggleContextPanel() {
  const open = !ctxPanel.value.open;
  ctxPanel.value = { ...ctxPanel.value, open };
  if (open) void loadPreview();
}

/** The user typed in the context box. Text equal to the rendered context is "no edit". */
export function editContext(text: string) {
  ctxNotice.value = '';
  ctxEdit.value = text === ctxPanel.value.context ? null : { text, sig: ctxPanel.value.baseSig };
}

export function discardContextEdit() {
  ctxEdit.value = null;
  ctxNotice.value = '';
}

/** Take a new context (pushed by the shell or fetched). Edits tied to an older page state
 *  are dropped, and an open preview is rendered again from the fresh page. */
export function applyContext(next: AgentContext) {
  const prev = contextSignature(ctx.value);
  const sig = contextSignature(next);
  if (next.content?.at && next.content.at !== ctx.value?.content?.at) contentUpdatedAt.value = next.content.at;
  ctx.value = next;
  if (prev === sig) return;
  const { edit, discarded } = reconcileEdit(ctxEdit.value, sig);
  ctxEdit.value = edit;
  if (discarded) ctxNotice.value = 'The page changed, so your edit of the context was discarded. The context follows the page again.';
  if (ctxPanel.value.open) {
    window.clearTimeout(previewTimer);
    previewTimer = window.setTimeout(() => void loadPreview(), 250);
  }
}

export async function refreshContext() {
  try {
    applyContext(await native.invoke<AgentContext>('shell_agent', { operation: 'context_get' }));
  } catch {
    /* the host is closing */
  }
}

export async function refreshStatus() {
  try {
    const s = await native.invoke<AiStatus>('shell_ai', { operation: 'status_get' });
    aiStatus.value = s;
    applyStatus(s);
  } catch {
    /* ignore */
  }
}

export function applyStatus(s: AiStatus) {
  aiStatus.value = s;
  if (s.models?.length) applyModels(s.models);
  else if (s.provider?.connections) {
    models.value = s.provider.connections.flatMap(connection => connection.models.map(entry => ({
      ...entry, display_name: entry.display_name || entry.model_id,
      provider_id: connection.id, available: connection.enabled,
    })));
  }
  const had = useSelection.peek();
  if (!s.selection) useSelection.value = false;
  else if (!had) useSelection.value = true;
  const live = new Set((s.other_tabs ?? []).map((t) => t.tab_id));
  const kept = extraTabs.peek().filter((id) => live.has(id));
  if (kept.length !== extraTabs.peek().length) extraTabs.value = kept;
}

export function applyModels(entries: ListedModel[]) {
  models.value = entries.map(entry => ({
    ...entry.manifest, local: entry.manifest.execution === 'local',
    available: entry.available, unavailable_reason: entry.unavailable_reason,
  }));
}

export async function refreshAgentState() {
  try {
    agentState.value = await call<AgentState>('shell_agent', 'state_get');
  } catch {
    /* engine not reachable yet */
  }
}

// ── sending ──────────────────────────────────────────────────────────────

/** "open X" / "打开X": needs the agent's tools, so chat mode hands these turns to the agent. */
export const NAV_INTENT = /^\s*(open|go to|goto|visit|navigate to|take me to|打开|访问|前往|跳转到?|去)\s*\S/i;

export async function sendMessage(text: string, over?: { intent: Intent; selection?: boolean; question?: string }) {
  if (busy.value) return;
  if (text.trim() && promptHistory.value.at(-1) !== text) {
    promptHistory.value = [...promptHistory.value, text].slice(-MAX_TURNS);
    try { store()?.setItem('promptHistory', JSON.stringify(promptHistory.value)); } catch { /* History is optional when storage is full. */ }
  }
  const command = modelCommand(text, models.value, agentOn.value);
  if (command) {
    if (command.kind === 'select') {
      setModel(command.id);
      modelMenu.value = false;
      setStatus(`Model: ${command.label}`);
      focusInput();
    } else {
      modelFilter.value = command.query;
      modelMenu.value = true;
      setStatus(command.error || 'Choose a model below.');
    }
    return;
  }
  if (!agentOn.value && !over && NAV_INTENT.test(text)) setAgentOn(true);
  if (agentOn.value) await sendAgent(text.trim());
  else await sendAsk(over ? (over.question ?? '') : text.trim(), over);
}

async function sendAgent(content: string) {
  if (!content) return;
  const edit = ctxEdit.value;
  const turn = { ...newAgentTurn(nextId(), content), echo: echoOf(ctx.value, !!edit) };
  pushTurn(turn);
  setStatus('Starting…');
  try {
    const r = await native.invoke<{ run_id: number }>('shell_agent', { operation: 'run',
      messages: toMessages(turns.value).map((m) => ({ role: m.role, content: m.content })),
      loaded_skills: loadedSkills,
      model: model.value || undefined,
      nav_mode: navAuto.value ? 'auto' : 'manual',
      ...(edit ? { context_override: edit.text } : {}),
    });
    updateTurn(turn.id, (t) => (t.kind === 'agent' && t.status === 'running' && !t.runId ? { ...t, runId: r.run_id } : t));
  } catch (e) {
    updateTurn(turn.id, (t) => (t.kind === 'agent' ? { ...t, status: 'error', error: `Could not start: ${errText(e)}` } : t));
    setStatus('');
  }
}

async function sendAsk(question: string, over?: { intent: Intent; selection?: boolean }) {
  const useIntent = over ? over.intent : intent.value;
  const useSel = over ? !!over.selection : useSelection.value;
  if (!question && !(useSel && useIntent !== 'ask' && useIntent !== 'act')) return;
  let res: { accepted?: boolean; reason?: string; turn_id: number; task_id?: number };
  try {
    res = await native.invoke('shell_ai', { operation: 'ask',
      question, intent: useIntent, selection: useSel, model: model.value || undefined,
      local_only: localOnly.value, tabs: extraTabs.value,
    });
  } catch (e) {
    return setStatus(errText(e) || 'the host did not accept the question', 'error');
  }
  if (!res?.accepted) return setStatus(res?.reason || 'the host did not accept the question', 'error');
  candidates.value = [];
  // The `ai.answer` events carry this turn id and fill the turn in: one question, one card.
  if (!turns.value.some((t) => t.kind === 'ask' && t.turnId === res.turn_id)) {
    pushTurn(newAskTurn(nextId(), res.turn_id, question || useIntent));
  }
  setStatus('Thinking…');
}

export async function stop() {
  const running = turns.value.find((t) => t.kind === 'agent' && t.status === 'running');
  try {
    if (running) await native.invoke('shell_agent', { operation: 'stop' });
    else {
      const pending = turns.value.find((t): t is AskTurn => t.kind === 'ask' && t.status === 'pending');
      await native.invoke('shell_ai', { operation: 'stop', turn_id: pending?.turnId });
    }
    setStatus('Stopping…');
  } catch (e) {
    setStatus(errText(e) || 'nothing to stop', 'warn');
  }
}

export function approve(callId: string, allow: boolean) {
  turns.value = turns.value.map((t) =>
    t.kind === 'agent'
      ? { ...t, tools: t.tools.map((r) => (r.id === callId && r.approval ? { ...r, approval: { ...r.approval, answered: allow ? 'allowed' : 'denied' } } : r)) }
      : t,
  );
  native.invoke('shell_agent', { operation: 'approval_resolve', call_id: callId, allow }).catch(() => undefined);
}

export function setNavAuto(on: boolean) {
  navAuto.value = on;
  store()?.setItem('navAuto', on ? '1' : '0');
}

export function openLink(url: string) {
  native.invoke('shell_page_navigate', { url }).catch((e) => setStatus(errText(e), 'error'));
}

export function setAgentOn(on: boolean) {
  agentOn.value = on;
  store()?.setItem('agentOn', on ? '1' : '0');
  if (view.value === 'history' && !on) view.value = 'chat';
  focusInput();
}

// ── conversations ────────────────────────────────────────────────────────

function titleOf(): string {
  const first = messages().find((m) => m.role === 'user');
  return first ? first.content.replace(/\s+/g, ' ').slice(0, 60) : 'Conversation';
}

async function autosave() {
  const list = messages();
  if (!list.length) return;
  if (!convId) convId = 'c' + Date.now().toString(36);
  try {
    await call('shell_conversation', 'save', { id: convId, doc: { title: titleOf(), updated: Date.now(), messages: list, loaded_skills: loadedSkills } });
  } catch (e) {
    setStatus('Could not save this conversation: ' + errText(e), 'warn');
  }
}

export function newChat() {
  if (busy.value) return;
  batch(() => {
    turns.value = [];
    candidates.value = [];
    view.value = 'chat';
  });
  convId = null;
  loadedSkills = [];
  setStatus('');
  native.invoke('shell_ai', { operation: 'history_clear' }).catch(() => undefined);
  focusInput();
}

export async function listConversations(query: string): Promise<ConversationRow[]> {
  return (await call<{ conversations?: ConversationRow[] }>('shell_conversation', 'list', { query })).conversations ?? [];
}

export async function deleteConversation(id: string) {
  await call('shell_conversation', 'delete', { id });
}

export async function openConversation(id: string) {
  if (busy.value) return;
  try {
    const doc = await call<{ messages?: never[]; loaded_skills?: string[] }>('shell_conversation', 'load', { id });
    await native.invoke('shell_ai', { operation: 'history_clear' });
    turns.value = fromMessages(Array.isArray(doc.messages) ? doc.messages : [], nextId);
    loadedSkills = Array.isArray(doc.loaded_skills) ? doc.loaded_skills : [];
    convId = id;
    view.value = 'chat';
  } catch (e) {
    setStatus(errText(e), 'warn');
  }
}

export async function saveAsSkill() {
  const list = messages();
  if (!list.length) return setStatus('Have a conversation first, then save it as a skill.', 'warn');
  setStatus('Drafting a skill from this conversation…');
  try {
    const draft = await call('shell_agent_skill', 'distill', { messages: list.map((m) => ({ role: m.role, content: m.content })) });
    setStatus('');
    await native.invoke('shell_settings', { operation: 'open', section: 'skills', skill_draft: draft });
  } catch (e) {
    setStatus('Could not draft a skill: ' + errText(e), 'error');
  }
}
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const skillDraft = signal<any>(null);

export function openSettings(tab: SettingsTab) {
  native.invoke('shell_settings', { operation: 'open', section: tab }).catch(error => setStatus(errText(error), 'error'));
}

// ── selection routing ────────────────────────────────────────────────────

export const SELECTION_PROMPTS: Record<string, string> = {
  explain: 'Explain the selected text.',
  verify: 'Verify the claims in the selected text. Search the web if you need to, and cite sources.',
  translate: 'Translate the selected text into English (or into the other language if it is already English).',
  rewrite: 'Rewrite the selected text to be clearer and more concise.',
};

/** The user acted on a selection in the page while this pane is open. The pane decides
 *  which pipeline answers: the agent in agent mode, the cited-answer pipeline otherwise. */
export async function onSidebarSelection(which: string | undefined) {
  view.value = 'chat';
  await refreshContext();
  await refreshStatus();
  if (busy.value) return;
  if (!which || which === 'ask') {
    focusInput();
    setStatus('Your selection is attached. Ask your question.');
    return;
  }
  if (agentOn.value) await sendAgent(SELECTION_PROMPTS[which] ?? `Help with the selected text (${which}).`);
  else await sendAsk('', { intent: which as Intent, selection: true });
}

// ── events from the shell ────────────────────────────────────────────────

let started = false;
export function startStore() {
  if (started) return;
  started = true;
  const on = native.on.bind(native);

  on('agent.event', (ev: AgentEvent) => {
    observeSessionEvent(ev);
    const running = turns.value.find((t) => t.kind === 'agent' && t.status === 'running' && (!t.runId || t.runId === ev.run_id));
    if (!running) return;
    updateTurn(running.id, (t) => (t.kind === 'agent' ? reduceAgent(t, ev) : t));
    const line = agentStatusLine(ev);
    if (line) setStatus(line, ev.type === 'approval' ? 'warn' : undefined);
    if (ev.type === 'done' || ev.type === 'error') {
      if (ev.type === 'done' && Array.isArray(ev.loaded_skills)) loadedSkills = ev.loaded_skills;
      setStatus('');
      void autosave();
      focusInput();
    }
  });

  on('ai.delta', (p) => {
    observeSessionEvent(p);
    turns.value = turns.value.map((t) => (t.kind === 'ask' && t.turnId === p?.turn_id && t.status === 'pending' ? { ...t, streamed: t.streamed + String(p.text || '') } : t));
  });
  on('ai.stage', (p) => {
    observeSessionEvent(p);
    if (p?.stage === 'routed') {
      const label = `${p.model || 'model'} · ${p.local ? 'on this device' : 'cloud'}`;
      turns.value = turns.value.map((t) => (t.kind === 'ask' && t.turnId === p.turn_id ? { ...t, routeLabel: label } : t));
      setStatus(`Routed to ${label}`);
    } else if (typeof p?.stage === 'string') setStatus(p.stage.replace(/_/g, ' ') + '…');
  });
  on('ai.answer', (p) => {
    if (!p || typeof p.turn_id !== 'number') return;
    observeSessionEvent(p);
    if (!turns.value.some((t) => t.kind === 'ask' && t.turnId === p.turn_id)) pushTurn(newAskTurn(nextId(), p.turn_id, p.question || ''));
    turns.value = turns.value.map((t) => (t.kind === 'ask' && t.turnId === p.turn_id ? reduceAnswer(t, p) : t));
    const t = turns.value.find((x): x is AskTurn => x.kind === 'ask' && x.turnId === p.turn_id);
    if (!t) return;
    if (t.status === 'pending') setStatus('Thinking…');
    else if (t.status === 'error') setStatus(failureLine(t.failure) || 'No answer.', 'error');
    else {
      const n = t.citations.length;
      setStatus(n ? `${n} citation${n === 1 ? '' : 's'} — click one to scroll the page there.` : '');
    }
    if (t.status !== 'pending') {
      void autosave();
      focusInput();
    }
  });
  on('ai.answer.stale', (p) => {
    observeSessionEvent(p);
    const ids: number[] = p?.turn_ids ?? [];
    if (!ids.length) return;
    turns.value = turns.value.map((t) => (t.kind === 'ask' && ids.includes(t.turnId) ? { ...t, stale: true, staleStatus: p.status || 'stale' } : t));
    setStatus('The page changed; these citations no longer point at it.', 'warn');
  });
  on('ai.models', (p) => {
    applyModels(p?.models ?? []);
  });
  on('ai.context', (p: AiStatus) => {
    if (p) applyStatus(p);
  });
  on('ai.link', (p) => {
    if (p?.state === 'down') setStatus('The AI engine stopped. The browser is unaffected; ask again to restart it.', 'warn');
  });
  on('ai.candidate', (p: Candidate) => {
    if (p) candidates.value = [...candidates.value, p];
  });
  on('ai.history.cleared', (p) => {
    observeSessionEvent(p);
    turns.value = turns.value.filter((t) => t.kind !== 'ask');
    candidates.value = [];
  });
  on('ai.focus', focusInput);
  on('context.changed', (p: AgentContext) => {
    if (p) applyContext(p);
    void refreshStatus();
  });
  for (const ev of ['tab.created', 'tab.closed', 'tab.updated', 'tab.navigated', 'state.tabs']) {
    on(ev, () => {
      void refreshContext();
      void refreshStatus();
    });
  }
  on('sidebar.selection', (p) => void onSidebarSelection(p?.intent));
  const pendingConversation = async () => {
    try {
      const { id } = await native.invoke<{ id?: string }>('shell_ai', { operation: 'conversation_pending_get' });
      if (id) await openConversation(id);
    } catch (e) { setStatus(errText(e), 'error'); }
  };
  on('conversation.open', () => void pendingConversation());
  on('state.settings', (p) => {
    const s = p || {};
    document.documentElement.dataset.theme = (s.theme || 'system') === 'system' ? (s.system_dark ? 'dark' : 'light') : s.theme;
  });
  window.addEventListener('focus', () => {
    void syncSession();
    void refreshContext();
    void refreshStatus();
  });

  window.__agentUi = {
    send: (text: string) => sendMessage(text),
    get agentOn() {
      return agentOn.value;
    },
    get messages() {
      return messages();
    },
    get state() {
      return agentState.value;
    },
  };

  void (async () => {
    await Promise.all([refreshContext(), refreshStatus(), refreshAgentState(), syncSession()]);
    native.invoke('shell_ai', { operation: 'models_list' }).catch(() => undefined);
    await pendingConversation();
  })();
}
