// Agent mode: tool loop, approvals, editable prompts/skills/instructions, MCP, stop.
// Fake OpenAI-compatible model that speaks native tool calls. Run after `cargo build`.
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { launch, startFixtures, listen, run, sleep } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;

// ── fake model ────────────────────────────────────────────────────────────
const calls = [];
let script = () => ({ text: 'ok' });
const { server: modelServer, port: modelPort } = await listen((req, res) => {
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    if (req.url.endsWith('/models')) { res.setHeader('content-type', 'application/json'); return res.end(JSON.stringify({ data: [{ id: 'fake-model' }] })); }
    const parsed = JSON.parse(body || '{}');
    calls.push(parsed);
    const out = script(parsed.messages || [], parsed, calls.length) || { text: 'ok' };
    setTimeout(() => {
      res.writeHead(200, { 'content-type': 'text/event-stream' });
      const send = (delta, finish) => res.write(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: finish || null }] })}\n\n`);
      if (out.tools) {
        out.tools.forEach((t, i) => send({ tool_calls: [{ index: i, id: `call_${calls.length}_${i}`, type: 'function', function: { name: t.name, arguments: JSON.stringify(t.args || {}) } }] }));
        send({}, 'tool_calls');
      } else {
        for (const c of out.text.match(/[\s\S]{1,12}/g) || ['']) send({ content: c });
        send({}, 'stop');
      }
      res.end('data: [DONE]\n\n');
    }, out.delay || 0);
  });
});

// ── fake MCP server (streamable HTTP, JSON replies) ───────────────────────
const mcpSeen = [];
const { port: mcpPort } = await listen((req, res) => {
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    const m = JSON.parse(body || '{}');
    mcpSeen.push(m.method);
    const reply = (result) => { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify({ jsonrpc: '2.0', id: m.id, result })); };
    if (m.method === 'initialize') return reply({ protocolVersion: '2025-03-26', capabilities: { tools: {} }, serverInfo: { name: 'fake', version: '1' } });
    if (m.method === 'tools/list') return reply({ tools: [
      { name: 'echo', description: 'Echo text back', inputSchema: { type: 'object', properties: { text: { type: 'string' } } }, annotations: { readOnlyHint: true } },
      { name: 'delete_all', description: 'Dangerous', inputSchema: { type: 'object', properties: {} } },
    ] });
    if (m.method === 'tools/call') return reply({ content: [{ type: 'text', text: `echo:${m.params.arguments.text || 'none'}` }] });
    res.statusCode = 202; res.end();
  });
});

const app = await launch({ modelPort, debugPort: 9447 });
let side;
const ui = (fn, arg) => side.evaluate(fn, arg);
const waitFor = async (fn, timeout = 25000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};
const ask = async (text) => {
  const n = await ui(() => window.__agentUi.messages.length);
  await ui((t) => window.__agentUi.send(t), text);
  await waitFor(() => ui((n0) => window.__agentUi.messages.length >= n0 + 2, n));
  return ui(() => window.__agentUi.messages.at(-1));
};
const toolMsg = (messages) => messages.filter((m) => m.role === 'tool');
const mgmt = (resource, operation, params = {}) => ui(([resource, operation, p]) =>
  window.__shell.invoke(resource, { ...p, operation }), [resource, operation, params]);

async function externalClient() {
  const descriptor = JSON.parse(fs.readFileSync(path.join(app.data, 'shell-api.json'), 'utf8'));
  let session, sequence = 0, protocol;
  const request = async (method, params = {}, notification = false) => {
    const response = await fetch(descriptor.url, { method: 'POST', headers: {
      authorization: `Bearer ${descriptor.token}`, 'content-type': 'application/json',
      ...(session ? { 'Mcp-Session-Id': session } : {}), ...(protocol ? { 'MCP-Protocol-Version': protocol } : {}),
    }, body: JSON.stringify({ jsonrpc: '2.0', ...(notification ? {} : { id: ++sequence }), method, params }), signal: AbortSignal.timeout(55000) });
    assert.ok(response.ok, `MCP HTTP ${response.status}`);
    session ||= response.headers.get('mcp-session-id');
    if (notification) return;
    const body = await response.json();
    if (body.error) throw new Error(body.error.message);
    return body.result;
  };
  protocol = (await request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'discussion-test', version: '1' } })).protocolVersion;
  await request('notifications/initialized', {}, true);
  return {
    async agent(args) {
      const result = await request('tools/call', { name: 'shell_agent', arguments: args });
      if (result.isError) throw new Error(result.content.map(item => item.text || '').join('\n'));
      return result.structuredContent;
    },
    async close() { await fetch(descriptor.url, { method: 'DELETE', headers: { authorization: `Bearer ${descriptor.token}`, 'Mcp-Session-Id': session } }); },
  };
}

const cases = [
  ['setup: article tab, agent mode', async () => {
    await app.invoke('shell_page_navigate', { url: `${base}/` });
    await (await app.pageWith(/127\.0\.0\.1:\d+\/$/)).waitForSelector('#btn');
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
    side = await app.pageWith(/sidebar\.html/);
        await side.waitForSelector('#ag-input');
    const st = await mgmt('shell_agent', 'state_get');
    assert.equal(st.prompts.length, 3);
    assert.ok(st.skills.length >= 4);
  }],

  ['tool loop: list_tabs then read_page, final answer streamed', async () => {
    calls.length = 0;
    script = (messages) => {
      const tools = toolMsg(messages);
      if (tools.length === 0) return { tools: [{ name: 'shell_tab', args: { operation: 'list' } }] };
      if (tools.length === 1) return { tools: [{ name: 'shell_page_read' }] };
      return { text: 'The page says: ' + (/Returns policy/.test(tools[1].content) ? 'RETURNS-OK' : 'MISSING') };
    };
    const last = await ask('what is on my current tab?');
    assert.match(last.content, /RETURNS-OK/);
    assert.equal(last.tools.length, 2);
    assert.ok(calls[0].tools.some((t) => t.function.name === 'shell_page_read'));
    assert.match(calls[0].messages[0].content, /browser/i);
  }],

  ['external discussions share sidebar answers, retain history and selection, and safely retry or refresh', async () => {
    const external = await externalClient(), other = await externalClient();
    try {
      const page = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
      const select = async id => {
        await page.evaluate(id => { const range = document.createRange(); range.selectNodeContents(document.getElementById(id)); const selected = getSelection(); selected.removeAllRanges(); selected.addRange(range); document.dispatchEvent(new Event('selectionchange')); }, id);
      };
      await select('p1');
      await waitFor(async () => (await app.invoke('shell_agent', { operation: 'context_get' })).page.selection?.includes('Returns are accepted'));
      calls.length = 0;
      script = messages => ({ delay: 350, text: `RIVER-ROUND-${messages.filter(m => m.role === 'user').length}` });
      const args = { operation: 'discuss', request_id: 'discussion-1', message: 'Explain my selection', wait_ms: 0 };
      const started = await external.agent(args);
      assert.equal(started.status, 'running');
      const retry = await external.agent(args);
      assert.equal(retry.run_id, started.run_id);
      assert.equal(retry.turn_id, started.turn_id);
      const id = started.discussion_id;
      const firstRow = side.locator(`[data-discussion-id="${id}"]`).first();
      await firstRow.waitFor();
      await assert.rejects(other.agent({ operation: 'discussion_wait', discussion_id: id, wait_ms: 0 }), /not found/);
      await assert.rejects(other.agent({ operation: 'discussion_stop', discussion_id: id, wait_ms: 0 }), /not found/);
      const waiting = external.agent({ operation: 'discussion_wait', discussion_id: id, turn_id: 1, wait_ms: 5000 });
      assert.ok((await app.invoke('shell_window', { operation: 'state_get' })).displays.length);
      const first = await waiting;
      assert.equal(first.status, 'completed');
      assert.equal(first.answer, 'RIVER-ROUND-1');
      await waitFor(async () => (await firstRow.locator('.ag-answer').textContent()).trim() === first.answer);
      assert.match(await firstRow.textContent(), /VSCode \/ River/);
      await select('p2');
      await waitFor(async () => (await app.invoke('shell_agent', { operation: 'context_get' })).page.selection?.includes('Shipping is free'));
      const second = await external.agent({ operation: 'discuss', discussion_id: id, request_id: 'discussion-2', message: 'Challenge your first conclusion' });
      assert.equal(second.turn_id, 2);
      assert.equal(second.answer, 'RIVER-ROUND-2');
      assert.match(second.context.selection, /Returns are accepted/);
      assert.ok(calls[1].messages.some(m => m.role === 'assistant' && m.content === first.answer));
      assert.match(calls[1].messages[0].content, /pinned snapshot/);
      assert.match(calls[1].messages[0].content.match(/The user has selected this text[\s\S]*?"""([\s\S]*?)"""/)[1], /Returns are accepted/);
      const third = await external.agent({ operation: 'discuss', discussion_id: id, request_id: 'discussion-3', message: 'Now use the newly selected text', refresh_context: true });
      assert.equal(third.answer, 'RIVER-ROUND-3');
      assert.match(third.context.selection, /Shipping is free/);
      assert.equal(calls.length, 3, 'retries and wait calls must not execute a second model run');
      await waitFor(async () => (await side.locator(`[data-discussion-id="${id}"] .ag-answer`).allTextContents()).at(-1)?.trim() === third.answer);
      const cached = await external.agent({ operation: 'discussion_wait', discussion_id: id, turn_id: 1, wait_ms: 0 });
      assert.equal(cached.answer, first.answer);
      await assert.rejects(external.agent({ ...args, message: 'A different request using the same key' }), /already used/);
    } finally { await Promise.allSettled([external.close(), other.close()]); }
  }],

  ['discussion waiting, stopping and provider errors return explicit terminal results', async () => {
    const external = await externalClient();
    try {
      calls.length = 0;
      script = () => ({ delay: 1500, text: 'SHOULD-BE-STOPPED' });
      const started = await external.agent({ operation: 'discuss', request_id: 'stop-1', message: 'Wait for cancellation', wait_ms: 0 });
      const stillRunning = await external.agent({ operation: 'discussion_wait', discussion_id: started.discussion_id, wait_ms: 1 });
      assert.equal(stillRunning.status, 'running');
      const stopped = await external.agent({ operation: 'discussion_stop', discussion_id: started.discussion_id, wait_ms: 5000 });
      assert.equal(stopped.status, 'stopped');
      assert.equal(stopped.answer, null);
      const row = side.locator(`[data-discussion-id="${started.discussion_id}"]`);
      await waitFor(async () => /Stopped/.test(await row.textContent()));
      const failed = await external.agent({ operation: 'discuss', request_id: 'failure-1', message: 'Use an unavailable model', model: 'does-not-exist/model', wait_ms: 5000 });
      assert.equal(failed.status, 'failed');
      assert.equal(failed.answer, null);
      assert.ok(failed.error);
    } finally { await external.close(); }
  }],

  ['RiverMCP changes migrate a running agent and sidebar without replaying its tool', async () => {
    calls.length = 0;
    script = messages => toolMsg(messages).length === 0
      ? { delay: 1200, tools: [{ name: 'shell_tab', args: { operation: 'list' } }] }
      : { text: 'MIGRATED: ' + toolMsg(messages)[0].content };
    const answer = ask('list my tabs after reconnecting');
    await waitFor(() => calls.length > 0);
    const reservation = await listen((_request, response) => response.end());
    const port = reservation.port;
    await new Promise(resolve => reservation.server.close(resolve));
    await app.invoke('shell_settings', { operation: 'river_mcp_apply', value: { port, token: 'running-agent-migration-token' } });
    const last = await answer;
    assert.match(last.content, /MIGRATED:.*tabs/s);
    assert.equal(last.tools.length, 1);
    assert.equal(calls.length, 2);
    await side.reload();
    await side.waitForSelector('#ag-input');
    assert.equal((await mgmt('shell_settings', 'river_mcp_get')).active_port, port);
  }],

  ['write tool asks for approval: deny', async () => {
    calls.length = 0;
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'shell_page_click', args: { selector: '#btn' } }] }
      : { text: 'got: ' + toolMsg(messages)[0].content.slice(0, 60) };
    const n = await ui(() => window.__agentUi.messages.length);
    await ui((t) => window.__agentUi.send(t), 'click the button');
    await side.waitForSelector('.ag-approve');
    assert.match(await side.textContent('.ag-approve'), /click/i);
    await side.click('.ag-approve .is-danger');
    await waitFor(() => ui((n0) => window.__agentUi.messages.length >= n0 + 2, n));
    const last = await ui(() => window.__agentUi.messages.at(-1));
    assert.match(last.content, /rejected/i);
  }],

  ['auto mode: "open X" navigates the page with no approval', async () => {
    calls.length = 0;
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'shell_page_navigate', args: { url: `${base}/other` } }] }
      : { text: 'opened it' };
    const n = await ui(() => window.__agentUi.messages.length);
    await ui((t) => window.__agentUi.send(t), 'open 新浪新闻');
    await waitFor(() => ui((n0) => window.__agentUi.messages.length >= n0 + 2, n));
    assert.match(calls[0].messages[0].content, /Navigation mode: AUTO/);
    assert.equal(await side.locator('.ag-approve').count(), 0);
    await app.pageWith(/127\.0\.0\.1:\d+\/other/);
  }],

  ['chat mode: "打开 X" is handed to the agent and navigates', async () => {
    await ui(() => document.getElementById('agent-switch').click());
    assert.equal(await ui(() => window.__agentUi.agentOn), false);
    await app.invoke('shell_page_navigate', { url: `${base}/` });
    await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'shell_page_navigate', args: { url: `${base}/other` } }] }
      : { text: 'opened it' };
    await ui((t) => window.__agentUi.send(t), '打开新浪');
    await app.pageWith(/127\.0\.0\.1:\d+\/other/);
    assert.equal(await ui(() => window.__agentUi.agentOn), true);
  }],

  ['agent tool: canvas opens a grid of pages the user can see', async () => {
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'shell_canvas', args: { operation: 'open', cols: 2, rows: 1, urls: [`${base}/?c=1`, `${base}/?c=2`] } }] }
      : { text: 'canvas ready' };
    await ui((t) => window.__agentUi.send(t), 'put two pages side by side');
    await waitFor(async () => (await app.invoke('shell_canvas', { operation: 'state_get' }))?.cells?.filter((c) => c.tab_id != null).length === 2);
    const cv = await app.invoke('shell_canvas', { operation: 'state_get' });
    assert.equal(cv.cols, 2);
    await app.invoke('shell_tab', { operation: 'close', tab_id: cv.id });
  }],

  ['agent: "open X at (1,2)" loads that row/column of the canvas', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 3, rows: 2, urls: [`${base}/?g=0`] });
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'shell_canvas', args: { operation: 'cell_open', url: `${base}/?g=target`, row: 1, col: 2 } }] }
      : { text: 'done' };
    await ui((t) => window.__agentUi.send(t), 'open google at (1,2)');
    await waitFor(async () => (await app.invoke('shell_canvas', { operation: 'state_get' }))?.cells?.[5]?.tab_id != null);
    await waitFor(async () => app.ctx.pages().some((p) => p.url().includes('g=target')));
    assert.match(calls.at(-1).messages[0].content, /# Canvas/);
    await app.invoke('shell_tab', { operation: 'close', tab_id: (await app.invoke('shell_canvas', { operation: 'state_get' })).id });
  }],

  ['canvas container page tools navigate and read its focused visible cell', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2, urls: [0, 1, 2, 3].map(cell => `${base}/?canvas-cell=${cell}`) });
    const canvas = await app.invoke('shell_canvas', { operation: 'state_get' });
    try {
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 2 });
      calls.length = 0;
      const target = `${base}/other?canvas-focused=2`;
      script = messages => {
        const tools = toolMsg(messages);
        if (tools.length === 0) return { tools: [{ name: 'shell_tab', args: { operation: 'list' } }] };
        if (tools.length === 1) return { tools: [{ name: 'shell_page_navigate', args: { tab_id: canvas.id, url: target } }] };
        if (tools.length === 2) return { tools: [{ name: 'shell_page_read', args: { tab_id: canvas.id } }] };
        return { text: 'navigation checked' };
      };
      await ask('in the current cell, open the other page');
      const results = toolMsg(calls.at(-1).messages).map(message => JSON.parse(message.content));
      const listed = results[0].tabs;
      assert.equal(listed.find(tab => tab.tab_id === canvas.id).kind, 'canvas');
      assert.equal(listed.find(tab => tab.tab_id === canvas.id).focused_tab_id, canvas.cells[2].tab_id);
      assert.equal(listed.find(tab => tab.tab_id === canvas.cells[2].tab_id).cell, 2);
      assert.equal(results[1].tab_id, canvas.cells[2].tab_id, 'navigation returns the actual focused page, not the container');
      assert.equal(results[2].url, target, 'read_page of a canvas reads its focused cell');
      const visible = await app.pageWith(/\/other\?canvas-focused=2$/);
      assert.equal(await visible.evaluate(() => document.visibilityState), 'visible');
      const after = await app.invoke('shell_canvas', { operation: 'state_get' });
      assert.equal(after.id, canvas.id);
      assert.equal(after.focus, 2);
      for (let cell = 0; cell < 4; cell++) {
        assert.equal(after.cells[cell].url, cell === 2 ? target : `${base}/?canvas-cell=${cell}`);
      }
      const context = await app.invoke('shell_agent', { operation: 'context_get' });
      assert.equal(context.tabs.find(tab => tab.tab_id === canvas.id).url, 'native://canvas');
      assert.match(calls[0].messages[0].content, new RegExp(`canvas ${canvas.id}, cell 2`));
    } finally {
      await app.invoke('shell_tab', { operation: 'close', tab_id: canvas.id });
    }
  }],

  ['default agent navigation stays in the current cell with another canvas open', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 1, urls: [`${base}/?background=0`, `${base}/?background=1`] });
    const background = await app.invoke('shell_canvas', { operation: 'state_get' });
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2, urls: [0, 1, 2, 3].map(cell => `${base}/?current=${cell}`) });
    const current = await app.invoke('shell_canvas', { operation: 'state_get' });
    try {
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 3 });
      calls.length = 0;
      const target = `${base}/other?current-cell=3`;
      script = messages => toolMsg(messages).length === 0
        ? { tools: [{ name: 'shell_page_navigate', args: { url: target } }] }
        : { text: 'opened in the current cell' };
      await ask('google in the current cell');
      const result = JSON.parse(toolMsg(calls.at(-1).messages)[0].content);
      assert.equal(result.tab_id, current.cells[3].tab_id);
      await app.pageWith(/\/other\?current-cell=3$/);
      const context = await app.invoke('shell_agent', { operation: 'context_get' });
      assert.equal(context.page.tab_id, current.cells[3].tab_id);
      for (let cell = 0; cell < 3; cell++) {
        assert.equal(context.tabs.find(tab => tab.tab_id === current.cells[cell].tab_id).url, `${base}/?current=${cell}`);
      }
      for (let cell = 0; cell < 2; cell++) {
        assert.equal(context.tabs.find(tab => tab.tab_id === background.cells[cell].tab_id).url, `${base}/?background=${cell}`);
      }
    } finally {
      await app.invoke('shell_tab', { operation: 'close', tab_id: current.id });
      await app.invoke('shell_tab', { operation: 'close', tab_id: background.id });
    }
  }],

  ['canvas navigation fills the focused empty cell without creating a hidden page', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 1 });
    const canvas = await app.invoke('shell_canvas', { operation: 'state_get' });
    try {
      calls.length = 0;
      const target = `${base}/other?blank-canvas=1`;
      script = messages => toolMsg(messages).length === 0
        ? { tools: [{ name: 'shell_page_navigate', args: { tab_id: canvas.id, url: target } }] }
        : { text: 'opened in the blank cell' };
      await ask('open the other page in the current blank cell');
      const result = JSON.parse(toolMsg(calls.at(-1).messages)[0].content);
      assert.ok(result.tab_id);
      const page = await app.pageWith(/\/other\?blank-canvas=1$/);
      assert.equal(await page.evaluate(() => document.visibilityState), 'visible');
      const after = await app.invoke('shell_canvas', { operation: 'state_get' });
      assert.equal(result.tab_id, after.cells[0].tab_id);
      assert.equal(after.cells[0].url, target);
      assert.equal(after.cells[1].tab_id, null);
      const context = await app.invoke('shell_agent', { operation: 'context_get' });
      assert.equal(context.tabs.find(tab => tab.tab_id === canvas.id).url, 'native://canvas');
    } finally {
      await app.invoke('shell_tab', { operation: 'close', tab_id: canvas.id });
    }
  }],

  ['manual mode: the pane shows a link, the page does not move until it is clicked', async () => {
    await app.invoke('shell_page_navigate', { url: `${base}/` });
    await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await ui(() => document.getElementById('nav-switch').click());
    calls.length = 0;
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'shell_tab', args: { operation: 'open', url: `${base}/live` } }] }
      : { text: 'link ready' };
    const n = await ui(() => window.__agentUi.messages.length);
    await ui((t) => window.__agentUi.send(t), 'open the live page');
    await waitFor(() => ui((n0) => window.__agentUi.messages.length >= n0 + 2, n));
    assert.match(calls[0].messages[0].content, /Navigation mode: MANUAL/);
    await side.waitForSelector('.ag-link');
    const tabs = await ui(() => window.__shell.invoke('shell_agent', { operation: 'context_get' }));
    assert.ok(!tabs.tabs.some((t) => /\/live/.test(t.url)), 'nothing was opened yet');
    await side.click('.ag-link');
    await app.pageWith(/127\.0\.0\.1:\d+\/live/);
    await ui(() => document.getElementById('nav-switch').click());
  }],

  ['private addresses are refused by fetch_url', async () => {
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'fetch_url', args: { url: `${base}/` } }] }
      : { text: 'result: ' + toolMsg(messages)[0].content };
    const last = await ask('fetch the local page');
    assert.match(last.content, /result: Error/);
  }],

  ['editable prompts: system prompt edit reaches the model, reset restores', async () => {
    await mgmt('shell_agent_prompt', 'save', { name: 'system', text: 'MARKER-SYSTEM-42 today={{date}}' });
    calls.length = 0;
    script = () => ({ text: 'ok' });
    await ask('hello');
    assert.match(calls[0].messages[0].content, /^MARKER-SYSTEM-42 today=\d{4}-\d\d-\d\d/);
    const st = await mgmt('shell_agent_prompt', 'reset', { name: 'system' });
    assert.equal(st.prompts.find((p) => p.name === 'system').customized, false);
    calls.length = 0;
    await ask('hello again');
    assert.doesNotMatch(calls[0].messages[0].content, /MARKER-SYSTEM-42/);
  }],

  ['custom instructions reach the system prompt', async () => {
    await mgmt('shell_agent', 'config_set', { custom_instructions: 'ALWAYS-SAY-PINEAPPLE' });
    calls.length = 0;
    await ask('hi');
    assert.match(calls[0].messages[0].content, /ALWAYS-SAY-PINEAPPLE/);
    await mgmt('shell_agent', 'config_set', { custom_instructions: '' });
  }],

  ['live context: typing and in-page tabs change the context with no URL change', async () => {
    await app.invoke('shell_page_navigate', { url: `${base}/live` });
    const page = await app.pageWith(/127\.0\.0\.1:\d+\/live/);
    await page.waitForSelector('#nm');
    await sleep(800);
    await ui(() => { window.__ctxEvents = []; window.__shell.on('context.changed', (c) => window.__ctxEvents.push(c)); });
    await page.fill('#nm', 'Ada Lovelace');
    await page.fill('#pw', 'hunter2-secret');
    await waitFor(() => ui(() => window.__ctxEvents.some((c) => c.content && c.content.filled >= 1)));
    await page.click('#tab-b');
    await waitFor(() => ui(() => window.__ctxEvents.some((c) => /ZEBRA-PANEL/.test(c.content?.preview || '') || c.content?.chars > 0)));
    const urls = await ui(() => [...new Set(window.__ctxEvents.map((c) => c.page?.url))]);
    assert.equal(urls.length, 1);
    calls.length = 0;
    script = () => ({ text: 'ok' });
    await ask('what did I just do?');
    const system = calls[0].messages[0].content;
    assert.match(system, /Full name \[text\]: Ada Lovelace/);
    assert.match(system, /ZEBRA-PANEL/);
    assert.doesNotMatch(system, /hunter2/);
    assert.match(system, /Secret \[password\]: \(hidden\)/);
  }],

  ['edited context: the text the user wrote replaces the generated context', async () => {
    const preview = await mgmt('shell_agent', 'context_preview', {});
    assert.match(preview.context, /Ada Lovelace/);
    calls.length = 0;
    script = () => ({ text: 'ok' });
    const n = await ui(() => window.__agentUi.messages.length);
    await ui(() => window.__shell.invoke('shell_agent', { operation: 'run', messages: [{ role: 'user', content: 'hi' }], context_override: 'USER-EDITED-CONTEXT-77' }));
    await waitFor(() => calls.length > 0);
    const system = calls[0].messages[0].content;
    assert.match(system, /USER-EDITED-CONTEXT-77/);
    assert.doesNotMatch(system, /ZEBRA-PANEL/);
    await waitFor(() => ui(() => true));
    await sleep(500);
  }],

  ['selection: selecting page text shows it in the pane, and Explain sends it (agent mode)', async () => {
    await app.invoke('shell_page_navigate', { url: `${base}/` });
    const page = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await page.waitForSelector('#p1');
    await sleep(600);
    await page.evaluate(() => {
      const r = document.createRange();
      r.selectNodeContents(document.getElementById('p1'));
      const s = getSelection(); s.removeAllRanges(); s.addRange(r);
    });
    await side.waitForSelector('#sel-card', { timeout: 15000 });
    assert.match(await side.textContent('#sel-card'), /Returns are accepted within 30 days/);
    calls.length = 0;
    script = () => ({ text: 'EXPLAINED' });
    await side.click('#sel-card button:has-text("Explain")');
    await waitFor(() => calls.length > 0);
    const sys = calls[0].messages[0].content;
    assert.match(sys, /Returns are accepted within 30 days/, 'selection reaches the model');
    assert.match(JSON.stringify(calls[0].messages.at(-1)), /Explain the selected text/);
    await waitFor(() => ui(() => window.__agentUi.messages.at(-1)?.content === 'EXPLAINED'));
  }],

  ['selection: toolbar action with the pane open routes into the pane (ask mode too)', async () => {
    await ui(() => document.getElementById('agent-switch').click());
    await side.waitForSelector('#ask-input');
    const page = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await page.evaluate(() => {
      const r = document.createRange();
      r.selectNodeContents(document.getElementById('p2'));
      const s = getSelection(); s.removeAllRanges(); s.addRange(r);
    });
    await side.waitForFunction(() => /Shipping is free/.test(document.getElementById('sel-card')?.textContent || ''), null, { timeout: 15000 });
    calls.length = 0;
    script = () => ({ text: 'TRANSLATED' });
    await app.invoke('shell_ai', { operation: 'selection_run', intent: 'translate' });
    await waitFor(() => calls.length > 0);
    assert.match(JSON.stringify(calls[0].messages), /Shipping is free over \$50/, 'selection reaches the ask pipeline');
    await side.waitForFunction(() => /TRANSLATED/.test(document.getElementById('ask-log').textContent));
    await ui(() => document.getElementById('agent-switch').click());
    await side.waitForSelector('#ag-input');
  }],

  ['context panel: view, edit, and discard when the page changes', async () => {
    await side.click('#ctx-toggle');
    await side.waitForSelector('#ctx-edit');
    await waitFor(() => side.inputValue('#ctx-edit').then((v) => /Returns policy/.test(v)));
    await side.fill('#ctx-edit', 'MY-EDITED-CONTEXT-55');
    calls.length = 0;
    script = () => ({ text: 'ok' });
    await ask('use my context');
    assert.match(calls[0].messages[0].content, /MY-EDITED-CONTEXT-55/);
    await app.invoke('shell_page_navigate', { url: `${base}/other` });
    await app.pageWith(/127\.0\.0\.1:\d+\/other/);
    await waitFor(() => side.inputValue('#ctx-edit').then((v) => !/MY-EDITED-CONTEXT-55/.test(v)));
    calls.length = 0;
    await ask('and now?');
    assert.doesNotMatch(calls[0].messages[0].content, /MY-EDITED-CONTEXT-55/);
  }],

  ['pane resize: width follows the request and is clamped', async () => {
    const w0 = await ui(() => window.innerWidth);
    const r = await ui(() => window.__shell.invoke('shell_ai', { operation: 'pane_resize', width: 600 }));
    assert.equal(r.width, 600);
    await waitFor(() => ui(() => Math.abs(window.innerWidth - 600) <= 1));
    const small = await ui(() => window.__shell.invoke('shell_ai', { operation: 'pane_resize', width: 10 }));
    assert.equal(small.width, 320);
    const big = await ui(() => window.__shell.invoke('shell_ai', { operation: 'pane_resize', width: 5000 }));
    assert.ok(big.width > 900, 'no fixed maximum');
    await ui(() => window.__shell.invoke('shell_ai', { operation: 'pane_resize', width: 420 }));
    await waitFor(() => ui(() => Math.abs(window.innerWidth - 420) <= 1));
    assert.ok(w0 > 0);
    assert.ok(await side.locator('#resize-grip').count());
  }],

  ['skills: create in UI store, always-on injected, auto loaded via fetch_skill', async () => {
    await mgmt('shell_agent_skill', 'save', { name: 'Always Polite', description: 'tone', mode: 'always-on', instructions: 'ALWAYS-POLITE-BODY', triggers: [] });
    await mgmt('shell_agent_skill', 'save', { name: 'Tax Helper', description: 'help with tax forms', mode: 'auto', instructions: 'TAX-STEPS-BODY', triggers: ['tax'] });
    calls.length = 0;
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'fetch_skill', args: { skill_id: 'tax-helper' } }] }
      : { text: 'loaded: ' + (/TAX-STEPS-BODY/.test(toolMsg(messages)[0].content) ? 'yes' : 'no') };
    const last = await ask('help me with my tax form');
    assert.match(calls[0].messages[0].content, /ALWAYS-POLITE-BODY/);
    assert.ok(calls[0].tools.some((t) => t.function.name === 'fetch_skill'));
    assert.match(last.content, /loaded: yes/);
    // Off removes it from the catalog entirely.
    await mgmt('shell_agent_skill', 'mode_set', { id: 'tax-helper', mode: 'off' });
    calls.length = 0;
    script = () => ({ text: 'ok' });
    await ask('tax again');
    assert.doesNotMatch(calls[0].messages[0].content, /Tax Helper/);
    await mgmt('shell_agent_skill', 'delete', { id: 'tax-helper' });
    await mgmt('shell_agent_skill', 'delete', { id: 'always-polite' });
  }],

  ['MCP: add server, tools listed, read-only runs, other asks approval', async () => {
    const st = await mgmt('shell_agent_mcp', 'save', { name: 'Fake', url: `http://127.0.0.1:${mcpPort}/mcp`, trust: false, enabled: true });
    assert.equal(st.mcp.length, 1);
    await waitFor(async () => (await mgmt('shell_agent', 'state_get')).mcp[0].status === 'connected');
    const tools = (await mgmt('shell_agent', 'state_get')).mcp[0].tools.map((t) => t.name);
    assert.deepEqual(tools.sort(), ['delete_all', 'echo']);
    calls.length = 0;
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'mcp_fake__echo', args: { text: 'hi' } }] }
      : { text: 'mcp said ' + toolMsg(messages)[0].content };
    const last = await ask('use the echo tool');
    assert.match(last.content, /echo:hi/);
    assert.ok(calls[0].tools.some((t) => t.function.name === 'mcp_fake__delete_all'));
    // The dangerous one waits for the user.
    script = (messages) => toolMsg(messages).length === 0
      ? { tools: [{ name: 'mcp_fake__delete_all' }] }
      : { text: 'finished ' + toolMsg(messages)[0].content.slice(0, 40) };
    const n = await ui(() => window.__agentUi.messages.length);
    await ui((t) => window.__agentUi.send(t), 'delete everything');
    await side.waitForSelector('.ag-approve');
    await side.click('.ag-approve .is-danger');
    await waitFor(() => ui((n0) => window.__agentUi.messages.length >= n0 + 2, n));
    await mgmt('shell_agent_mcp', 'delete', { id: 'fake' });
  }],

  ['conversation history: saved, listed, loaded', async () => {
    const list = await mgmt('shell_conversation', 'list', { query: '' });
    assert.ok(list.conversations.length >= 1);
    const doc = await mgmt('shell_conversation', 'load', { id: list.conversations[0].id });
    assert.ok(Array.isArray(doc.messages) && doc.messages.length >= 2);
  }],

  ['stop cancels a slow run', async () => {
    script = () => ({ text: 'x'.repeat(40), delay: 15000 });
    const n = await ui(() => window.__agentUi.messages.length);
    await ui((t) => window.__agentUi.send(t), 'take forever');
    await side.waitForSelector('#ag-stop:not([hidden])');
    await side.click('#ag-stop');
    await side.waitForSelector('#ag-send:not([hidden])', { timeout: 10000 });
  }],

  ['settings UI: every editor renders and prompt edit saves from the textarea', async () => {
    await side.getByRole('button', { name: 'Settings', exact: true }).click();
    const settings = await app.pageWith(/pages\/settings\.html/);
    for (const label of ['Skills', 'MCP', 'Prompts', 'Instructions', 'Models', 'General']) {
      await settings.getByRole('tab', { name: label, exact: true }).click();
      await settings.getByRole('heading', { name: label, exact: true }).waitFor();
      assert.ok(await settings.$('.ag-body > *'), label);
    }
    await settings.getByRole('tab', { name: 'Prompts', exact: true }).click();
    const ta = settings.locator('.ag-edit').first();
    await ta.fill('UI-EDITED {{date}}');
    await settings.locator('.ag-card').first().getByRole('button', { name: 'Save', exact: true }).click();
    await waitFor(async () => (await mgmt('shell_agent', 'state_get')).prompts.some((p) => p.customized && /UI-EDITED/.test(p.text)));
    await mgmt('shell_agent_prompt', 'reset', { name: (await mgmt('shell_agent', 'state_get')).prompts.find((p) => p.customized).name });
  }],
];

const failed = await run(cases);
model_close: {
  modelServer.close();
}
await app.close();
fx.close();
process.exit(failed ? 1 : 0);

