// Real native shell control over MCP, without a model provider or AI process.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { performance } from 'node:perf_hooks';
import { launch, startFixtures, run, sleep } from './harness.mjs';

const fixtures = await startFixtures();
const base = `http://127.0.0.1:${fixtures.port}`;
const app = await launch({ debugPort: 16000 + Math.floor(Math.random() * 30000), withProvider: false });
const descriptor = JSON.parse(fs.readFileSync(path.join(app.data, 'shell-api.json'), 'utf8'));
let sequence = 0;
const waitFor = async fn => {
  const end = Date.now() + 20000;
  let last;
  while (Date.now() < end) {
    try { const result = await fn(); if (result) return result; } catch (error) { last = error; }
    await sleep(100);
  }
  throw last || new Error('native API condition timed out');
};
async function client() {
  let session;
  const request = async (method, params = {}, notification = false) => {
    const response = await fetch(descriptor.url, {
      method: 'POST',
      headers: { authorization: `Bearer ${descriptor.token}`, 'content-type': 'application/json',
        accept: 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-03-26',
        ...(session ? { 'Mcp-Session-Id': session } : {}) },
      body: JSON.stringify({ jsonrpc: '2.0', ...(notification ? {} : { id: ++sequence }), method, params }),
      signal: AbortSignal.timeout(25000),
    });
    assert.ok(response.ok, `MCP HTTP ${response.status}`);
    session ||= response.headers.get('mcp-session-id');
    if (notification) return;
    const body = await response.json();
    if (body.error) throw new Error(body.error.message);
    return body.result;
  };
  await request('initialize', { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 'native-e2e', version: '1' } });
  assert.ok(session);
  await request('notifications/initialized', {}, true);
  return {
    request,
    async read(uri) {
      const result = await request('resources/read', { uri });
      return JSON.parse(result.contents.find(item => typeof item.text === 'string').text);
    },
    async updates(uris) {
      for (const uri of uris) await request('resources/subscribe', { uri });
      const controller = new AbortController();
      const response = await fetch(descriptor.url, { method: 'GET', headers: {
        authorization: `Bearer ${descriptor.token}`, 'Mcp-Session-Id': session,
        'MCP-Protocol-Version': '2025-03-26', accept: 'text/event-stream',
      }, signal: controller.signal });
      assert.equal(response.status, 200);
      const notices = [];
      const reader = response.body.getReader();
      const pump = (async () => {
        const decoder = new TextDecoder();
        let buffered = '';
        try {
          while (true) {
            const chunk = await reader.read();
            if (chunk.done) return;
            buffered += decoder.decode(chunk.value, { stream: true });
            let boundary;
            while ((boundary = /\r?\n\r?\n/.exec(buffered))) {
              const frame = buffered.slice(0, boundary.index);
              buffered = buffered.slice(boundary.index + boundary[0].length);
              const data = frame.split(/\r?\n/).filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n');
              if (!data) continue;
              const message = JSON.parse(data);
              if (message.method === 'notifications/resources/updated') notices.push(message.params.uri);
            }
          }
        } catch (error) { if (!controller.signal.aborted) throw error; }
        finally { reader.releaseLock(); }
      })();
      return { notices, async stop() { controller.abort(); await pump; } };
    },
    async call(name, args = {}) {
      const result = await request('tools/call', { name, arguments: args });
      if (result.isError) throw new Error(result.content.map(item => item.text || '').join('\n'));
      return result.structuredContent;
    },
    async close() {
      await fetch(descriptor.url, { method: 'DELETE', headers: { authorization: `Bearer ${descriptor.token}`, 'Mcp-Session-Id': session } });
    },
  };
}
const first = await client();
const second = await client();
let pageId;
let canvasId;
let failed;
try {
  failed = await run([
    ['external clients discover the canonical API without starting AI', async () => {
      const tools = (await first.request('tools/list')).tools;
      assert.ok(tools.some(tool => tool.name === 'shell_canvas'));
      assert.ok(tools.every(tool => tool.name.startsWith('shell_')));
      assert.equal((await first.call('shell_ai', { operation: 'status_get' })).link, 'idle');
      const legacy = { page: 'read_page', reload: 'nav.reload' };
      const unknownOperation = 'not_an_operation';
      await assert.rejects(first.call(legacy.page), /unknown shell API/);
      await assert.rejects(first.call('shell_tab', { operation: unknownOperation }), /unknown operation/);
      await assert.rejects(app.invoke(legacy.reload), /unknown shell API/);
      const response = await fetch(descriptor.url, { method: 'POST', headers: { authorization: 'Bearer invalid' }, body: '{}' });
      assert.equal(response.status, 401);
    }],
    ['resources expose current state and the complete bidirectional interface', async () => {
      const resources = (await first.request('resources/list')).resources.map(resource => resource.uri);
      for (const uri of ['shell://context', 'shell://tabs', 'shell://groups', 'shell://window', 'shell://settings', 'shell://downloads', 'shell://chat', 'shell://events', 'shell://interface']) assert.ok(resources.includes(uri), uri);
      const catalog = await first.read('shell://interface');
      assert.ok(catalog.events.some(event => event.name === 'context.changed' && event.public));
      assert.ok(catalog.events.some(event => event.name === 'overlay.state' && !event.public));
      assert.ok((await first.request('resources/templates/list')).resourceTemplates.some(template => template.uriTemplate === 'shell://events{?after}'));
      const context = await first.read('shell://context');
      assert.ok(context.page);
      assert.ok(Array.isArray(context.tabs));
      assert.ok((await first.read('shell://window')).displays.length > 0);
    }],
    ['overlay connects itself and renders the first palette without a manual client startup', async () => {
      await app.invoke('shell_ui', { operation: 'palette_open' });
      try {
        const overlay = await app.pageWith(/overlay\.html/);
        await overlay.locator('#pal-input').waitFor({ timeout: 5000 });
        await overlay.reload();
        await overlay.locator('#pal-input').waitFor({ timeout: 5000 });
      } finally { await app.invoke('shell_ui', { operation: 'overlay_close_top' }); }
    }],
    ['incomplete external requests do not block UI commands', async () => {
      const address = new URL(descriptor.url);
      const pending = [];
      try {
        for (let index = 0; index < 16; index++) {
          const socket = net.createConnection({ host: address.hostname, port: Number(address.port) });
          pending.push(socket);
          socket.on('error', () => {});
          await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
          socket.write(`POST /mcp HTTP/1.1\r\nHost: ${address.host}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n`);
        }
        const started = performance.now();
        const state = await app.invoke('shell_window', { operation: 'state_get' });
        assert.ok(state.displays.length > 0);
        assert.ok(performance.now() - started < 1000, 'UI waited for incomplete external request timeouts');
      } finally { for (const socket of pending) socket.destroy(); }
    }],
    ['ordinary page selection publishes MCP updates, with replay and unsubscribe', async () => {
      const opened = await app.invoke('shell_tab', { operation: 'open', url: `${base}/?mcp=notifications` });
      const page = await app.pageWith(/mcp=notifications/);
      await page.locator('#p1').waitFor();
      assert.deepEqual(await page.evaluate(() => [typeof window.__shell, typeof window.__shellMcpBootstrap, typeof window.__native]), ['undefined', 'undefined', 'undefined']);
      const updates = await first.updates(['shell://context', 'shell://events']);
      try {
        await waitFor(() => updates.notices.includes('shell://context') && updates.notices.includes('shell://events'));
        const cursor = (await first.read('shell://events')).sequence;
        const count = updates.notices.filter(uri => uri === 'shell://context').length;
        await page.evaluate(() => {
          const range = document.createRange(); range.selectNodeContents(document.getElementById('p1'));
          const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
          document.dispatchEvent(new Event('selectionchange'));
        });
        await waitFor(() => updates.notices.filter(uri => uri === 'shell://context').length > count);
        await waitFor(async () => (await first.read('shell://context')).page.selection?.includes('Returns are accepted'));
        const replay = await first.read(`shell://events?after=${cursor}`);
        assert.ok(replay.events.some(event => event.event === 'context.changed' && event.payload.page.selection?.includes('Returns are accepted')));
        assert.ok(replay.events.every((event, index, list) => index === 0 || event.sequence > list[index - 1].sequence));
        await first.request('resources/unsubscribe', { uri: 'shell://context' });
        const stoppedCount = updates.notices.filter(uri => uri === 'shell://context').length;
        await page.evaluate(() => { getSelection().removeAllRanges(); document.dispatchEvent(new Event('selectionchange')); });
        await waitFor(async () => !(await first.read('shell://context')).page.selection);
        await sleep(100);
        assert.equal(updates.notices.filter(uri => uri === 'shell://context').length, stoppedCount);
        await page.evaluate(() => { window.__mcpPreserved = 'page state survives'; });
        await app.shell.evaluate(async () => {
          const original = window.fetch;
          let connection;
          window.fetch = (input, init) => {
            if (init?.method === 'POST' && init.headers['Mcp-Session-Id']) connection = { url: input, headers: init.headers };
            return original.call(window, input, init);
          };
          try { await window.__shell.invoke('shell_window', { operation: 'state_get' }); }
          finally { window.fetch = original; }
          let off;
          let timer;
          const connected = new Promise((resolve, reject) => {
            timer = setTimeout(() => reject(new Error('UI MCP did not reconnect')), 15000);
            off = window.__shell.on('mcp.connected', resolve);
          });
          try {
            const response = await original.call(window, connection.url, { method: 'DELETE', headers: connection.headers });
            if (!response.ok) throw new Error(`UI session delete failed: ${response.status}`);
            await connected;
          } finally { clearTimeout(timer); off(); }
        });
        await app.invoke('shell_ui', { operation: 'ready' });
        assert.equal(await page.evaluate(() => window.__mcpPreserved), 'page state survives');
        const head = (await first.read('shell://events')).sequence;
        await app.invoke('shell_ui', { operation: 'omnibox_focus' });
        const privateChange = await first.read(`shell://events?after=${head}`);
        assert.ok(privateChange.events.every(event => !event.event.startsWith('shell.') && event.event !== 'overlay.state'));
      } finally {
        await first.request('resources/unsubscribe', { uri: 'shell://events' });
        await updates.stop();
        await app.invoke('shell_tab', { operation: 'close', tab_id: opened.tab_id });
      }
    }],
    ['an external client opens, reads and clicks a concrete native page', async () => {
      const opened = await first.call('shell_tab', { operation: 'open', url: `${base}/?mcp=first` });
      pageId = opened.tab_id;
      const content = await waitFor(async () => {
        const read = await first.call('shell_page_read', { tab_id: pageId });
        return read.text.includes('Returns are accepted') && read;
      });
      assert.ok(content.document_token);
      const snapshot = await first.call('shell_page_snapshot', { document: content.document_token, workspace: content.workspace });
      assert.ok(snapshot.payload.blocks.length > 0);
      await assert.rejects(first.call('shell_page_snapshot', { document: content.document_token, workspace: content.workspace + 1 }), /workspace_mismatch/);
      const page = await app.pageWith(/mcp=first/);
      await first.call('shell_page_scroll_to', { tab_id: pageId, selector: '#far' });
      await waitFor(() => page.evaluate(() => scrollY > 0));
      await first.call('shell_page_scroll_to', { tab_id: pageId, text: 'Returns are accepted' });
      await waitFor(() => page.evaluate(() => scrollY < 100));
      const result = await first.call('shell_page_click', { tab_id: pageId, selector: '#btn' });
      assert.equal(result.dispatched, true);
      assert.equal(await page.locator('#count').textContent(), '1');
    }],
    ['document observations and opened-tab ownership are isolated by MCP session', async () => {
      await assert.rejects(second.call('shell_page_click', { tab_id: pageId, selector: '#btn' }), /read|document changed/);
      await assert.rejects(second.call('shell_tab', { operation: 'close', tab_id: pageId }), /opened by this client/);
      await second.call('shell_page_read', { tab_id: pageId });
      await second.call('shell_page_click', { tab_id: pageId, selector: '#btn' });
      const page = await app.pageWith(/mcp=first/);
      assert.equal(await page.locator('#count').textContent(), '2');
    }],
    ['storage writes use this caller\'s observed document and verify read-back', async () => {
      const read = await first.call('shell_storage', { operation: 'read', storage: 'local', tab_id: pageId });
      const result = await first.call('shell_storage', { operation: 'set', storage: 'local', tab_id: pageId,
        document_token: read.document_token, key: 'mcp-e2e', value: 'native-value' });
      assert.equal(result.verified, true);
      const stored = await first.call('shell_storage', { operation: 'read', storage: 'local', tab_id: pageId, key: 'mcp-e2e' });
      assert.equal(stored.entries[0].value, 'native-value');
      const cookies = await first.call('shell_cookie', { operation: 'list', tab_id: pageId });
      const changed = await first.call('shell_cookie', { operation: 'set', tab_id: pageId, document_token: cookies.document_token,
        name: 'mcp-e2e', value: 'cookie-value', httpOnly: true });
      assert.equal(changed.verified, true);
      assert.ok(changed.cookies.some(cookie => cookie.name === 'mcp-e2e' && cookie.httpOnly));
      assert.equal((await first.call('shell_cookie', { operation: 'delete', tab_id: pageId, document_token: cookies.document_token, name: 'mcp-e2e' })).verified, true);
    }],
    ['navigation invalidates both clients\' prior document observations', async () => {
      await first.call('shell_page_navigate', { tab_id: pageId, url: `${base}/other?mcp=next` });
      const page = await app.pageWith(/mcp=next/);
      await page.locator('h1').waitFor();
      await assert.rejects(first.call('shell_page_click', { tab_id: pageId, selector: 'h1' }), /document changed|read/);
      const read = await waitFor(async () => {
        const read = await first.call('shell_page_read', { tab_id: pageId });
        return read.text.includes('gardening') && read;
      });
      assert.ok(read.document_token);
    }],
    ['external canvas operations leave untouched cells empty and target row/column', async () => {
      const opened = await first.call('shell_canvas', { operation: 'open', cols: 2, rows: 2 });
      canvasId = opened.canvas_id;
      assert.equal(opened.canvas.cells.filter(cell => cell.tab_id !== null).length, 0);
      await first.call('shell_canvas', { operation: 'cell_open', canvas_id: canvasId, row: 1, col: 1, url: `${base}/?mcp=cell` });
      const state = await first.call('shell_canvas', { operation: 'state_get' });
      assert.equal(state.cells[3].url, `${base}/?mcp=cell`);
      assert.equal(state.cells.filter(cell => cell.tab_id !== null).length, 1);
      const read = await waitFor(() => first.call('shell_page_read', { tab_id: canvasId, row: 1, col: 1 }));
      assert.ok(read.text.includes('Returns policy'));
      await first.call('shell_canvas', { operation: 'cell_close', canvas_id: canvasId, row: 1, col: 1 });
      assert.ok((await first.call('shell_canvas', { operation: 'state_get' })).cells.every(cell => cell.tab_id === null));
      assert.equal((await first.call('shell_tab', { operation: 'close', tab_id: canvasId })).closed, canvasId);
    }],
    ['the opener can close its page and reserved task metadata is not exposed', async () => {
      await assert.rejects(first.call('shell_tab', { operation: 'list', _task: {} }), /Reserved task metadata/);
      assert.equal((await first.call('shell_tab', { operation: 'close', tab_id: pageId })).closed, pageId);
      assert.ok(!(await first.call('shell_tab', { operation: 'list' })).tabs.some(tab => tab.tab_id === pageId));
      await first.call('shell_omnibox', { operation: 'query', text: 'mcp' });
      assert.equal((await first.call('shell_ai', { operation: 'status_get' })).link, 'idle');
    }],
  ]);
} finally {
  await Promise.all([first.close(), second.close()]);
  await app.close();
  fixtures.close();
}
process.exitCode = failed ? 1 : 0;
