// PDF viewer: PDFs open in our pdf.js viewer so selection and page text reach the AI.
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { launch, listen, run, sleep } from './harness.mjs';

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

let redirectedCookie;
let authenticatedCookie;
const redirectTarget = await listen((req, res) => {
  redirectedCookie = req.headers.cookie || '';
  res.setHeader('content-type', 'application/pdf');
  res.end(makePdf('Redirected document'));
});
const server = http.createServer((req, res) => {
  if (req.url === '/sign-in') {
    res.setHeader('set-cookie', 'pdf_session=allowed; HttpOnly; SameSite=Lax; Path=/');
    res.setHeader('content-type', 'text/html');
    res.end('<h1>Signed in</h1>');
    return;
  }
  if (req.url === '/redirect.pdf') {
    res.writeHead(302, { location: `http://127.0.0.1:${redirectTarget.port}/doc.pdf` });
    res.end();
    return;
  }
  if (req.url === '/oversized.pdf') {
    res.writeHead(200, { 'content-type': 'application/pdf', 'content-length': 64 * 1024 * 1024 + 1 });
    res.end(makePdf('Too large'));
    return;
  }
  if (req.url === '/login.pdf') {
    res.setHeader('content-type', 'text/html');
    res.end('<h1>Please sign in</h1>');
    return;
  }
  if (req.url === '/authenticated.pdf') {
    authenticatedCookie = req.headers.cookie || '';
    if (!authenticatedCookie.includes('pdf_session=allowed')) {
      res.writeHead(403);
      res.end('Please sign in');
      return;
    }
  }
  if (req.url.endsWith('.pdf')) {
    res.setHeader('content-type', 'application/pdf');
    res.end(makePdf('Quarterly revenue grew twelve percent'));
  } else {
    res.setHeader('content-type', 'text/html');
    res.end(`<h1>Page before a drop</h1><a id="l" href="http://127.0.0.1:${server.address().port}/doc.pdf">pdf</a>
      <input id="upload" type="file" accept="application/pdf">
      <div id="dropzone" style="width:240px;height:80px;background:#ddd">Upload drop zone</div>
      <div id="window-dropzone" style="width:240px;height:80px;background:#ddd">Window upload drop zone</div>
      <script>
        const zone = document.getElementById('dropzone');
        zone.addEventListener('dragover', event => event.preventDefault());
        zone.addEventListener('drop', event => { event.preventDefault(); window.uploadedFile = event.dataTransfer.files[0]?.name; });
        window.addEventListener('dragover', event => {
          if (event.target.closest('#window-dropzone')) event.preventDefault();
        });
        window.addEventListener('drop', event => {
          if (event.target.closest('#window-dropzone')) { event.preventDefault(); window.windowUploadedFile = event.dataTransfer.files[0]?.name; }
        });
      </script>`);
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const reservation = await listen((_req, res) => res.end());
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ debugPort: reservation.port, withProvider: false });
const localDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'browser-pdf-drop-'));
const localPath = path.join(localDirectory, 'Local document 中文 # 100%.PDF');
fs.writeFileSync(localPath, makePdf('Local dropped PDF document'));
const localUrl = pathToFileURL(localPath).href;
async function dropFile(page, selector) {
  const box = await page.locator(selector).boundingBox();
  assert.ok(box);
  const client = await app.ctx.newCDPSession(page);
  try {
    const data = { items: [], files: [localPath], dragOperationsMask: 1 };
    for (const type of ['dragEnter', 'dragOver', 'drop']) {
      await client.send('Input.dispatchDragEvent', { type, x: box.x + box.width / 2, y: box.y + box.height / 2, data });
    }
  } finally { await client.detach(); }
}
const viewer = () => app.ctx.pages().find((p) => /pdf\.localhost/.test(p.url()));
const waitViewer = async () => { for (let i = 0; i < 60; i++) { const v = viewer(); if (v) return v; await sleep(250); } return null; };

