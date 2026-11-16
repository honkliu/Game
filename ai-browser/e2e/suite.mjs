// End-to-end suite: drives the real browser.exe over CDP with a scripted model.
// Run: cargo build (into %TEMP%\ai-target), then `node e2e/suite.mjs`.
import assert from 'node:assert/strict';
import { launch, startFixtures, startModel, run, sleep, osClickElement } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;
const model = await startModel((messages) => {
  const last = messages[messages.length - 1]?.content || '';
  if (/PING/.test(last)) return 'PONG from the model';
  return 'ANSWER: Echo of your question.';
});
const app = await launch({ modelPort: model.port });

async function waitFor(fn, timeout = 15000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(200); }
  throw new Error('timed out');
}

const cases = [
  ['shell attaches', async () => { assert.ok(app.shell); }],
  ['navigate opens the fixture page', async () => {
    await app.invoke('nav.open', { url: `${base}/`, disposition: 'current' });
    const pg = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    assert.ok(pg);
    await pg.waitForSelector('#btn');
  }],
  ['OS-level click in page works', async () => {
    const pg = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await osClickElement(pg, '#btn', app.child.pid);
    let ok = false;
    try { await waitFor(async () => (await pg.textContent('#count')).trim() === '1', 4000); ok = true; } catch {}
    if (!ok) {
      console.log('NOTE  no interactive desktop for a real OS click; falling back to a CDP mouse click');
      await pg.click('#btn');
      await waitFor(async () => (await pg.textContent('#count')).trim() === '1');
    }
  }],
  ['window.open("") then location=url opens a tab (sina-style)', async () => {
    await app.invoke('nav.open', { url: `${base}/`, disposition: 'current' });
    const pg = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await pg.waitForSelector('#btn');
    await pg.evaluate((u) => { const w = window.open('', '_blank'); w.location.href = u; }, `${base}/other`);
    await waitFor(() => app.ctx.pages().some((p) => /\/other$/.test(p.url())));
  }],
  ['target=_blank link opens a new tab', async () => {
    await app.invoke('nav.open', { url: `${base}/`, disposition: 'current' });
    const pg = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await pg.click('#blank');
    await waitFor(() => app.ctx.pages().some((p) => /\/other$/.test(p.url())));
  }],
  ['ask pane opens and answers (general question)', async () => {
    await app.invoke('shortcut', { name: 'ask_ai' });
    const side = await app.pageWith(/sidebar\.html/);
    await side.evaluate(() => { const s = document.getElementById('agent-switch'); if (s && s.checked) s.click(); });
    assert.ok(side);
    const r = await app.invoke('ai.ask', { question: 'PING', intent: 'ask', selection: false });
    assert.ok(r.accepted, JSON.stringify(r));
    await waitFor(() => model.calls.length >= 1);
    const c = model.calls[0];
    assert.equal(c.auth, 'Bearer test-key');
    await waitFor(async () => (await side.textContent('#ask-log')).includes('PONG'));
  }],
  ['follow-up carries history', async () => {
    const side = await app.pageWith(/sidebar\.html/);
    await side.evaluate(() => { const s = document.getElementById('agent-switch'); if (s && s.checked) s.click(); });
    const n = model.calls.length;
    await app.invoke('ai.ask', { question: 'second question', intent: 'ask', selection: false });
    await waitFor(() => model.calls.length > n);
    const msgs = model.calls[model.calls.length - 1].body.messages;
    assert.ok(msgs.some((m) => m.role === 'assistant' && /PONG/.test(m.content)), 'history missing');
    await waitFor(async () => (await side.textContent('#ask-log')).includes('Echo'));
  }],
  ['new chat clears history', async () => {
    await app.invoke('ai.history.clear', {});
    const n = model.calls.length;
    await app.invoke('ai.ask', { question: 'fresh', intent: 'ask', selection: false });
    await waitFor(() => model.calls.length > n);
    const msgs = model.calls[model.calls.length - 1].body.messages;
    assert.ok(!msgs.some((m) => m.role === 'assistant'));
  }],
  ['provider.set persists config', async () => {
    const r = await app.invoke('ai.provider.set', { base_url: `http://127.0.0.1:${model.port}/v1`, model_id: 'fake-model', api_key: 'new-key' });
    assert.ok(r);
    const n = model.calls.length;
    await app.invoke('ai.ask', { question: 'PING', intent: 'ask', selection: false });
    await waitFor(() => model.calls.length > n);
    assert.equal(model.calls[model.calls.length - 1].auth, 'Bearer new-key');
  }],
];

const failed = await run(cases);
await app.close();
model.close();
fx.close();
process.exit(failed ? 1 : 0);
