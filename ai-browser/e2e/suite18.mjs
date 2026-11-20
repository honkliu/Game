// Real native shell control over MCP, without a model provider or AI process.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
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
      await assert.rejects(first.call('read_page'), /unknown shell API/);
      await assert.rejects(first.call('shell_tab', { operation: 'not_an_operation' }), /unknown operation/);
      await assert.rejects(app.invoke('nav.reload'), /unknown shell API/);
      const response = await fetch(descriptor.url, { method: 'POST', headers: { authorization: 'Bearer invalid' }, body: '{}' });
      assert.equal(response.status, 401);
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
