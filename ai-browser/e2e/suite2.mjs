// Spec behaviours: omnibox AI tasks, multi-tab context, cross-tab citations,
// selection toolbar, act/candidates, stop, stale, privacy. Run after `cargo build`.
import assert from 'node:assert/strict';
import { launch, startFixtures, startModel, run, sleep } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;
let nextDelay = 0;
const model = await startModel((messages) => {
  const all = messages.map((m) => m.content).join('\n');
  const last = messages[messages.length - 1]?.content || '';
  const delay = nextDelay;
  const reply = (text) => (delay ? { text, delay } : text);
  const foreign = /\[(t\d+\.b\d+)\] Flights to Lisbon start at 99 euros in March\./.exec(all);
  if (/Lisbon/.test(last) && foreign) {
    return reply(`ANSWER: Flights to Lisbon start at 99 euros.\nQUOTE ${foreign[1]}: Flights to Lisbon start at 99 euros in March.`);
  }
  if (/"action"/.test(messages[0]?.content || '') && /open the other page/i.test(last)) {
    const link = /\[(l\d+)\] "[^"]*" -> [^\n]*\/other/.exec(all);
    if (/step 1:/.test(last)) return reply(JSON.stringify({ action: 'finish', message: 'Opened it beside this page.' }));
    if (link) return reply(JSON.stringify({ action: 'open_tab', link_id: link[1] }));
  }
  if (/selected/i.test(last) || /Explain/.test(last)) return reply('ANSWER: This explains the selection.');
  return reply('ANSWER: ok ' + last.slice(0, 40));
});
const app = await launch({ modelPort: model.port });

async function waitFor(fn, timeout = 20000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(200); }
  throw new Error('timed out');
}
const status = () => app.invoke('shell_ai', { operation: 'status_get' });
let side;
const sideText = async () => side.textContent('#ask-log');

