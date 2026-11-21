// Compact scrollbars: actual gutter sizes, site overrides, CSP, frames and browser panels.
import assert from 'node:assert/strict';
import { launch, listen, run } from './harness.mjs';

const fixtures = await listen((req, res) => {
  if (req.url === '/fixture.css') {
    res.setHeader('content-type', 'text/css');
    return res.end(`html,body{margin:0} main{width:3000px;height:3000px}
      .box{width:160px;height:100px;overflow:scroll}.inner{width:500px;height:500px}
      #custom::-webkit-scrollbar{width:12px;height:10px}
      #hidden::-webkit-scrollbar{display:none} #standard-hidden{scrollbar-width:none}`);
  }
  if (req.url === '/csp') res.setHeader('content-security-policy', "default-src 'self'; style-src 'self'");
  res.setHeader('content-type', 'text/html');
  res.end(`<!doctype html><html><head><link rel="stylesheet" href="/fixture.css"></head><body>
    <main>${['default','custom','hidden','standard-hidden'].map(id => `<div class="box" id="${id}"><div class="inner"></div></div>`).join('')}
    ${req.url === '/frame' ? '' : '<iframe src="/frame"></iframe>'}</main></body></html>`);
});
const reservation = await listen((_req, res) => res.end());
const debugPort = reservation.port;
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ debugPort, withProvider: false });
const base = `http://127.0.0.1:${fixtures.port}`;

async function pageAt(route) {
  await app.invoke('shell_page_navigate', { url: base + route });
  const page = await app.pageWith(new RegExp(`${fixtures.port}${route}$`));
  await page.waitForSelector('#default');
  return page;
}

async function assertCompact(page) {
  const measured = await page.evaluate(() => {
    const gap = el => ({ x: el.offsetWidth - el.clientWidth, y: el.offsetHeight - el.clientHeight });
    const read = () => ({ viewport: { x: innerWidth - document.documentElement.clientWidth,
      y: innerHeight - document.documentElement.clientHeight }, nested: gap(document.getElementById('default')) });
    const sheets = document.adoptedStyleSheets;
    const compact = read();
    document.adoptedStyleSheets = [];
    const original = read();
    document.adoptedStyleSheets = sheets;
    return { compact, original };
  });
  for (const kind of ['viewport', 'nested']) for (const axis of ['x', 'y']) {
    const original = measured.original[kind][axis];
    assert.equal(measured.compact[kind][axis], Math.min(original, 8), JSON.stringify(measured));
  }
  await page.evaluate(() => { scrollTo(100, 150); document.getElementById('default').scrollTo(70, 90); });
  await page.waitForFunction(() => scrollX === 100 && scrollY === 150);
  assert.deepEqual(await page.locator('#default').evaluate(el => [el.scrollLeft, el.scrollTop]), [70, 90]);
}

const cases = [
  ['page and nested scrollbars use at most 8px on both axes without preventing scrolling', async () => {
    const page = await pageAt('/normal');
    await assertCompact(page);
    assert.deepEqual(await page.locator('#custom').evaluate(el => [el.offsetWidth - el.clientWidth, el.offsetHeight - el.clientHeight]), [12, 10]);
    for (const id of ['hidden', 'standard-hidden']) {
      assert.deepEqual(await page.locator(`#${id}`).evaluate(el => [el.offsetWidth - el.clientWidth, el.offsetHeight - el.clientHeight]), [0, 0]);
    }
    const frame = page.frames().find(frame => frame.url() === `${base}/frame`);
    assert.ok(frame);
    await frame.waitForSelector('#default');
    await assertCompact(frame);
  }],
  ['compact defaults also work when inline styles are blocked by CSP', async () => {
    await assertCompact(await pageAt('/csp'));
  }],
  ['every cell in a 2x2 canvas has compact independent scrollbars on both axes', async () => {
    const opened = await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2,
      urls: [0, 1, 2, 3].map(cell => `${base}/cell-${cell}`) });
    try {
      const pages = [];
      for (let cell = 0; cell < 4; cell++) {
        const page = await app.pageWith(new RegExp(`${fixtures.port}/cell-${cell}$`));
        await page.waitForSelector('#default');
        await assertCompact(page);
        pages.push(page);
      }
      await pages[0].evaluate(() => scrollTo(250, 300));
      await pages[0].waitForFunction(() => scrollX === 250 && scrollY === 300);
      for (const page of pages.slice(1)) assert.deepEqual(await page.evaluate(() => [scrollX, scrollY]), [100, 150]);
    } finally { await app.invoke('shell_tab', { operation: 'close', tab_id: opened.canvas_id }); }
  }],
  ['browser chrome, Settings and River panels have thin usable scrollbars', async () => {
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
    const side = await app.pageWith(/sidebar\.html/);
    await app.invoke('shell_settings', { operation: 'open' });
    const settings = await app.pageWith(/pages\/settings\.html/);
    for (const page of [app.shell, side, settings]) {
      const gap = await page.evaluate(() => {
        const el = document.createElement('div');
        el.style.cssText = 'position:fixed;left:0;top:0;width:160px;height:100px;overflow:scroll;border:0;padding:0';
        el.innerHTML = '<div style="width:500px;height:500px"></div>';
        document.body.appendChild(el);
        const result = [el.offsetWidth - el.clientWidth, el.offsetHeight - el.clientHeight];
        el.remove();
        return result;
      });
      assert.ok(gap.every(size => size <= 8), JSON.stringify(gap));
    }
  }],
  ['bundled PDF styling reduces both scrollbar dimensions', async () => {
    const css = await app.shell.evaluate(async () => (await fetch('pdf/viewer.css')).text());
    const page = await pageAt('/pdf-style');
    await page.addStyleTag({ content: css });
    const gap = await page.evaluate(() => [innerWidth - document.documentElement.clientWidth, innerHeight - document.documentElement.clientHeight]);
    assert.ok(gap.every(size => size <= 8), JSON.stringify(gap));
  }],
];

try { process.exitCode = await run(cases) ? 1 : 0; }
finally { await app.close(); fixtures.server.close(); }
