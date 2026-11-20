import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { launch, listen, run, sleep, startFixtures } from './harness.mjs';

const waitFor = async (fn, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const result = await fn(); if (result) return result; await sleep(75); }
  throw new Error('timed out waiting for condition');
};
const fixtures = await startFixtures();
const requests = [];
const mcpRequests = [];
const api = await listen((req, res) => {
  let text = '';
  req.on('data', chunk => { text += chunk; });
  req.on('end', () => {
    res.setHeader('content-type', 'application/json');
    if (req.url === '/mcp') {
      const message = JSON.parse(text || '{}');
      mcpRequests.push(message);
      const result = message.method === 'initialize'
        ? { protocolVersion: '2025-03-26', capabilities: { tools: {} }, serverInfo: { name: 'fixture', version: '1' } }
        : message.method === 'tools/list'
          ? { tools: [{ name: 'echo', description: 'Echo fixture', inputSchema: { type: 'object', properties: { text: { type: 'string' } } }, annotations: { readOnlyHint: true } }] }
          : message.method === 'tools/call' ? { content: [{ type: 'text', text: 'MCP-ECHO-OK' }] } : {};
      if (message.id === undefined) { res.statusCode = 202; res.end(); }
      else res.end(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }));
      return;
    }
    requests.push({ method: req.method, url: req.url, body: text ? JSON.parse(text) : null });
    res.end(JSON.stringify(req.url === '/v1/search'
      ? { documents: [{ document_id: 'doc /1', title: 'Search fixture' }] }
      : { content: 'COMPLETE-DOCUMENT-CONTENT' }));
  });
});
const calls = [];
let search = false;
let aliases = false;
const query = '树先生 "quoted" & details';
const model = await listen((req, res) => {
  let text = '';
  req.on('data', chunk => { text += chunk; });
  req.on('end', () => {
    res.setHeader('content-type', 'application/json');
    if (req.url.endsWith('/models')) return res.end(JSON.stringify({ data: [{ id: 'fake-model' }] }));
    const body = JSON.parse(text);
    calls.push(body);
    const tools = body.messages.filter(message => message.role === 'tool');
    let delta;
    const aliasCalls = [
      ['read_current_page', {}],
      ['read_skill_definition', { skill_id: 'research' }],
      ['moonshot_search', { query }],
      ['configured_echo', { text: 'configured argument' }],
      ['propose_page', { url: `http://127.0.0.1:${fixtures.port}/other?manual-alias=1` }],
    ];
    if (aliases && tools.length < aliasCalls.length) {
      const [name, args] = aliasCalls[tools.length];
      delta = { tool_calls: [{ index: 0, id: `tool_${calls.length}`, type: 'function',
        function: { name, arguments: JSON.stringify(args) } }] };
    } else if (aliases) {
      const content = tools.map(tool => tool.content).join('\n');
      delta = { content: content.includes('MCP-ECHO-OK') && content.includes('Manual mode:') &&
        content.includes('Other page') && content.includes('# Skill:') &&
        !content.includes('Unknown tool') ? 'ALIASES-OK' : 'ALIASES-FAILED' };
    } else if (search && tools.length < 2) {
      const name = tools.length ? 'get_search_document' : 'web_search';
      const args = tools.length ? { document_id: 'doc /1' } : { query };
      delta = { tool_calls: [{ index: 0, id: `tool_${calls.length}`, type: 'function',
        function: { name, arguments: JSON.stringify(args) } }] };
    } else delta = { content: search && tools.at(-1)?.content.includes('COMPLETE-DOCUMENT-CONTENT') ? 'MOONSHOT-OK' : 'CONFIGURED-ONLY' };
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    res.write(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: null }] })}\n\n`);
    res.write(`data: ${JSON.stringify({ choices: [{ delta: {}, finish_reason: delta.tool_calls ? 'tool_calls' : 'stop' }] })}\n\n`);
    res.end('data: [DONE]\n\n');
  });
});
const reservation = await listen((_req, res) => res.end());
const debugPort = reservation.port;
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ modelPort: model.port, debugPort });
const base = `http://127.0.0.1:${fixtures.port}`;
const apiBase = `http://127.0.0.1:${api.port}`;
let side, chat, workingId;
const mgmt = (resource, operation, payload = {}) => side.evaluate(([resource, operation, payload]) => new Promise((resolve, reject) => {
  const req = 1000000 + Math.floor(Math.random() * 1000000);
  const off = window.__native.on('agent.reply', result => {
    if (result.req !== req) return;
    off();
    result.ok ? resolve(result.value) : reject(new Error(result.error));
  });
  window.__native.invoke(resource, { ...payload, operation, req }).catch(error => { off(); reject(error); });
}), [resource, operation, payload]);
const ask = async text => {
  const count = await chat.evaluate(() => window.__agentUi.messages.length);
  await chat.evaluate(text => window.__agentUi.send(text), text);
  return waitFor(() => chat.evaluate(count => {
    const messages = window.__agentUi.messages;
    return messages.length >= count + 2 ? messages.at(-1) : null;
  }, count));
};
const openTools = async () => {
  const context = await app.invoke('shell_agent', { operation: 'context_get' });
  if (context.page.url !== 'native://settings') workingId = context.page.tab_id;
  await chat.getByRole('button', { name: 'Settings', exact: true }).click();
  side = await app.pageWith(/pages\/settings\.html/);
  await side.waitForFunction(() => document.querySelector('#settings-general')?.getAttribute('aria-selected') === 'true');
  await side.getByRole('tab', { name: 'Tools', exact: true }).click();
  await side.locator('[data-tool-id="web_search"]').waitFor();
};
const backToChat = async () => {
  await app.invoke('shell_tab', { operation: 'activate', tab_id: workingId });
  side = chat;
};
const inspectColors = () => app.shell.evaluate(() => {
  const rail = document.querySelector('#rail');
  const active = rail.querySelector('.tab-row[aria-selected="true"]');
  const inactive = rail.querySelector('.tab-row[aria-selected="false"]');
  return {
    id: Number(active?.dataset.tabId),
    strip: getComputedStyle(rail).backgroundColor,
    line: getComputedStyle(rail).borderBottomColor,
    lineWidth: getComputedStyle(rail).borderBottomWidth,
    pixelRatio: devicePixelRatio,
    gap: active ? rail.getBoundingClientRect().bottom - parseFloat(getComputedStyle(rail).borderBottomWidth) -
      active.getBoundingClientRect().bottom : null,
    active: active ? getComputedStyle(active).backgroundColor : null,
    inactive: inactive ? getComputedStyle(inactive).backgroundColor : null,
    canvas: !!active?.classList.contains('is-canvas'),
  };
});