let failed = 0;
try {
failed = await run([
  ['a PDF url opens in the pdf.js viewer with a selectable text layer', async () => {
    await app.invoke('nav.open', { url: `${base}/doc.pdf`, disposition: 'current' });
    const v = await waitViewer();
    assert.ok(v, 'viewer page exists');
    await v.waitForSelector('.textLayer span', { timeout: 20000 });
    assert.match(await v.locator('.textLayer').first().innerText(), /Quarterly revenue/);
    assert.ok(await v.locator('.page canvas').count() >= 1, 'canvas drawn');
  }],
  ['the tab keeps the real PDF url, not the viewer url', async () => {
    const s = JSON.stringify(await app.invoke('agent.context', {}));
    assert.ok(s.includes('/doc.pdf'), s);
    assert.ok(!s.includes('pdf.localhost'), s);
  }],
  ['selecting PDF text reaches the AI selection pipeline', async () => {
    const v = viewer();
    await v.evaluate(() => {
      const el = document.querySelector('.textLayer span');
      const r = document.createRange(); r.selectNodeContents(el);
      const s = getSelection(); s.removeAllRanges(); s.addRange(r);
      document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    });
    await sleep(800);
    const sel = await v.evaluate(() => String(getSelection()));
    assert.match(sel, /Quarterly/);
    const ctx = await app.invoke('agent.context', {});
    assert.match(ctx.page.selection || '', /Quarterly/, JSON.stringify(ctx.page));
  }],
  ['PDF find reports and navigates document text', async () => {
    await app.shell.evaluate(() => {
      window.__pdfFindResults = [];
      window.__native.on('find.result', result => window.__pdfFindResults.push(result));
    });
    await app.invoke('find.start', { query: 'revenue' });
    await app.shell.waitForFunction(() => window.__pdfFindResults.at(-1)?.total === 1);
    await app.invoke('find.next', { query: 'revenue' });
    await app.shell.waitForFunction(() => window.__pdfFindResults.at(-1)?.active === 1);
    await app.invoke('find.stop');
  }],
  ['PDF loading uses the browser HttpOnly login cookie', async () => {
    await app.invoke('nav.open', { url: base + '/sign-in' });
    const signedIn = await app.pageWith(/\/sign-in$/);
    assert.ok(signedIn);
    await signedIn.locator('h1').waitFor();
    assert.equal(await signedIn.evaluate(() => document.cookie), '', 'login cookie is not available to page JavaScript');
    await app.invoke('nav.open', { url: base + '/authenticated.pdf' });
    const v = await app.pageWith(/authenticated.*pdf/);
    assert.ok(v);
    await v.waitForSelector('.textLayer span');
    assert.match(authenticatedCookie, /pdf_session=allowed/);
  }],
  ['PDF redirects do not forward login cookies to a different origin', async () => {
    await app.invoke('nav.open', { url: base + '/redirect.pdf' });
    const v = await app.pageWith(/redirect.*pdf/);
    assert.ok(v);
    await v.waitForSelector('.textLayer span');
    assert.equal(redirectedCookie, '');
    assert.match(await v.locator('.textLayer').first().innerText(), /Redirected document/);
  }],
  ['oversized PDFs and sign-in HTML produce visible errors instead of a blank viewer', async () => {
    for (const [file, message] of [['oversized.pdf', '64 MiB limit'], ['login.pdf', 'non-PDF response']]) {
      await app.invoke('nav.open', { url: `${base}/${file}` });
      const v = await app.pageWith(new RegExp(file.replace('.', '.*')));
      assert.ok(v);
      await v.waitForFunction(expected => document.querySelector('#status').textContent.includes(expected), message);
    }
  }],
  ['dropping a local PDF replaces the page and preserves toolbar Back and Forward', async () => {
    const previousUrl = base + '/before-drop';
    await app.invoke('nav.open', { url: previousUrl });
    const page = await app.pageWith(/\/before-drop$/);
    await page.locator('h1').waitFor();
    const before = await app.invoke('agent.context');
    await dropFile(page, 'h1');
    await page.waitForURL(/pdf\.localhost/);
    await page.waitForSelector('.textLayer span');
    assert.match(await page.locator('.textLayer').first().innerText(), /Local dropped PDF/);
    const after = await app.invoke('agent.context');
    assert.equal(after.page.tab_id, before.page.tab_id, 'opens in the same page, not another tab');
    assert.equal(after.tabs.length, before.tabs.length);
    assert.equal(after.page.url, localUrl);
    await app.shell.locator('#btn-back').click();
    await page.waitForURL(previousUrl);
    await page.locator('h1').waitFor();
    await app.shell.locator('#btn-forward').click();
    await page.waitForURL(/pdf\.localhost/);
    await page.waitForSelector('.textLayer span');
    assert.equal((await app.invoke('agent.context')).page.url, localUrl);
  }],
  ['native file inputs and website upload drop zones retain PDF drops', async () => {
    const uploadUrl = base + '/upload-test';
    await app.invoke('nav.open', { url: uploadUrl });
    const page = await app.pageWith(/\/upload-test$/);
    await page.locator('#upload').waitFor();
    await dropFile(page, '#upload');
    await page.waitForFunction(name => document.querySelector('#upload').files[0]?.name === name, path.basename(localPath));
    assert.equal(page.url(), uploadUrl);
    await dropFile(page, '#dropzone');
    await page.waitForFunction(name => window.uploadedFile === name, path.basename(localPath));
    assert.equal(page.url(), uploadUrl);
    await dropFile(page, '#window-dropzone');
    await page.waitForFunction(name => window.windowUploadedFile === name, path.basename(localPath));
    assert.equal(page.url(), uploadUrl);
  }],
  ['dropping a PDF into an unfocused canvas cell navigates only that cell with Back and Forward', async () => {
    const urls = [base + '/drop-left', base + '/drop-right'];
    await app.invoke('canvas.create', { cols: 2, rows: 1, urls });
    const left = await app.pageWith(/\/drop-left$/);
    const right = await app.pageWith(/\/drop-right$/);
    await left.locator('h1').waitFor();
    await right.locator('h1').waitFor();
    await left.evaluate(() => { window.cellMarker = 'preserved'; });
    await app.invoke('canvas.focus', { cell: 0 });
    const before = await app.invoke('agent.context');
    const target = before.tabs.find(tab => tab.url === urls[1]);
    await dropFile(right, 'h1');
    await right.waitForURL(/pdf\.localhost/);
    await right.waitForSelector('.textLayer span');
    const after = await app.invoke('agent.context');
    assert.equal(after.page.tab_id, target.tab_id, 'the drop focuses its destination cell');
    assert.equal(after.page.url, localUrl);
    assert.equal(after.tabs.length, before.tabs.length);
    assert.equal(after.tabs.find(tab => tab.tab_id === target.tab_id).canvas_id, target.canvas_id);
    assert.equal(left.url(), urls[0]);
    assert.equal(await left.evaluate(() => window.cellMarker), 'preserved');
    await app.shell.locator('#btn-back').click();
    await right.waitForURL(urls[1]);
    await right.locator('h1').waitFor();
    await app.shell.locator('#btn-forward').click();
    await right.waitForURL(/pdf\.localhost/);
    await right.waitForSelector('.textLayer span');
    assert.equal(left.url(), urls[0]);
  }],
]);
} finally {
  await app.close();
  server.close();
  redirectTarget.server.close();
  fs.unlinkSync(localPath);
  fs.rmdirSync(localDirectory);
}
process.exitCode = failed ? 1 : 0;
