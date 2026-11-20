import { createServer, type ServerResponse } from 'node:http';
import { describe, expect, it, vi } from 'vitest';
import { bridge, createAgentCalls, createMcpClient } from '../src/mcp';

async function fixture(token = 'fixture-token') {
  let sequence = 0;
  let sessions = 0;
  let reset = false;
  let rejectInitialization = false;
  let pausedReplay: { started: () => void; gate: Promise<void> } | undefined;
  const events: { sequence: number; event: string; payload: unknown }[] = [];
  const streams = new Set<ServerResponse>();
  const calls: { method: string; params: Record<string, any> }[] = [];
  let tool: (params: Record<string, any>) => Promise<unknown> = async params => params.arguments;
  const updated = () => {
    const bytes = Buffer.from(`event: message\r\ndata: ${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/resources/updated', params: { uri: 'shell://events', _meta: { note: '\u9009\u533a' } } })}\r\n\r\n`);
    for (const stream of streams) {
      if (stream.writableEnded || stream.destroyed) continue;
      for (let i = 0; i < bytes.length; i += 7) stream.write(bytes.subarray(i, i + 7));
    }
  };
  const server = createServer(async (req, res) => {
    if (req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'text/event-stream' });
      res.write(': connected\n\n');
      streams.add(res);
      res.on('close', () => streams.delete(res));
      updated();
      return;
    }
    if (req.method === 'DELETE') { res.end(); return; }
    let raw = '';
    for await (const chunk of req) raw += chunk;
    const message = JSON.parse(raw);
    calls.push(message);
    expect(req.headers.authorization).toBe(`Bearer ${token}`);
    if (message.method === 'initialize' && rejectInitialization) {
      rejectInitialization = false;
      res.writeHead(503).end();
      return;
    }
    res.setHeader('content-type', 'application/json');
    const reply = (result: unknown) => res.end(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }));
    switch (message.method) {
      case 'initialize':
        res.setHeader('Mcp-Session-Id', `fixture-${++sessions}`);
        reply({ protocolVersion: '2025-03-26', capabilities: { resources: { subscribe: true }, tools: {} } });
        break;
      case 'notifications/initialized':
        res.writeHead(202).end();
        break;
      case 'resources/subscribe': reply({}); break;
      case 'resources/read': {
        const uri = new URL(message.params.uri);
        const after = uri.searchParams.get('after');
        if (after !== null && pausedReplay) {
          const paused = pausedReplay;
          pausedReplay = undefined;
          paused.started();
          await paused.gate;
        }
        const value = { sequence, oldest_sequence: 1, reset: after !== null && reset, events: after === null ? [] : events.filter(event => event.sequence > Number(after)) };
        reset = false;
        reply({ contents: [{ uri: message.params.uri, mimeType: 'application/json', text: JSON.stringify(value) }] });
        break;
      }
      case 'tools/call': {
        if (message.params.arguments.operation === 'stop') reply({ isError: true, content: [{ type: 'text', text: 'operation refused' }] });
        else reply({ isError: false, structuredContent: await tool(message.params), content: [] });
        break;
      }
      default: res.writeHead(400).end();
    }
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('test server unavailable');
  return {
    connection: { url: `http://127.0.0.1:${address.port}/mcp`, token }, calls,
    migrate(connection: object) {
      for (const stream of streams) stream.write(`event: message\ndata: ${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/river/connection_changed', params: connection })}\n\n`);
    },
    get sessions() { return sessions; },
    set tool(next: typeof tool) { tool = next; },
    failInitialization() { rejectInitialization = true; },
    pauseReplay() {
      let release!: () => void;
      let started!: () => void;
      const waiting = new Promise<void>(resolve => { started = resolve; });
      const gate = new Promise<void>(resolve => { release = resolve; });
      pausedReplay = { started, gate };
      return { waiting, release };
    },
    publish(event: string, payload: unknown) { events.push({ sequence: ++sequence, event, payload }); updated(); },
    disconnect() { for (const stream of streams) stream.end(); streams.clear(); },
    gap() { reset = true; sequence += 1000; updated(); },
    async close() {
      for (const stream of streams) stream.end();
      server.closeAllConnections();
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
  };
}

