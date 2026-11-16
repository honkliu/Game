// Canvas tabs: a grid of live pages in one tab. Run after `cargo build`.
import assert from 'node:assert/strict';
import { launch, startFixtures, run, sleep, osClick } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;
const app = await launch({ debugPort: 9447, withProvider: false });
const pageUrl = (n) => `${base}/?n=${n}`;
const six = [1, 2, 3, 4, 5, 6].map(pageUrl);

const canvas = () => app.invoke('canvas.state');
const livePage = (n) => app.ctx.pages().find((p) => p.url() === pageUrl(n));
const waitFor = async (fn, ms = 15000) => {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) { last = await fn(); if (last) return last; await sleep(150); }
  throw new Error('timed out waiting for condition');
};
const overlap = (a, b) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;

let cv;

await run([
  ['create a 3x2 canvas with six live pages', async () => {
    const r = await app.invoke('canvas.create', { cols: 3, rows: 2, urls: six });
    assert.equal(r.tab_ids.length, 6);
    await waitFor(async () => six.every((_, i) => livePage(i + 1)));
    cv = await canvas();
    assert.equal(cv.cols, 3);
    assert.equal(cv.rows, 2);
    assert.equal(cv.capacity, 6);
    assert.equal(cv.cells.filter((c) => c.tab_id != null).length, 6);
  }],

  ['cells are laid out without overlap inside the window', async () => {
    cv = await canvas();
    for (const a of cv.cells) {
      assert.ok(a.body.w > 100 && a.body.h > 100, `cell ${a.index} has a usable body`);
      for (const b of cv.cells) if (a.index < b.index) assert.ok(!overlap(a.frame, b.frame), `${a.index} overlaps ${b.index}`);
    }
    // Each page's real viewport matches its cell body (the pages are live and sized).
    for (const c of cv.cells) {
      const p = livePage(c.index + 1);
      const w = await p.evaluate(() => window.innerWidth);
      assert.ok(Math.abs(w - c.body.w) <= 2, `cell ${c.index}: viewport ${w} vs body ${c.body.w}`);
    }
  }],

  ['the rail shows one row for the canvas, not its pages', async () => {
    const rows = await app.shell.evaluate(() => document.querySelectorAll('#rail-list .tab-row').length);
    // The startup tab plus the canvas: the six pages never appear as rows.
    assert.equal(rows, 2);
    const frames = await app.shell.evaluate(() => document.querySelectorAll('.cv-cell').length);
    assert.equal(frames, 6);
  }],

  ['a click inside a page focuses its cell and the omnibox follows', async () => {
    await livePage(5).mouse.click(40, 40);
    await waitFor(async () => (await canvas()).focus === 4);
    const url = await app.shell.evaluate(() => document.getElementById('omnibox').value);
    assert.ok(url.includes('n=5'), `omnibox shows ${url}`);
    await livePage(2).mouse.click(40, 40);
    await waitFor(async () => (await canvas()).focus === 1);
  }],

  ['cells carry no chrome; the shared toolbar acts on the focused page', async () => {
    assert.equal(await app.shell.evaluate(() => document.querySelectorAll('.cv-head, .cv-title').length), 0);
    const c = (await canvas()).cells[0];
    assert.deepEqual(
      { x: c.body.x - c.frame.x, w: c.frame.w - c.body.w },
      { x: 2, w: 4 }, 'a cell is its page plus a 2px focus ring');
    const [c1, c2] = (await canvas()).cells;
    assert.equal(c2.body.x - (c1.body.x + c1.body.w), 4, 'neighbouring pages are 4px apart');
    await app.invoke('canvas.focus', { cell: 3 });
    await app.shell.evaluate(() => document.getElementById('cv-zoom').click());
    await waitFor(async () => (await canvas()).zoomed === true);
    await app.shell.evaluate(() => document.getElementById('cv-zoom').click());
    await waitFor(async () => (await canvas()).zoomed === false);
    assert.equal(await app.shell.evaluate(() => document.getElementById('canvas-bar').hidden), false);
  }],

  ['the focused page is what the AI context sees', async () => {
    await app.invoke('canvas.focus', { cell: 2 });
    const ctx = await app.invoke('agent.context', {});
    const list = JSON.stringify(ctx);
    assert.ok(list.includes('n=3'), 'context mentions the focused page');
  }],

  ['zoom shows one page full size and back', async () => {
    await app.invoke('canvas.focus', { cell: 0 });
    await app.invoke('canvas.zoom', { zoomed: true });
    cv = await canvas();
    assert.equal(cv.zoomed, true);
    const visible = cv.cells.filter((c) => !c.hidden);
    assert.equal(visible.length, 1);
    assert.equal(visible[0].index, 0);
    const w = await waitFor(async () => {
      const x = await livePage(1).evaluate(() => window.innerWidth);
      return x > cv.cells[1].body.w * 2 ? x : 0;
    });
    assert.ok(w > 0);
    await app.invoke('canvas.zoom', { zoomed: false });
    cv = await canvas();
    assert.equal(cv.cells.filter((c) => !c.hidden).length, 6);
  }],

  ['reshaping to 2x2 closes the overflow pages', async () => {
    const r = await app.invoke('canvas.layout', { cols: 2, rows: 2 });
    assert.equal(r.closed, 2);
    cv = await canvas();
    assert.equal(cv.capacity, 4);
    assert.equal(cv.cells.filter((c) => c.tab_id != null).length, 4);
  }],

  ['a page beyond capacity is refused', async () => {
    await assert.rejects(app.invoke('canvas.set', { url: pageUrl(7) }), /fit|full|capacity|empty/i);
    await assert.rejects(app.invoke('canvas.create', { cols: 5, rows: 5 }), /at most|columns|rows|fit/i);
  }],

  ['clearing a cell leaves an empty slot that can be refilled', async () => {
    await app.invoke('canvas.clear', { cell: 3 });
    cv = await canvas();
    assert.equal(cv.cells[3].tab_id, null);
    const r = await app.invoke('canvas.set', { cell: 3, url: pageUrl(8) });
    assert.ok(r.tab_id);
    await waitFor(async () => livePage(8));
  }],

  ['links that open a new window fill the next empty cell', async () => {
    await app.invoke('canvas.clear', { cell: 2 });
    await app.invoke('canvas.focus', { cell: 0 });
    await livePage(1).evaluate((u) => { window.open(u, '_blank'); }, pageUrl(9));
    await waitFor(async () => (await canvas()).cells[2].tab_id != null);
  }],

  ['clicking an empty cell gives it a page driven by the one address bar', async () => {
    await app.invoke('canvas.clear', { cell: 1 });
    await waitFor(async () => app.shell.evaluate(() => document.querySelectorAll('.cv-cell.is-empty').length >= 1));
    assert.equal(await app.shell.evaluate(() => document.querySelectorAll('.cv-input').length), 0);
    const idx = (await canvas()).cells.find((c) => c.tab_id == null).index;
    await app.shell.evaluate((i) => document.querySelector(`.cv-cell[data-cell="${i}"] .cv-empty`).click(), idx);
    await waitFor(async () => (await canvas()).cells[idx].tab_id != null && (await canvas()).focus === idx);
    await app.invoke('nav.open', { url: pageUrl(11), disposition: 'current' });
    await waitFor(async () => livePage(11));
    assert.equal((await canvas()).cells[idx].url, pageUrl(11));
  }],

  ['ctrl+wheel over a page zooms it', async () => {
    const p = livePage(1) ?? livePage(3) ?? livePage(4);
    const base0 = await p.evaluate(() => window.devicePixelRatio);
    await p.evaluate(() => window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, ctrlKey: true, cancelable: true })));
    await waitFor(async () => (await p.evaluate(() => window.devicePixelRatio)) > base0 * 1.05);
    await p.evaluate(() => window.dispatchEvent(new WheelEvent('wheel', { deltaY: 100, ctrlKey: true, cancelable: true })));
    await waitFor(async () => (await p.evaluate(() => window.devicePixelRatio)) < base0 * 1.05);
  }],

  ['a real mouse click in any cell moves the focus there and keeps it', async () => {
    cv = await canvas();
    const full = cv.cells.filter((c) => c.tab_id != null);
    const order = [4, 0, 5, 2, 1, 3, 3, 0].filter((i) => full[i]);
    for (const i of order) {
      const c = full[i];
      // Other pages grab focus for themselves (autofocus); that must not steal it.
      for (const o of full) {
        if (o.index === c.index) continue;
        const p = livePage(o.index + 1);
        await p?.evaluate(() => { const e = document.createElement('input'); e.value = 'hello world'; document.body.append(e); e.focus(); e.select(); document.body.dispatchEvent(new Event('selectionchange')); window.dispatchEvent(new Event('focus')); });
      }
      const info = await app.shell.evaluate(() => ({ sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio }));
      const trace = [];
      const t0 = Date.now();
      const watch = setInterval(async () => { const f = (await canvas()).focus; if (trace.at(-1)?.[1] !== f) trace.push([Date.now() - t0, f]); }, 40);
      osClick((info.sx + c.body.x + c.body.w / 2) * info.dpr, (info.sy + c.body.y + c.body.h / 2) * info.dpr, app.child.pid);
      await waitFor(async () => (await canvas()).focus === c.index, 8000).catch(async (e) => {
        throw new Error(`clicked cell ${c.index}, focus is ${(await canvas()).focus}; trace ${JSON.stringify(trace)} ${JSON.stringify({ info, body: c.body })}`);
      }).finally(() => clearInterval(watch));
      await sleep(800);
      assert.equal((await canvas()).focus, c.index, `focus stayed on cell ${c.index}`);
    }
  }],

  ['a click on blank space between cells picks the nearest cell', async () => {
    cv = await canvas();
    await app.invoke('canvas.focus', { cell: 3 });
    const a = cv.cells[0], b = cv.cells[2];
    const info = await app.shell.evaluate(() => ({ sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio }));
    // the gap just below cell 0's frame, horizontally centred on it
    const gx = a.frame.x + a.frame.w / 2, gy = (a.frame.y + a.frame.h + b.frame.y) / 2;
    osClick((info.sx + gx) * info.dpr, (info.sy + gy) * info.dpr, app.child.pid);
    await waitFor(async () => [0, 2].includes((await canvas()).focus), 8000);
  }],

  ['blank pages: a real click on each cell of a 3x2 focuses exactly that cell', async () => {
    await app.invoke('canvas.create', { cols: 3, rows: 2 });
    for (let i = 1; i < 6; i++) await app.invoke('canvas.focus', { cell: i });
    await sleep(1500);
    cv = await canvas();
    assert.equal(cv.cells.filter((c) => c.tab_id != null).length, 6);
    const info = await app.shell.evaluate(() => ({ sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio }));
    for (const i of [1, 4, 0, 2, 5, 3, 1]) {
      const c = cv.cells[i];
      osClick((info.sx + c.body.x + c.body.w / 2) * info.dpr, (info.sy + c.body.y + c.body.h / 2) * info.dpr, app.child.pid);
      await sleep(900);
      assert.equal((await canvas()).focus, i, `clicked cell ${i}`);
    }
    await app.invoke('tab.close', { tab_id: (await canvas()).id });
    await waitFor(async () => (await canvas()) === null || (await canvas()).cols === 2);
    if ((await canvas()) && (await canvas()).cols === 3) throw new Error('blank canvas not closed');
  }],

  ['closing the canvas closes all its pages', async () => {
    const id = (await canvas()).id;
    await app.invoke('tab.close', { tab_id: id });
    await waitFor(async () => (await canvas()) === null);
    await waitFor(async () => !app.ctx.pages().some((p) => /\/\?n=\d/.test(p.url())));
    const rows = await app.shell.evaluate(() => document.querySelectorAll('#rail-list .tab-row').length);
    assert.equal(rows, 1);
    assert.equal(await app.shell.evaluate(() => document.getElementById('canvas-layer').hidden), true);
  }],
]);

await app.close();
fx.close();
process.exit(0);
