// One MCP client for chrome, sidebar, Settings and overlay. Commands and ordered
// state/event updates use the same authenticated server as external agents.

export const COMMANDS = [
  'shell_tab', 'shell_canvas', 'shell_page_read', 'shell_page_navigate',
  'shell_page_click', 'shell_page_fill', 'shell_page_popups_dismiss',
  'shell_page_url_copy', 'shell_page_scroll_to', 'shell_page_snapshot',
  'shell_page_navigation', 'shell_page_zoom', 'shell_page_find',
  'shell_cookie', 'shell_storage', 'shell_tab_group', 'shell_history',
  'shell_bookmark', 'shell_download', 'shell_window', 'shell_settings',
  'shell_omnibox', 'shell_ui', 'shell_diagnostics', 'shell_ai', 'shell_agent',
  'shell_agent_tool', 'shell_agent_skill', 'shell_agent_prompt',
  'shell_agent_mcp', 'shell_conversation',
] as const;

export type Command = (typeof COMMANDS)[number];
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type Payload = any;
export type Listener = (payload: Payload) => void;

export interface ConnectionInfo {
  url: string;
  token: string;
  cursor?: number;
}

export interface ShellClient {
  readonly ready: Promise<void>;
  invoke<T = Payload>(cmd: Command, payload?: object): Promise<T>;
  on(event: string, fn: Listener): () => void;
  readResource<T = Payload>(uri: string): Promise<T>;
  close(): void;
}

interface EventBatch {
  sequence: number;
  reset: boolean;
  events: { sequence: number; event: string; payload: Payload }[];
}

