// Horizontal tab strip (default) and the zoom control.
import assert from 'node:assert/strict';
import { launch, startFixtures, startModel, run, sleep, osClick } from './harness.mjs';

const fx = await startFixtures();
const base = `http://127.0.0.1:${fx.port}`;
const model = await startModel(() => 'ANSWER: ok');
const app = await launch({ modelPort: model.port, debugPort: 9452 });
const shell = () => app.shell;
const waitFor = async (fn, timeout = 15000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};

await run([
  ['horizontal tabs by default: strip on top, page uses the full width', async () => {
    await app.invoke('nav.open', { url: `${base}/`, disposition: 'current' });
    await app.invoke('nav.open', { url: `${base}/other`, disposition: 'new_tab' });
    const g = await waitFor(async () => {
      const r = await shell().evaluate(() => {
        const rail = document.querySelector('#rail').getBoundingClientRect();
        const rows = [...document.querySelectorAll('#rail-list .tab-row')].map((e) => e.getBoundingClientRect());
        return { tabs: document.documentElement.dataset.tabs, rail: [rail.left, rail.top, rail.width, rail.height], rows: rows.map((b) => [b.left, b.top, b.width]) };
      });
      return r.rows.length >= 2 ? r : null;
    });
    assert.equal(g.tabs, 'horizontal');
    assert.equal(g.rail[1], 0);
    assert.ok(g.rail[3] <= 40, 'strip height');
    assert.ok(g.rows[1][0] > g.rows[0][0] + 50, 'tabs laid out in a row');
    assert.equal(g.rows[0][1], g.rows[1][1], 'same top');
    const l = await app.invoke('shell.layout', {}).catch(() => null);
    const page = await app.pageWith(/\/other$/);
    const px = await page.evaluate(() => ({ x: window.screenX, y: window.screenY }));
    const sh = await shell().evaluate(() => ({ x: window.screenX, y: window.screenY }));
    assert.ok(Math.abs(px.x - sh.x) <= 2, `page starts at left edge (${px.x} vs ${sh.x})`);
    assert.ok(Math.abs(px.y - sh.y - 72) <= 2, `page below strip+toolbar (dy=${px.y - sh.y})`);
    void l;
  }],
  ['a real click on a tab switches to it', async () => {
    const g = await shell().evaluate(() => {
      const r = document.querySelectorAll('#rail-list .tab-row')[0].getBoundingClientRect();
      return { sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio, x: r.left + 40, y: r.top + r.height / 2 };
    });
    await osClick((g.sx + g.x) * g.dpr, (g.sy + g.y) * g.dpr, app.child.pid);
    await waitFor(async () => (await app.invoke('agent.context', {})).page.url.endsWith('/'));
  }],
  ['zoom buttons change page zoom and the label follows', async () => {
    await shell().click('#zoom-in');
    await waitFor(async () => (await shell().textContent('#zoom-label')) === '110%');
    await shell().click('#zoom-out');
    await shell().click('#zoom-out');
    await waitFor(async () => (await shell().textContent('#zoom-label')) === '91%');
    await shell().click('#zoom-label');
    await waitFor(async () => (await shell().textContent('#zoom-label')) === '100%');
  }],
  ['frameless: the tab strip is the title bar and the window buttons work', async () => {
    const m = await shell().evaluate(() => ({ oh: outerHeight, ih: innerHeight, ow: outerWidth, iw: innerWidth }));
    assert.ok(m.oh - m.ih <= 2, 'no native title bar');
    assert.ok(m.ow - m.iw <= 2, 'no native frame');
    const r = await shell().evaluate(() => ['#win-min', '#win-max', '#win-close', '#win-drag'].map((s) => { const b = document.querySelector(s).getBoundingClientRect(); return [b.top, b.height, b.width]; }));
    for (const [top, h, w] of r) { assert.equal(top, 0); assert.ok(h >= 30 && w > 20); }
    const before = await shell().evaluate(() => window.screenY + ':' + innerHeight);
    await shell().click('#win-max');
    await waitFor(async () => (await shell().evaluate(() => window.screenY + ':' + innerHeight)) !== before);
    await shell().click('#win-max');
    await shell().screenshot({ path: process.env.TEMP + '\\shell-strip.png' });
  }],
  ['vertical tabs remain available as a setting', async () => {
    await app.invoke('settings.update', { key: 'vertical_tabs', value: true });
    await waitFor(() => shell().evaluate(() => document.documentElement.dataset.tabs === 'vertical'));
    await app.invoke('settings.update', { key: 'vertical_tabs', value: false });
    await waitFor(() => shell().evaluate(() => document.documentElement.dataset.tabs === 'horizontal'));
  }],
]);
await app.close?.();
