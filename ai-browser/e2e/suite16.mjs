// Empty cells and layered new-window routing, without physical desktop input.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { launch, startFixtures, run, sleep } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;
const debugPort = 16000 + Math.floor(Math.random() * 30000);
let app = await launch({ debugPort, withProvider: false });
const url = n => `${base}/?layer=${n}`;
const canvas = () => app.invoke('canvas.state');
const waitFor = async fn => {
  const end = Date.now() + 15000;
  while (Date.now() < end) {
    const result = await fn();
    if (result) return result;
    await sleep(100);
  }
  throw new Error('timed out waiting for condition');
};
const page = n => waitFor(() => app.ctx.pages().find(p => p.url() === url(n)));
const popup = async (opener, n, cell) => {
  await (await page(opener)).evaluate(u => window.open(u, '_blank'), url(n));
  await waitFor(async () => (await canvas()).cells[cell].url === url(n));
  return page(n);
};

let failed;
try {
  failed = await run([
    ['new canvases and selected empty cells remain genuinely unused', async () => {
      await app.invoke('canvas.create', { cols: 2, rows: 2 });
      assert.ok((await canvas()).cells.every(c => c.tab_id === null));
      for (const cell of [1, 3, 0, 2]) {
        await app.shell.evaluate(i => document.querySelector(`.cv-cell[data-cell="${i}"] .cv-empty`).click(), cell);
        await waitFor(async () => (await canvas()).focus === cell);
        assert.ok((await canvas()).cells.every(c => c.tab_id === null));
      }
      await app.invoke('omnibox.activate', { text: url(1) });
      await page(1);
      assert.equal((await canvas()).cells[2].url, url(1));
      await app.invoke('canvas.clear', { cell: 2 });
      assert.ok((await canvas()).cells.every(c => c.tab_id === null));
      await app.invoke('canvas.focus', { cell: 0 });
      await app.invoke('nav.open', { url: url(1) });
      await page(1);
      assert.equal((await canvas()).cells[0].url, url(1));
    }],
    ['a selected empty cell is still used by a new-window link', async () => {
      await app.invoke('canvas.focus', { cell: 1 });
      const first = await popup(1, 2, 1);
      await first.evaluate(() => { window.layerState = 'preserve me'; document.getElementById('btn').click(); });
      await popup(1, 3, 2);
      await popup(1, 4, 3);
      assert.deepEqual((await canvas()).cells.map(c => c.url), [1, 2, 3, 4].map(url));
    }],
    ['a full canvas cycles through other cells and close restores live contents', async () => {
      const originals = (await canvas()).cells.map(c => c.tab_id);
      await popup(1, 5, 1);
      await popup(1, 6, 2);
      await popup(1, 7, 3);
      await popup(1, 8, 1);
      assert.equal((await canvas()).cells[0].tab_id, originals[0]);
      await app.shell.evaluate(() => document.getElementById('cv-close').click());
      await waitFor(async () => (await canvas()).cells[1].url === url(5));
      await app.invoke('canvas.clear', { cell: 1 });
      assert.equal((await canvas()).cells[1].tab_id, originals[1]);
      const restored = await page(2);
      assert.equal(await restored.evaluate(() => window.layerState), 'preserve me');
      assert.equal(await restored.locator('#count').textContent(), '1');
      for (const cell of [2, 3]) await app.invoke('canvas.clear', { cell });
      assert.deepEqual((await canvas()).cells.map(c => c.tab_id), originals);
    }],
    ['an opener in the last cell cycles through 00, 01, 10, never itself', async () => {
      const source = (await canvas()).cells[3].tab_id;
      for (const [n, cell] of [[9, 0], [10, 1], [11, 2], [12, 0]]) await popup(4, n, cell);
      assert.equal((await canvas()).cells[3].tab_id, source);
      assert.deepEqual((await canvas()).cells.map(c => c.url), [12, 10, 11, 4].map(url));
    }],
    ['restart preserves layers and empty focus, and migrates old new-tab placeholders', async () => {
      const layered = await canvas();
      const empty = await app.invoke('canvas.create', { cols: 2, rows: 2 });
      await app.invoke('canvas.focus', { cell: 3 });
      const profilePath = app.profile;
      await app.invoke('window.close').catch(() => {});
      await waitFor(() => app.child.exitCode !== null);
      await app.browser.close();
      const sessionPath = path.join(app.data, 'session.json');
      const session = JSON.parse(fs.readFileSync(sessionPath, 'utf8'));
      const grid = session.tabs.find(t => t.id === empty.canvas_id).canvas;
      const id = Math.max(...session.tabs.map(t => t.id)) + 1;
      session.tabs.push({ id, url: 'native://newtab', member_of: empty.canvas_id });
      grid.cells[1] = id;
      fs.writeFileSync(sessionPath, JSON.stringify(session));
      app = await launch({ debugPort, withProvider: false, profilePath });
      await waitFor(async () => (await canvas())?.id === empty.canvas_id);
      assert.equal((await canvas()).focus, 3);
      assert.ok((await canvas()).cells.every(c => c.tab_id === null));
      await app.invoke('shortcut', { name: 'next_tab' });
      assert.notEqual((await canvas())?.id, empty.canvas_id);
      await app.invoke('tab.select', { tab_id: empty.canvas_id });
      await app.invoke('tab.close');
      await app.invoke('tab.select', { tab_id: layered.id });
      assert.deepEqual((await canvas()).cells.map(c => c.url), [12, 10, 11, 4].map(url));
    }],
    ['reshaping and closing a canvas dispose of covered pages too', async () => {
      const id = (await canvas()).id;
      await app.invoke('canvas.layout', { cols: 1, rows: 1 });
      assert.equal((await canvas()).cells[0].url, url(12));
      await app.invoke('canvas.clear', { cell: 0 });
      assert.equal((await canvas()).cells[0].url, url(9));
      await app.invoke('canvas.clear', { cell: 0 });
      assert.equal((await canvas()).cells[0].url, url(1));
      await app.invoke('tab.close', { tab_id: id });
      await waitFor(() => !app.ctx.pages().some(p => p.url().startsWith(base)));
    }],
    ['standalone pages and single-cell canvases open a separate tab', async () => {
      await app.invoke('nav.open', { url: url(13) });
      await (await page(13)).evaluate(u => window.open(u, '_blank'), url(14));
      await page(14);
      assert.equal(await canvas(), null);
      const created = await app.invoke('canvas.create', { cols: 1, rows: 1, urls: [url(15)] });
      await (await page(15)).evaluate(u => window.open(u, '_blank'), url(16));
      await page(16);
      assert.equal(await canvas(), null);
      await app.invoke('tab.select', { tab_id: created.canvas_id });
      assert.equal((await canvas()).cells[0].url, url(15));
    }],
    ['chat arrows recall submitted input in both modes and restore the draft after reload', async () => {
      await app.invoke('shortcut', { name: 'ask_ai' });
      const side = await app.pageWith(/sidebar\.html/);
      const input = () => side.locator('textarea.ask-input');
      await input().fill('/model default');
      await input().press('Enter');
      await input().fill('unfinished question');
      await input().press('ArrowUp');
      assert.equal(await input().inputValue(), '/model default');
      await input().press('ArrowDown');
      assert.equal(await input().inputValue(), 'unfinished question');
      await side.locator('label.switch').filter({ has: side.locator('#agent-switch') }).click();
      await input().press('ArrowUp');
      assert.equal(await input().inputValue(), '/model default');
      await input().press('ArrowDown');
      assert.equal(await input().inputValue(), 'unfinished question');
      await side.reload();
      await input().waitFor();
      await input().press('ArrowUp');
      assert.equal(await input().inputValue(), '/model default');
    }],
  ]);
} finally {
  await app.close();
  fx.close();
}
process.exitCode = failed ? 1 : 0;
