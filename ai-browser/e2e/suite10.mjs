// Page-level "close all popups": the shell command and the agent tool, on a 2x2 canvas.
import assert from 'node:assert/strict';
import http from 'node:http';
import { launch, listen, run, sleep } from './harness.mjs';

const PAGE = (n) => `<!doctype html><body style="margin:0;font:16px sans-serif">
<header id="hdr" style="position:fixed;top:0;left:0;right:0;height:48px;background:#eee;z-index:9999">Site header ${n} <input type="search"></header>
<main style="padding:80px 20px"><h1>Article ${n}</h1><p>${'Real content. '.repeat(40)}</p></main>
<div id="mask" class="modal-mask" style="position:fixed;inset:0;background:rgba(0,0,0,.6);z-index:10000"></div>
<div id="dlg" class="popup-dialog" style="position:fixed;left:25%;top:25%;width:50%;height:40%;background:#fff;z-index:10001">
  <div>Subscribe now!</div><span id="x" class="close-btn" style="position:absolute;right:6px;top:4px;width:20px;height:20px;cursor:pointer" onclick="window.__clicked=(window.__clicked||0)+1">×</span>
</div>
<div id="ad" class="ad-float" style="position:fixed;right:10px;bottom:10px;width:240px;height:160px;background:#fc0;z-index:9000">Buy things <a class="close" href="#" onclick="return false">X</a></div>
<div id="cookie" style="position:fixed;left:0;right:0;bottom:0;height:70px;background:#333;color:#fff;z-index:9500" class="cookie-consent">We use cookies <button>Got it</button></div>
<script>document.body.style.overflow='hidden'</script></body>`;
const server = http.createServer((req, res) => { res.setHeader('content-type', 'text/html'); res.end(PAGE(req.url.slice(1) || 0)); });
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const calls = [];
let script = () => ({ text: 'ok' });
const { port: modelPort } = await listen((req, res) => {
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    if (req.url.endsWith('/models')) { res.setHeader('content-type', 'application/json'); return res.end(JSON.stringify({ data: [{ id: 'fake-model' }] })); }
    const parsed = JSON.parse(body || '{}');
    calls.push(parsed);
    const out = script(parsed.messages || []) || { text: 'ok' };
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    const send = (delta, finish) => res.write(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: finish || null }] })}\n\n`);
    if (out.tools) {
      out.tools.forEach((t, i) => send({ tool_calls: [{ index: i, id: `call_${calls.length}_${i}`, type: 'function', function: { name: t.name, arguments: JSON.stringify(t.args || {}) } }] }));
      send({}, 'tool_calls');
    } else { send({ content: out.text }); send({}, 'stop'); }
    res.end('data: [DONE]\n\n');
  });
});

const app = await launch({ modelPort, debugPort: 9458 });
const waitFor = async (fn, timeout = 25000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};
const pageOf = (n) => app.ctx.pages().find((p) => p.url() === `${base}/${n}`);
const state = (p) => p.evaluate(() => {
  const vis = (id) => { const e = document.getElementById(id); return !!e && getComputedStyle(e).display !== 'none'; };
  return { dlg: vis('dlg'), mask: vis('mask'), ad: vis('ad'), cookie: vis('cookie'), hdr: vis('hdr'), clicked: window.__clicked || 0, scroll: getComputedStyle(document.body).overflow };
});
const popupsLeft = async (n) => { const s = await state(pageOf(n)); return s.dlg || s.mask || s.ad || s.cookie; };
let side;
const ask = async (text) => {
  const n = await side.evaluate(() => window.__agentUi.messages.length);
  await side.evaluate((t) => window.__agentUi.send(t), text);
  await waitFor(() => side.evaluate((n0) => window.__agentUi.messages.length >= n0 + 2, n));
  return side.evaluate(() => window.__agentUi.messages.at(-1));
};

await run([
  ['2x2 canvas of four popup-ridden pages', async () => {
    await app.invoke('canvas.create', { cols: 2, rows: 2, urls: [0, 1, 2, 3].map((n) => `${base}/${n}`) });
    for (let n = 0; n < 4; n++) await waitFor(() => pageOf(n));
    for (let n = 0; n < 4; n++) await pageOf(n).waitForSelector('#dlg');
    await sleep(500);
    assert.ok(await popupsLeft(0));
  }],
  ['shell command, scope cell: only the focused cell is cleaned; header and content survive', async () => {
    await app.invoke('canvas.focus', { cell: 1 });
    await app.invoke('page.dismiss_popups', { scope: 'cell' });
    await waitFor(async () => !(await popupsLeft(1)));
    const s = await state(pageOf(1));
    assert.equal(s.hdr, true, 'site header is chrome, not a popup');
    assert.equal(s.clicked, 1, 'the popup own close button was clicked');
    assert.notEqual(s.scroll, 'hidden', 'page scrolling is restored');
    assert.ok(await popupsLeft(0), 'cell 0 untouched');
    assert.ok(await popupsLeft(3), 'cell 3 untouched');
  }],
  ['agent tool, scope tab: one prompt cleans every cell of the canvas', async () => {
    script = (messages) => messages.some((m) => m.role === 'tool')
      ? { text: 'done' }
      : { tools: [{ name: 'dismiss_popups', args: { scope: 'tab' } }] };
    await app.invoke('shortcut', { name: 'ask_ai' });
    side = await app.pageWith(/sidebar\.html/);
    await side.waitForSelector('#ag-input');
    calls.length = 0;
    const last = await ask('close all popups in the current tab');
    assert.equal(last.tools.length, 1);
    for (let n = 0; n < 4; n++) {
      await waitFor(async () => !(await popupsLeft(n)));
      assert.equal((await state(pageOf(n))).hdr, true);
    }
    const toolResult = calls.at(-1).messages.filter((m) => m.role === 'tool').at(-1).content;
    assert.match(toolResult, /"pages"/);
    assert.ok(JSON.parse(toolResult).dismissed >= 4, toolResult);
  }],
  ['agent tool, scope cell with a cell index', async () => {
    for (let n = 0; n < 4; n++) await pageOf(n).reload();
    for (let n = 0; n < 4; n++) await pageOf(n).waitForSelector('#dlg');
    await sleep(400);
    script = (messages) => messages.some((m) => m.role === 'tool')
      ? { text: 'done' }
      : { tools: [{ name: 'dismiss_popups', args: { scope: 'cell', cell: 2 } }] };
    await ask('close all popups in cell 3');
    await waitFor(async () => !(await popupsLeft(2)));
    assert.ok(await popupsLeft(0) && await popupsLeft(1) && await popupsLeft(3), 'other cells untouched');
  }],
]);

await app.close();
server.close();
