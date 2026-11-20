// Real-mouse text selection and clickable Markdown links in the AI pane.
import assert from 'node:assert/strict';
import { launch, startFixtures, startModel, run, sleep, osDrag } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;
const model = await startModel((messages) => {
  const last = messages[messages.length - 1]?.content || '';
  if (/link please/.test(last)) {
    return `ANSWER: Open [the other page](${base}/other) or see **bold**.\n\n| a | b |\n|---|---|\n| 1 | 2 |`;
  }
  return 'ANSWER: ok';
});
const app = await launch({ modelPort: model.port, debugPort: 9451 });
const waitFor = async (fn, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};
const selection = async () => (await app.invoke('shell_agent', { operation: 'context_get' })).page.selection || '';
let page;

await run([
  ['setup: article page', async () => {
    await app.invoke('shell_page_navigate', { url: `${base}/` });
    page = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
    await page.waitForSelector('#btn');
    await sleep(800);
  }],
  ['a real mouse drag selects text, and the AI toolbar appears only after release', async () => {
    const g = await page.evaluate(() => {
      const r = document.querySelector('p').getBoundingClientRect();
      return { sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio, r: [r.left, r.top, r.width, r.height] };
    });
    const y = (g.sy + g.r[1] + g.r[3] / 2) * g.dpr;
    const drag = osDrag((g.sx + g.r[0] + 2) * g.dpr, y, (g.sx + g.r[0] + g.r[2] - 2) * g.dpr, y, app.child.pid);
    let early = '';
    const t0 = Date.now();
    let done = false;
    drag.then(() => { done = true; });
    while (!done && Date.now() - t0 < 20000) {
      await sleep(120);
      if (!done) early = (await selection()) || early;
    }
    // While the button is still held nothing is reported to the shell.
    assert.equal(early, '', 'selection reported during the drag');
    await waitFor(async () => (await selection()).length > 20);
    const overlay = app.ctx.pages().find((p) => /overlay/.test(p.url()));
    assert.ok(await waitFor(() => overlay.evaluate(() => !!document.querySelector('.sel-toolbar'))), 'toolbar shown');
  }],
  ['chat replies render Markdown and a link click opens a new tab', async () => {
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
    const side = await app.pageWith(/sidebar\.html/);
    await side.evaluate(() => { const s = document.getElementById('agent-switch'); if (s && s.checked) s.click(); });
    await side.waitForSelector('#ask-input');
    await side.fill('#ask-input', 'link please');
    await side.keyboard.press('Enter');
    await side.waitForSelector('.ask-answer .rich a', { timeout: 20000 });
    assert.equal(await side.locator('.ask-answer .rich strong').first().textContent(), 'bold');
    assert.equal(await side.locator('.ask-answer .rich td').count(), 2);
    await side.click('.ask-answer .rich a');
    await waitFor(() => app.ctx.pages().some((p) => p.url() === `${base}/other`), 20000);
  }],
]);

await app.close();
model.close();
fx.close();
