// PDF in a canvas cell: a real mouse drag over its text reaches the AI selection.
import assert from 'node:assert/strict';
import http from 'node:http';
import { launch, run, sleep, osDrag } from './harness.mjs';

function makePdf(text) {
  const stream = `BT /F1 24 Tf 72 700 Td (${text}) Tj ET`;
  const objs = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let out = '%PDF-1.4\n';
  const offsets = [];
  objs.forEach((o, i) => { offsets.push(out.length); out += `${i + 1} 0 obj\n${o}\nendobj\n`; });
  const xref = out.length;
  out += `xref\n0 ${objs.length + 1}\n0000000000 65535 f \n` + offsets.map((o) => String(o).padStart(10, '0') + ' 00000 n \n').join('');
  out += `trailer\n<< /Size ${objs.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, 'latin1');
}

const server = http.createServer((req, res) => {
  if (req.url.endsWith('.pdf') || req.url.startsWith('/pdf/')) {
    res.setHeader('content-type', 'application/pdf');
    res.end(makePdf('Quarterly revenue grew twelve percent'));
  } else {
    res.setHeader('content-type', 'text/html');
    res.end('<body><p id="t" style="font:20px sans-serif;margin:30px">Plain web page text to select here</p></body>');
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const app = await launch({ debugPort: 9453, withProvider: false });
const viewer = () => app.ctx.pages().find((p) => /pdf\.localhost/.test(p.url()));
const waitFor = async (fn, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};
const selection = async () => (await app.invoke('agent.context', {})).page.selection || '';

async function dragText(page, selector, cell) {
  const cv = await app.invoke('canvas.state');
  const c = cv.cells.find((x) => x.index === cell);
  const g = await page.evaluate((sel) => {
    const el = document.querySelector(sel);
    const r = el.getBoundingClientRect();
    return { r: [r.left, r.top, r.width, r.height] };
  }, selector);
  const sh = await app.shell.evaluate(() => ({ sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio }));
  const y = (sh.sy + c.body.y + g.r[1] + g.r[3] / 2) * sh.dpr;
  const x1 = (sh.sx + c.body.x + g.r[0] + 2) * sh.dpr;
  const x2 = (sh.sx + c.body.x + g.r[0] + g.r[2] - 2) * sh.dpr;
  await osDrag(x1, y, x2, y, app.child.pid);
}

async function jsSelect(page, selector) {
  await page.evaluate((sel) => {
    const el = document.querySelector(sel); const r = document.createRange(); r.selectNodeContents(el);
    const s = getSelection(); s.removeAllRanges(); s.addRange(r);
    const b = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, clientX: b.right, clientY: b.top + 3 }));
  }, selector);
}

await run([
  ['2x2 canvas: a web page and a PDF', async () => {
    await app.invoke('canvas.create', { cols: 2, rows: 2, urls: [`${base}/`, `${base}/doc.pdf`] });
    const v = await waitFor(() => viewer());
    await v.waitForSelector('.textLayer span', { timeout: 20000 });
    await sleep(800);
  }],
  ['web cell: a real drag selects text and the context shows it', async () => {
    const page = app.ctx.pages().find((p) => p.url() === `${base}/`);
    await dragText(page, '#t', 0);
    await waitFor(async () => /Plain web/.test(await selection()));
  }],
  ['PDF cell: a real drag selects text and the context shows it', async () => {
    await dragText(viewer(), '.textLayer span', 1);
    await sleep(1200);
    const ctx = await app.invoke('agent.context', {});
    const local = await viewer().evaluate(() => String(getSelection()));
    assert.match(local, /Quarterly/, 'the page itself selected the text');
    assert.match(ctx.page.selection || '', /Quarterly/, JSON.stringify(ctx.page));
  }],
  ['PDF cell: zoom buttons and ctrl+wheel really scale the pages', async () => {
    await app.invoke('canvas.focus', { cell: 1 });
    const v = app.ctx.pages().find((p) => /pdf\.localhost/.test(p.url()) && /doc\.pdf/.test(decodeURIComponent(p.url())));
    const width = () => v.evaluate(() => Math.round(document.querySelector('.page').getBoundingClientRect().width));
    const w0 = await width();
    await app.invoke('shortcut', { name: 'zoom_in' });
    await app.invoke('shortcut', { name: 'zoom_in' });
    await waitFor(async () => (await width()) > w0 * 1.05);
    await v.evaluate(() => window.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, ctrlKey: true, bubbles: true, cancelable: true })));
    await waitFor(async () => (await width()) > w0 * 1.12);
    await app.invoke('shortcut', { name: 'zoom_reset' });
    await sleep(1500); const w1 = await width(); assert.ok(Math.abs(w1 - w0) <= 20, 'reset: ' + w0 + ' -> ' + w1);
  }],
  ['extensionless PDF url (like arxiv.org/pdf/ID) also gets the viewer and a selectable text', async () => {
    await app.invoke('canvas.set', { cell: 2, url: `${base}/pdf/1234` });
    await waitFor(async () => app.ctx.pages().filter((p) => /pdf\.localhost/.test(p.url())).length >= 2);
    const v = app.ctx.pages().filter((p) => /pdf\.localhost/.test(p.url())).find((p) => /1234/.test(decodeURIComponent(p.url())));
    assert.ok(v, 'second viewer');
    await v.waitForSelector('.textLayer span', { timeout: 20000 });
    await sleep(500);
    await dragText(v, '.textLayer span', 2);
    await sleep(1200);
    assert.match(await selection(), /Quarterly/);
    assert.ok(JSON.stringify(await app.invoke('agent.context', {})).includes('/pdf/1234'), 'tab keeps the real url');
  }],
  ['local file:// PDF: a real drag selects text and the context shows it', async () => {
    const fs = await import('node:fs'); const os = await import('node:os'); const path = await import('node:path');
    const bytes = Buffer.from(await (await fetch(`${base}/doc.pdf`)).arrayBuffer());
    const file = path.join(os.tmpdir(), 'ab-local-test.pdf'); fs.writeFileSync(file, bytes);
    await app.invoke('canvas.set', { cell: 3, url: 'file:///' + file.replace(/\\/g, '/') });
    await waitFor(async () => app.ctx.pages().some((p) => /pdf\.localhost/.test(p.url()) && /ab-local-test/.test(decodeURIComponent(p.url()))));
    const v = app.ctx.pages().find((p) => /ab-local-test/.test(decodeURIComponent(p.url())));
    await v.waitForSelector('.textLayer span', { timeout: 20000 });
    await sleep(800);
    await app.invoke('canvas.focus', { cell: 3 });
    await sleep(500);
    await jsSelect(v, '.textLayer span');
    try { await waitFor(async () => /Quarterly/.test(await selection())); } catch (e) { const cs = await app.invoke('canvas.state'); console.log('DBG', JSON.stringify(await v.evaluate(() => String(getSelection()))), JSON.stringify(cs.cells.map((c) => [c.index, c.focused, c.url])), JSON.stringify((await app.invoke('agent.context', {})).page).slice(0, 400)); throw e; }
  }],
]);

await app.close();
server.close();