describe('shared UI MCP client', () => {
  it('migrates a native UI connection without replaying a write already in flight', async () => {
    const first = await fixture();
    const second = await fixture('replacement-token');
    const changed = vi.fn();
    const connected = vi.fn();
    const client = createMcpClient({ ...first.connection, client_id: 'native-view' }, { onConnection: changed });
    client.on('mcp.connected', connected);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    first.tool = async () => { await gate; return { saved: true }; };
    try {
      await client.ready;
      const pending = client.invoke('shell_settings', { operation: 'set', key: 'theme', value: 'dark' });
      await vi.waitFor(() => expect(first.calls.filter(call => call.method === 'tools/call')).toHaveLength(1));
      first.migrate({ ...second.connection, revision: 2 });
      await vi.waitFor(() => expect(connected).toHaveBeenCalledTimes(2));
      expect(changed).toHaveBeenCalledWith({ ...second.connection, client_id: 'native-view', revision: 2 });
      release();
      await expect(pending).resolves.toEqual({ saved: true });
      expect(second.calls.filter(call => call.method === 'tools/call')).toHaveLength(0);
      expect(second.calls.find(call => call.method === 'initialize')?.params._meta).toEqual({ 'river.client_id': 'native-view' });
    } finally { release(); client.close(); await first.close(); await second.close(); }
  });

  it('restores a migrated endpoint on reload even when the native bootstrap predates Apply', async () => {
    const first = await fixture();
    const second = await fixture('replacement-token');
    const previous = window.__shell;
    const bootstrap = { ...first.connection, client_id: 'native-view', revision: 1, cursor: 0 };
    const key = `shell.mcp.cursor:${window.location.pathname}`;
    let client;
    let replacement;
    try {
      window.__shell = undefined;
      window.__shellMcpBootstrap = bootstrap;
      client = bridge();
      await client.ready;
      first.migrate({ ...second.connection, revision: 2 });
      await vi.waitFor(() => expect(second.sessions).toBe(1));
      await client.ready;
      client.close();
      window.__shell = undefined;
      window.__shellMcpBootstrap = bootstrap;
      replacement = bridge();
      await replacement.ready;
      expect(first.sessions).toBe(1);
      expect(second.sessions).toBe(2);
    } finally {
      client?.close(); replacement?.close(); window.__shell = previous;
      window.sessionStorage.removeItem(key); await first.close(); await second.close();
    }
  });

  it('initializes, subscribes and returns correlated management results directly', async () => {
    const server = await fixture();
    const client = createMcpClient(server.connection);
    try {
      await client.ready;
      const call = createAgentCalls(client);
      const first = call('shell_agent_tool', 'enabled_set', { id: 'shell_cookie', enabled: false });
      const second = call('shell_conversation', 'load', { id: 'saved-chat' });
      await expect(first).resolves.toEqual({ id: 'shell_cookie', enabled: false, operation: 'enabled_set' });
      await expect(second).resolves.toEqual({ id: 'saved-chat', operation: 'load' });
      expect(server.calls.filter(call => call.method === 'tools/call').map(call => call.params.name)).toEqual(['shell_agent_tool', 'shell_conversation']);
      expect(server.calls.find(call => call.method === 'resources/subscribe')?.params).toEqual({ uri: 'shell://events' });
      await expect(client.invoke('shell_ai', { operation: 'stop' })).rejects.toThrow('operation refused');
    } finally { client.close(); await server.close(); }
  });

  it('delivers ordered events through fragmented SSE and unsubscribes local listeners', async () => {
    const server = await fixture();
    server.publish('sidebar.selection', { intent: 'old-event' });
    const client = createMcpClient(server.connection);
    const seen = vi.fn();
    const old = vi.fn();
    client.on('sidebar.selection', old);
    const off = client.on('context.changed', seen);
    try {
      await client.ready;
      server.publish('context.changed', { page: { selection: '\u4f60\u597d' } });
      await vi.waitFor(() => expect(seen).toHaveBeenCalledExactlyOnceWith({ page: { selection: '\u4f60\u597d' } }));
      expect(old).not.toHaveBeenCalled();
      off();
      server.publish('context.changed', { page: { selection: 'next' } });
      await vi.waitFor(() => expect(server.calls.filter(call => call.method === 'resources/read').length).toBeGreaterThanOrEqual(4));
      expect(seen).toHaveBeenCalledTimes(1);
    } finally { client.close(); await server.close(); }
  });

  it('reconnects and catches up missed events without repeating browser operations', async () => {
    const server = await fixture();
    const client = createMcpClient(server.connection, { reconnectMs: 5 });
    const seen: string[] = [];
    client.on('ai.delta', payload => seen.push(payload.text));
    try {
      await client.ready;
      await client.invoke('shell_tab', { operation: 'open', url: 'https://example.test' });
      server.publish('ai.delta', { text: 'before' });
      await vi.waitFor(() => expect(seen).toEqual(['before']));
      server.disconnect();
      server.publish('ai.delta', { text: 'during' });
      await vi.waitFor(() => expect(server.sessions).toBe(2));
      await vi.waitFor(() => expect(seen).toEqual(['before', 'during']));
      expect(server.calls.filter(call => call.method === 'tools/call')).toHaveLength(1);
    } finally { client.close(); await server.close(); }
  });

  it('replays events published after a trusted UI bootstrap was issued', async () => {
    const server = await fixture();
    server.publish('overlay.state', { open: true, kind: 'palette' });
    const client = createMcpClient({ ...server.connection, cursor: 0 });
    const seen = vi.fn();
    client.on('overlay.state', seen);
    try {
      await client.ready;
      expect(seen).toHaveBeenCalledExactlyOnceWith({ open: true, kind: 'palette' });
    } finally { client.close(); await server.close(); }
  });

  it('recovers when the first handshake fails', async () => {
    const server = await fixture();
    server.failInitialization();
    const client = createMcpClient(server.connection, { reconnectMs: 5 });
    const connected = vi.fn();
    client.on('mcp.connected', connected);
    try {
      await expect(client.ready).rejects.toThrow('HTTP 503');
      await vi.waitFor(() => expect(connected).toHaveBeenCalledTimes(1));
      await expect(client.invoke('shell_ai', { operation: 'status_get' })).resolves.toEqual({ operation: 'status_get' });
    } finally { client.close(); await server.close(); }
  });

  it('does not execute old transient commands again when a UI document reloads', async () => {
    const server = await fixture();
    const previous = window.__shell;
    delete window.__shell;
    window.__shellMcpBootstrap = { ...server.connection, cursor: 0 };
    const first = bridge();
    const firstClose = vi.fn();
    first.on('shell.ctrl_w', firstClose);
    let second: ReturnType<typeof bridge> | undefined;
    try {
      await first.ready;
      server.publish('shell.ctrl_w', {});
      await vi.waitFor(() => expect(firstClose).toHaveBeenCalledTimes(1));
      first.close();
      delete window.__shell;
      window.__shellMcpBootstrap = { ...server.connection, cursor: 0 };
      second = bridge();
      const reloadedClose = vi.fn();
      second.on('shell.ctrl_w', reloadedClose);
      await second.ready;
      expect(reloadedClose).not.toHaveBeenCalled();
      server.publish('shell.ctrl_w', {});
      await vi.waitFor(() => expect(reloadedClose).toHaveBeenCalledTimes(1));
    } finally {
      first.close(); second?.close();
      window.__shell = previous;
      window.sessionStorage.removeItem(`shell.mcp.cursor:${window.location.pathname}`);
      await server.close();
    }
  });

  it('does not let an old in-flight replay block a replacement connection', async () => {
    const server = await fixture();
    const paused = server.pauseReplay();
    const client = createMcpClient({ ...server.connection, cursor: 0 }, { reconnectMs: 5 });
    const connected = vi.fn();
    const changed = vi.fn();
    client.on('mcp.connected', connected);
    client.on('context.changed', changed);
    const initial = client.ready;
    void initial.catch(() => undefined);
    try {
      await paused.waiting;
      server.disconnect();
      server.publish('context.changed', { page: { selection: 'while reconnecting' } });
      await vi.waitFor(() => expect(connected).toHaveBeenCalledTimes(1));
      expect(changed).toHaveBeenCalledExactlyOnceWith({ page: { selection: 'while reconnecting' } });
      paused.release();
      await initial;
      expect(connected).toHaveBeenCalledTimes(1);
      expect(changed).toHaveBeenCalledTimes(1);
    } finally { paused.release(); client.close(); await server.close(); }
  });

  it('signals state resynchronization instead of applying an incomplete replay', async () => {
    const server = await fixture();
    const client = createMcpClient(server.connection);
    const resync = vi.fn();
    client.on('mcp.resync', resync);
    try {
      await client.ready;
      server.gap();
      await vi.waitFor(() => expect(resync).toHaveBeenCalledTimes(1));
    } finally { client.close(); await server.close(); }
  });

  it('does not retry a write when its HTTP result is lost', async () => {
    const server = await fixture();
    const client = createMcpClient(server.connection, { timeoutMs: 30 });
    try {
      await client.ready;
      server.tool = async () => new Promise(resolve => setTimeout(() => resolve({ tab_id: 1 }), 100));
      await expect(client.invoke('shell_tab', { operation: 'open' })).rejects.toThrow(/timed out|abort/i);
      expect(server.calls.filter(call => call.method === 'tools/call')).toHaveLength(1);
    } finally { client.close(); await server.close(); }
  });
});