let failed = 0;
try {
  await app.shell.waitForFunction(() => !!window.__native);
  failed = await run([
    ['River and 洛 labels retain the existing icon in the toolbar and address suggestions', async () => {
      const cdp = await app.ctx.newCDPSession(app.shell);
      const original = await app.shell.evaluate(() => ({ userAgent: navigator.userAgent, acceptLanguage: navigator.language }));
      try {
        for (const [language, name] of [['en-US', 'River'], ['zh-CN', '洛']]) {
          await cdp.send('Emulation.setUserAgentOverride', { userAgent: original.userAgent, acceptLanguage: language });
          await app.shell.reload();
          assert.equal(await app.shell.evaluate(() => navigator.language), language);
          await app.shell.getByRole('button', { name, exact: true }).waitFor();
          assert.equal((await app.shell.locator('#btn-ai').textContent()).replace(/\s+/g, ' ').trim(), `✦ ${name}`);
          await app.shell.locator('#omnibox').fill('@ai brand check');
          await app.shell.locator('.dropdown-sub').filter({ hasText: `${name}: brand check` }).waitFor();
          assert.equal(await app.shell.locator('#mode-chip').innerText(), `✨ ${name}`);
          await app.shell.locator('#omnibox').press('Escape');
        }
      } finally {
        await cdp.send('Emulation.setUserAgentOverride', original);
        await cdp.detach();
      }
      await app.shell.reload();
      await app.shell.waitForFunction(() => !!window.__native);
    }],
    ['only a thin tab-strip bottom line follows the active tab in light/dark themes and canvas tabs', async () => {
      const ids = [];
      for (let index = 0; index < 2; index++) {
        await app.invoke('shell_tab', { operation: 'open', url: `${base}/live?color=${index}`, background: false });
        ids.push((await app.invoke('shell_agent', { operation: 'context_get' })).page.tab_id);
      }
      const backgrounds = [];
      for (const id of ids) {
        await app.invoke('shell_tab', { operation: 'activate', tab_id: id });
        await waitFor(async () => (await inspectColors()).id === id);
        for (const theme of ['light', 'dark']) {
          await app.shell.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
          const colors = await inspectColors();
          assert.equal(colors.line, colors.active);
          assert.ok(Math.abs(parseFloat(colors.lineWidth) - 3) <= 1 / colors.pixelRatio,
            '3 CSS pixel line allows native device-pixel rounding');
          assert.notEqual(colors.strip, colors.active, 'empty space stays neutral');
          assert.ok(Math.abs(colors.gap) < 0.02, 'active header touches the colored line without a dark gap');
          assert.notEqual(colors.inactive, colors.active);
          backgrounds.push(colors.active);
        }
      }
      assert.notEqual(backgrounds[0], backgrounds[2], 'switching tabs changes the active hue');
      await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 1, urls: [`${base}/live?cell=0`, `${base}/other?cell=1`] });
      await waitFor(async () => (await inspectColors()).canvas);
      const canvas = await inspectColors();
      assert.equal(canvas.line, canvas.active, 'uses the selected canvas header, not its focused child hue');
      assert.notEqual(canvas.strip, canvas.active);
      assert.ok(Math.abs(canvas.gap) < 0.02);
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 1 });
      assert.equal((await inspectColors()).line, canvas.line);
    }],
    ['Tools exposes migrated browser, HTTP and MCP tools and saves disable switches', async () => {
      await app.shell.locator('#btn-ai').click();
      side = await app.pageWith(/sidebar\.html/);
      chat = side;
      await side.waitForSelector('#ag-input');
      const state = await mgmt('shell_agent', 'state_get');
      assert.ok(state.tools.some(tool => tool.id === 'shell_page_read' && tool.source === 'browser' && tool.enabled));
      assert.ok(state.tools.some(tool => tool.id === 'fetch_url' && tool.source === 'local'));
      assert.ok(state.tools.some(tool => tool.id === 'web_search' && tool.http.url.includes('duckduckgo')));
      await mgmt('shell_agent_mcp', 'save', { id: 'fixture', name: 'Fixture', url: `${apiBase}/mcp`, enabled: true });
      await openTools();
      await side.locator('[data-tool-id="mcp_fixture__echo"]').waitFor();
      for (const id of ['fetch_url', 'web_search', 'mcp_fixture__echo']) {
        await side.getByRole('checkbox', { name: `Enable ${id}`, exact: true }).uncheck();
        await waitFor(async () => !(await mgmt('shell_agent', 'state_get')).tools.find(tool => tool.id === id).enabled);
      }
      await backToChat();
      const answer = await ask('Show only configured tools.');
      assert.match(answer.content, /CONFIGURED-ONLY/);
      const names = calls.at(-1).tools.map(tool => tool.function.name);
      assert.ok(names.includes('shell_tab'));
      for (const id of ['fetch_url', 'web_search', 'mcp_fixture__echo']) assert.ok(!names.includes(id), `${id} must not be sent to the model`);
      assert.equal(requests.length, 0, 'disabled search performs no request');
    }],
    ['the HTTP editor replaces DuckDuckGo with MoonShot POST search and document GET', async () => {
      await openTools();
      await side.getByRole('button', { name: 'Edit web_search', exact: true }).click();
      await side.getByRole('button', { name: 'MoonShot search template', exact: true }).click();
      await side.getByLabel('URL', { exact: true }).fill(`${apiBase}/v1/search`);
      await side.getByRole('button', { name: 'Save', exact: true }).click();
      await side.locator('[data-tool-id="web_search"]').waitFor();
      await side.getByRole('checkbox', { name: 'Enable web_search', exact: true }).check();
      await waitFor(async () => (await mgmt('shell_agent', 'state_get')).tools.find(tool => tool.id === 'web_search').enabled);
      await side.getByRole('button', { name: 'Add HTTP tool', exact: true }).click();
      await side.getByRole('button', { name: 'Document lookup template', exact: true }).click();
      await side.getByLabel('URL', { exact: true }).fill(`${apiBase}/v1/documents/{{document_id}}`);
      await side.getByRole('button', { name: 'Save', exact: true }).click();
      await side.locator('[data-tool-id="get_search_document"]').waitFor();
      assert.equal(requests.length, 0, 'configuring tools does not execute them');
      await backToChat();
      await side.getByRole('button', { name: 'New', exact: true }).click();
      calls.length = 0;
      search = true;
      const answer = await ask(query);
      assert.match(answer.content, /MOONSHOT-OK/);
      assert.deepEqual(requests, [
        { method: 'POST', url: '/v1/search', body: { mode: 'text', query, offset: 0, limit: 10 } },
        { method: 'GET', url: '/v1/documents/doc%20%2F1', body: null },
      ]);
      const names = calls[0].tools.map(tool => tool.function.name);
      assert.ok(names.includes('web_search') && names.includes('get_search_document'));
      assert.ok(!names.includes('fetch_url') && !names.includes('mcp_fixture__echo'));
    }],
    ['tool definitions and disabled choices persist through pane reload', async () => {
      await side.reload();
      await side.waitForFunction(() => !!window.__native);
      await openTools();
      assert.equal(await side.getByRole('checkbox', { name: 'Enable fetch_url', exact: true }).isChecked(), false);
      assert.equal(await side.getByRole('checkbox', { name: 'Enable mcp_fixture__echo', exact: true }).isChecked(), false);
      assert.equal(await side.getByRole('checkbox', { name: 'Enable web_search', exact: true }).isChecked(), true);
      const config = JSON.parse(fs.readFileSync(path.join(app.data, 'agent.json'), 'utf8'));
      assert.equal(config.tools.http.find(tool => tool.id === 'web_search').url, `${apiBase}/v1/search`);
      assert.equal(config.tools.http.length, 2);
      assert.equal(config.tools.enabled.fetch_url, false);
      assert.equal(config.tools.enabled.get_search_document, true);
    }],
    ['every source has editable names and schemas, and renamed tools retain their original handlers', async () => {
      await openTools();
      const edit = async (id, name) => {
        await side.getByRole('button', { name: `Edit ${id}`, exact: true }).click();
        await side.getByLabel('Model tool name', { exact: true }).fill(name);
        await side.getByLabel('Description', { exact: true }).fill(`CONFIGURED ${id} description`);
        const field = side.getByLabel('Parameters (JSON Schema)', { exact: true });
        const parameters = JSON.parse(await field.inputValue());
        if (parameters.properties.tab_id) parameters.properties.tab_id.description = 'Configured tab selection';
        await field.fill(JSON.stringify(parameters));
        await side.getByRole('button', { name: 'Save', exact: true }).click();
        await side.locator(`[data-tool-id="${id}"]`).waitFor();
      };
      await edit('shell_page_read', 'read_current_page');
      await edit('fetch_skill', 'read_skill_definition');
      await edit('mcp_fixture__echo', 'configured_echo');
      assert.equal(await side.getByRole('checkbox', { name: 'Enable mcp_fixture__echo', exact: true }).isChecked(), false,
        'editing does not enable a disabled tool');
      await side.getByRole('checkbox', { name: 'Enable mcp_fixture__echo', exact: true }).check();
      await waitFor(async () => (await mgmt('shell_agent', 'state_get')).tools.find(tool => tool.id === 'mcp_fixture__echo').enabled);
      await edit('shell_page_navigate', 'propose_page');
      await side.getByRole('button', { name: 'Edit web_search', exact: true }).click();
      await side.getByLabel('Model tool name', { exact: true }).fill('moonshot_search');
      await side.getByRole('button', { name: 'Save', exact: true }).click();
      const readCard = side.locator('[data-tool-id="shell_page_read"]');
      await readCard.waitFor();
      await readCard.locator('summary').click();
      const definition = JSON.parse(await readCard.locator('pre').innerText());
      assert.equal(definition.function.name, 'read_current_page');
      assert.equal(definition.function.description, 'CONFIGURED shell_page_read description');
      assert.equal(definition.function.parameters.properties.tab_id.description, 'Configured tab selection');
      await backToChat();
      if (await side.locator('#nav-switch').isChecked()) {
        await side.locator('label.switch').filter({ has: side.locator('#nav-switch') }).click();
      }
      assert.equal(await side.locator('#nav-switch').isChecked(), false);
      await side.getByRole('button', { name: 'New', exact: true }).click();
      calls.length = 0;
      aliases = true;
      const answer = await ask('Use the configured tool names.');
      assert.match(answer.content, /ALIASES-OK/);
      const sent = calls[0].tools.map(tool => tool.function.name);
      for (const [id, name] of [['shell_page_read', 'read_current_page'], ['fetch_skill', 'read_skill_definition'],
        ['mcp_fixture__echo', 'configured_echo'], ['web_search', 'moonshot_search'], ['shell_page_navigate', 'propose_page']]) {
        assert.ok(sent.includes(name) && !sent.includes(id), `${id} is exposed only as ${name}`);
      }
      assert.ok(mcpRequests.some(message => message.method === 'tools/call' &&
        message.params.name === 'echo' && message.params.arguments.text === 'configured argument'));
      assert.ok(!app.ctx.pages().some(page => page.url().includes('manual-alias=1')), 'renaming navigation preserves manual mode');
      await side.reload();
      await side.waitForFunction(() => window.__agentUi?.state?.tools?.some(tool => tool.name === 'read_current_page'));
      await openTools();
      const state = await mgmt('shell_agent', 'state_get');
      assert.equal(state.tools.find(tool => tool.id === 'web_search').name, 'moonshot_search');
      assert.equal(state.tools.find(tool => tool.id === 'fetch_url').enabled, false);
    }],
    ['definitions can be restored and MCP connections remain editable from the tool editor', async () => {
      await openTools();
      await side.getByRole('button', { name: 'Edit fetch_skill', exact: true }).click();
      await side.getByRole('button', { name: 'Restore source definition', exact: true }).click();
      await side.locator('[data-tool-id="fetch_skill"]').waitFor();
      const state = await mgmt('shell_agent', 'state_get');
      assert.equal(state.tools.find(tool => tool.id === 'fetch_skill').name, 'fetch_skill');
      assert.equal(state.tools.find(tool => tool.id === 'fetch_skill').enabled, true);
      await side.getByRole('button', { name: 'Edit mcp_fixture__echo', exact: true }).click();
      await side.getByRole('button', { name: 'Edit MCP connection', exact: true }).click();
      await side.getByRole('button', { name: 'Add server', exact: true }).waitFor();
      assert.ok((await side.locator('.ag-card').filter({ hasText: 'Fixture' }).innerText()).includes(`${apiBase}/mcp`));
    }],
  ]);
} finally {
  await app.close();
  fixtures.close();
  model.server.close();
  api.server.close();
}
process.exitCode = failed ? 1 : 0;
