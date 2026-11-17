// The typed bridge between this page and the browser shell.
//
// WebView2 is out of process, so every command is a message: `window.ipc.postMessage`
// out, `window.__native._dispatch` (events) and `_reply` (command results) back in. The
// command list below is checked against the Rust dispatcher by a test, so a renamed or
// removed command fails the build instead of failing at runtime.

export const COMMANDS = [
  'ai.ask',
  'ai.status',
  'ai.session',
  'ai.models',
  'ai.stop',
  'ai.provider.set',
  'ai.citation.scroll',
  'ai.history.clear',
  'ai.selection.act',
  'ai.pane.resize',
  'nav.open',
  'tab.select',
  'agent.run',
  'agent.stop',
  'agent.approve',
  'agent.call',
  'agent.context',
] as const;

export type Command = (typeof COMMANDS)[number];
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Payload = any;
export type Listener = (payload: Payload) => void;

export interface Bridge {
  invoke<T = Payload>(cmd: Command, payload?: object): Promise<T>;
  on(event: string, fn: Listener): () => void;
}

export interface NativeBridge extends Bridge {
  _dispatch(envelope: { event: string; payload: Payload }): void;
  _reply(reply: { id: number; ok: boolean; data?: Payload; error?: string }): void;
}

/** Replies are addressed by the host to the originating webview. */
export function createBridge(post: (raw: string) => void, firstId = 1, timeoutMs = 150_000): NativeBridge {
  let next = firstId;
  const pending = new Map<number, { resolve: (v: Payload) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
  const listeners = new Map<string, Set<Listener>>();
  return {
    invoke(cmd, payload = {}) {
      const id = next++;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          if (pending.delete(id)) reject(new Error(`${cmd} timed out`));
        }, timeoutMs);
        pending.set(id, { resolve, reject, timer });
        try {
          post(JSON.stringify({ id, cmd, payload }));
        } catch (e) {
          pending.delete(id);
          clearTimeout(timer);
          reject(e);
        }
      });
    },
    on(event, fn) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      const mine = set;
      mine.add(fn);
      return () => mine.delete(fn);
    },
    _dispatch(envelope) {
      const set = envelope && listeners.get(envelope.event);
      if (set) for (const fn of [...set]) fn(envelope.payload);
    },
    _reply(reply) {
      const entry = pending.get(reply.id);
      if (!entry) return;
      pending.delete(reply.id);
      clearTimeout(entry.timer);
      if (reply.ok) entry.resolve(reply.data);
      else entry.reject(new Error(reply.error || 'command failed'));
    },
  };
}

/** Management operations: `agent.call` returns at once and the result arrives later as
 *  the `agent.reply` event with the same `req`. This turns that pair into a promise. */
export function createAgentCalls(bridge: Bridge, timeoutMs = 150_000) {
  let seq = 0;
  const waiting = new Map<number, { resolve: (v: Payload) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
  bridge.on('agent.reply', (r) => {
    const w = waiting.get(r.req);
    if (!w) return;
    waiting.delete(r.req);
    clearTimeout(w.timer);
    if (r.ok) w.resolve(r.value);
    else w.reject(new Error(r.error || 'failed'));
  });
  return function call<T = Payload>(op: string, params: object = {}): Promise<T> {
    const req = ++seq;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        if (waiting.delete(req)) reject(new Error('timed out'));
      }, timeoutMs);
      waiting.set(req, { resolve, reject, timer });
      bridge.invoke('agent.call', { ...params, op, req }).catch((e) => {
        waiting.delete(req);
        clearTimeout(timer);
        reject(e);
      });
    });
  };
}

declare global {
  interface Window {
    ipc?: { postMessage(raw: string): void };
    __native?: NativeBridge;
    __agentUi?: unknown;
  }
}

let installed: NativeBridge | null = null;

/** The page's one bridge. It is also published as `window.__native`, which the shell
 *  calls into and the end-to-end suites drive. */
export function bridge(): NativeBridge {
  if (!installed) {
    installed = createBridge((raw) => {
      if (!window.ipc) throw new Error('ipc bridge unavailable');
      window.ipc.postMessage(raw);
    });
    window.__native = installed;
  }
  return installed;
}