export function createMcpClient(connection?: ConnectionInfo, options: { fetch?: typeof fetch; timeoutMs?: number; reconnectMs?: number; onCursor?: (sequence: number) => void } = {}): ShellClient {
  const fetcher: typeof fetch = options.fetch ?? ((...args) => globalThis.fetch(...args));
  const timeoutMs = options.timeoutMs ?? 150_000;
  let next = 0;
  let session: string | null = null;
  let cursor = connection?.cursor;
  let startup: Promise<void> | undefined;
  let closed = false;
  let generation = 0;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let stream: AbortController | undefined;
  const requests = new Set<AbortController>();
  const listeners = new Map<string, Set<Listener>>();

  function emit(event: string, payload: Payload) {
    for (const listener of [...(listeners.get(event) ?? [])]) {
      try { listener(payload); } catch (error) { console.error(event, error); }
    }
  }

  function headers() {
    return {
      Authorization: `Bearer ${connection?.token ?? ''}`,
      Accept: 'application/json, text/event-stream',
      'MCP-Protocol-Version': '2025-03-26',
      ...(session ? { 'Mcp-Session-Id': session } : {}),
    };
  }

  function deleteSession(id: string) {
    if (!connection) return;
    const controller = new AbortController();
    requests.add(controller);
    const timer = setTimeout(() => controller.abort(), 1000);
    void fetcher(connection.url, {
      method: 'DELETE', headers: { ...headers(), 'Mcp-Session-Id': id },
      signal: controller.signal, keepalive: true,
    }).catch(() => undefined).finally(() => { clearTimeout(timer); requests.delete(controller); });
  }

  async function rpc(method: string, params: object = {}, notification = false): Promise<Payload> {
    if (closed) throw new Error('Shell MCP client closed');
    if (!connection) throw new Error('Shell MCP connection unavailable');
    const controller = new AbortController();
    const id = notification ? undefined : ++next;
    requests.add(controller);
    const timer = setTimeout(() => controller.abort(new Error(`${method} timed out; the operation may have completed`)), timeoutMs);
    try {
      const response = await fetcher(connection.url, {
        method: 'POST', headers: { ...headers(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', ...(notification ? {} : { id }), method, params }),
        signal: controller.signal,
      });
      if (!response.ok) throw new Error(`Shell MCP HTTP ${response.status}`);
      if (method === 'initialize') {
        session = response.headers.get('Mcp-Session-Id');
        if (!session) throw new Error('Shell MCP session missing');
      }
      if (notification) return;
      const reply = await response.json();
      if (reply.jsonrpc !== '2.0' || reply.id !== id) throw new Error('Invalid Shell MCP response');
      if (reply.error) throw new Error(reply.error.message || 'Shell MCP request failed');
      return reply.result;
    } finally {
      clearTimeout(timer);
      requests.delete(controller);
    }
  }

  async function read<T>(uri: string): Promise<T> {
    const result = await rpc('resources/read', { uri });
    const content = result.contents?.find((item: { text?: string }) => typeof item.text === 'string');
    if (!content) throw new Error(`Shell resource has no JSON content: ${uri}`);
    return JSON.parse(content.text) as T;
  }

  type Drain = { dirty: boolean; pending?: Promise<void> };
  let activeDrain: Drain = { dirty: false };
  function drain(): Promise<void> {
    const state = activeDrain;
    state.dirty = true;
    if (state.pending) return state.pending;
    state.pending = (async () => {
      while (state.dirty && !closed && state === activeDrain) {
        state.dirty = false;
        const batch = await read<EventBatch>(`shell://events?after=${cursor ?? 0}`);
        if (closed || state !== activeDrain) return;
        if (batch.reset) {
          cursor = batch.sequence;
          options.onCursor?.(cursor);
          emit('mcp.resync', {});
          continue;
        }
        for (const event of batch.events) {
          if (event.sequence > (cursor ?? 0)) emit(event.event, event.payload);
        }
        cursor = batch.sequence;
        options.onCursor?.(cursor);
      }
    })().finally(() => { state.pending = undefined; });
    return state.pending;
  }

  function reconnect() {
    if (closed || retry !== undefined) return;
    retry = setTimeout(() => {
      retry = undefined;
      void ensure().catch(() => reconnect());
    }, options.reconnectMs ?? 500);
  }

  async function receive(response: Response, current: number, controller: AbortController) {
    const reader = response.body?.getReader();
    if (!reader) throw new Error('Shell MCP stream unavailable');
    const decoder = new TextDecoder();
    let buffered = '';
    try {
      while (!closed && current === generation) {
        const chunk = await reader.read();
        if (chunk.done) break;
        buffered += decoder.decode(chunk.value, { stream: true });
        let boundary;
        while ((boundary = /\r?\n\r?\n/.exec(buffered))) {
          const frame = buffered.slice(0, boundary.index);
          buffered = buffered.slice(boundary.index + boundary[0].length);
          const data = frame.split(/\r?\n/).filter(line => line.startsWith('data:')).map(line => line.slice(5).replace(/^ /, '')).join('\n');
          if (!data) continue;
          const message = JSON.parse(data);
          if (message.method === 'notifications/resources/updated' && message.params?.uri === 'shell://events') {
            void drain().catch(() => { if (current === generation) controller.abort(); });
          }
        }
      }
    } finally {
      controller.abort();
      reader.releaseLock();
      if (!closed && current === generation) {
        startup = undefined;
        reconnect();
      }
    }
  }

  async function connect() {
    const current = ++generation;
    activeDrain = { dirty: false };
    if (session) deleteSession(session);
    session = null;
    await rpc('initialize', { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 'ai-browser-ui', version: '1' } });
    await rpc('notifications/initialized', {}, true);
    if (cursor === undefined) cursor = (await read<EventBatch>('shell://events')).sequence;
    await rpc('resources/subscribe', { uri: 'shell://events' });
    stream?.abort();
    stream = new AbortController();
    const controller = stream;
    const timer = setTimeout(() => controller.abort(new Error('Shell MCP stream timed out')), timeoutMs);
    let response: Response;
    try {
      response = await fetcher(connection!.url, { method: 'GET', headers: { ...headers(), Accept: 'text/event-stream' }, signal: controller.signal });
    } finally { clearTimeout(timer); }
    if (!response.ok) throw new Error(`Shell MCP stream HTTP ${response.status}`);
    if (!response.body) throw new Error('Shell MCP stream unavailable');
    void receive(response, current, controller).catch(() => undefined);
    await drain();
    if (!closed && current === generation) emit('mcp.connected', {});
  }

  function ensure(): Promise<void> {
    if (closed) return Promise.reject(new Error('Shell MCP client closed'));
    if (!startup) {
      const current = generation + 1;
      startup = connect().catch(error => {
        if (current === generation) {
          startup = undefined;
          stream?.abort();
          if (connection) reconnect();
        }
        throw error;
      });
    }
    return startup;
  }

  return {
    get ready() { return ensure(); },
    async invoke<T>(cmd: Command, payload: object = {}): Promise<T> {
      await ensure();
      const result = await rpc('tools/call', { name: cmd, arguments: payload });
      if (result.isError) throw new Error(result.content?.map((item: { text?: string }) => item.text ?? '').join('\n') || 'Shell operation failed');
      if (result.structuredContent !== undefined) return result.structuredContent as T;
      const text = result.content?.find((item: { type: string }) => item.type === 'text')?.text;
      return (text ? JSON.parse(text) : null) as T;
    },
    on(event, fn) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      const mine = set;
      mine.add(fn);
      return () => mine.delete(fn);
    },
    async readResource<T>(uri: string): Promise<T> {
      await ensure();
      return read<T>(uri);
    },
    close() {
      if (closed) return;
      closed = true;
      if (retry !== undefined) clearTimeout(retry);
      stream?.abort();
      for (const request of requests) request.abort();
      if (session) deleteSession(session);
    },
  };
}

export function createAgentCalls(client: Pick<ShellClient, 'invoke'>) {
  return function call<T = Payload>(resource: Command, operation: string, params: object = {}): Promise<T> {
    return client.invoke<T>(resource, { ...params, operation });
  };
}

declare global {
  interface Window {
    __shellMcpBootstrap?: ConnectionInfo;
    __shell?: ShellClient;
    __agentUi?: unknown;
  }
}

export function bridge(): ShellClient {
  if (!window.__shell) {
    let connection = window.__shellMcpBootstrap;
    delete window.__shellMcpBootstrap;
    const key = `shell.mcp.cursor:${window.location.pathname}`;
    try {
      const saved = JSON.parse(window.sessionStorage.getItem(key) ?? 'null');
      if (connection && saved?.url === connection.url && Number.isSafeInteger(saved.cursor) && saved.cursor >= (connection.cursor ?? 0)) {
        connection = { ...connection, cursor: saved.cursor };
      }
    } catch { /* Storage may be unavailable in an embedded document. */ }
    const client = createMcpClient(connection, { onCursor(cursor) {
      try { window.sessionStorage.setItem(key, JSON.stringify({ url: connection?.url, cursor })); }
      catch { /* Cursor recovery still works for the live connection. */ }
    } });
    window.__shell = client;
    window.addEventListener('pagehide', () => client.close(), { once: true });
  }
  return window.__shell;
}