const cases = [
  ['setup: article tab + sidebar', async () => {
    await app.invoke('shell_page_navigate', { url: `${base}/` });
    await (await app.pageWith(/127\.0\.0\.1:\d+\/$/)).waitForSelector('#btn');
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
    side = await app.pageWith(/sidebar\.html/);
    await side.evaluate(() => { const s = document.getElementById('agent-switch'); if (s && s.checked) s.click(); });
    assert.ok(side);
  }],
  ['omnibox classifies AI tasks, leaves searches alone', async () => {
    const q = (text) => app.invoke('shell_omnibox', { operation: 'query', text });
    assert.equal((await q('summarize this page')).mode, 'ask');
    assert.equal((await q('@ai why is the sky blue')).mode, 'ask');
    assert.equal((await q('compare these tabs')).mode, 'ask');
    assert.equal((await q('summarize cats')).mode, 'search');
    assert.equal((await q('example.com')).mode, 'url');
  }],
  ['omnibox @ai runs a task and shows the answer in the pane', async () => {
    const n = model.calls.length;
    await app.invoke('shell_omnibox', { operation: 'activate', text: '@ai say hello' });
    await waitFor(() => model.calls.length > n);
    await waitFor(async () => (await sideText()).includes('say hello'));
  }],
  ['omnibox "summarize this page" sends a page-grounded summary request', async () => {
    const n = model.calls.length;
    await app.invoke('shell_omnibox', { operation: 'activate', text: 'summarize this page' });
    await waitFor(() => model.calls.length > n);
    const all = model.calls[model.calls.length - 1].body.messages.map((m) => m.content).join('\n');
    assert.ok(/Returns are accepted within 30 days/.test(all), 'page text missing from prompt');
  }],
  ['no secret reaches the model prompt', async () => {
    for (const c of model.calls) {
      const all = JSON.stringify(c.body.messages);
      assert.ok(!all.includes('test-key'), 'API key leaked into a prompt');
    }
  }],
  ['multi-tab: header lists other tabs, question reads a ticked tab', async () => {
    await app.invoke('shell_tab', { operation: 'open', url: `${base}/inject`, background: true });
    await app.pageWith(/\/inject$/);
    await sleep(1500);
    const st = await waitFor(async () => { const s = await status(); return s.other_tabs && s.other_tabs.length ? s : null; });
    const other = st.other_tabs[0];
    assert.ok(other.title || other.host);
    await waitFor(async () => (await side.locator('.ask-tab-opt').count()) > 0);
    await app.invoke('shell_ai', { operation: 'history_clear' });
    const n = model.calls.length;
    const r = await app.invoke('shell_ai', { operation: 'ask', question: 'Lisbon price?', intent: 'ask', selection: false, tabs: [other.tab_id] });
    assert.ok(r.accepted);
    await waitFor(() => model.calls.length > n);
    const all = JSON.stringify(model.calls[model.calls.length - 1].body.messages);
    assert.ok(all.includes('Flights to Lisbon'), 'other tab text missing');
    assert.ok(all.includes('Returns are accepted'), 'active tab text missing');
    await waitFor(async () => (await side.locator('.ask-turn .ask-chip').count()) > 0);
    globalThis.lastTurn = r.turn_id;
    globalThis.otherTab = other.tab_id;
  }],
  ['multi-tab: clicking the cross-tab citation switches to that tab', async () => {
    const before = (await status()).tab.tab_id;
    await side.locator('.ask-turn .ask-chip').first().click();
    await waitFor(async () => (await status()).tab.tab_id === globalThis.otherTab);
    assert.notEqual(before, globalThis.otherTab);
  }],
  ['without ticking, other tabs are not read', async () => {
    await app.invoke('shell_tab', { operation: 'activate', tab_id: before0() });
    await sleep(800);
    await app.invoke('shell_ai', { operation: 'history_clear' });
    const n = model.calls.length;
    await app.invoke('shell_ai', { operation: 'ask', question: 'plain question', intent: 'ask', selection: false });
    await waitFor(() => model.calls.length > n);
    assert.ok(!JSON.stringify(model.calls[model.calls.length - 1].body.messages).includes('Flights to Lisbon'));
  }],
  ['act: model opens a link by id as a candidate tab', async () => {
    await app.invoke('shell_ai', { operation: 'ask', question: 'open the other page', intent: 'act', selection: false });
    await waitFor(async () => (await side.locator('.ask-cand').count()) > 0);
    const tabs = (await status()).other_tabs || [];
    assert.ok(tabs.length > 0, 'candidate tab was not opened');
  }],
  ['stop cancels a slow task', async () => {
    nextDelay = 6000;
    const r = await app.invoke('shell_ai', { operation: 'ask', question: 'slow one', intent: 'ask', selection: false });
    await sleep(800);
    await app.invoke('shell_ai', { operation: 'stop' });
    await waitFor(async () => !(await status()).busy, 5000);
    nextDelay = 0;
    assert.ok(r.accepted);
  }],
  ['navigating away makes an in-flight answer stale', async () => {
    nextDelay = 3000;
    await app.invoke('shell_tab', { operation: 'activate', tab_id: before0() });
    await sleep(600);
    await app.invoke('shell_ai', { operation: 'ask', question: 'will go stale', intent: 'ask', selection: false });
    await sleep(500);
    await app.invoke('shell_page_navigate', { url: `${base}/other` });
    await sleep(4500);
    nextDelay = 0;
    const html = await side.innerHTML('#ask-log');
    assert.ok(/is-stale|is-error/.test(html), 'turn should be stale or refused');
  }],
];

const firstTab = { id: null };
function before0() { return firstTab.id; }
cases[0][1] = ((orig) => async () => { await orig(); firstTab.id = (await status()).tab.tab_id; })(cases[0][1]);

const failed = await run(cases);
await app.close();
model.close();
fx.close();
process.exit(failed ? 1 : 0);
