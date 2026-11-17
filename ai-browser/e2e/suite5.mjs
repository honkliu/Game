// PDF viewer: PDFs open in our pdf.js viewer so selection and page text reach the AI.
import assert from 'node:assert/strict';
import http from 'node:http';
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
    res.end(`<a id="l" href="http://127.0.0.1:${server.address().port}/doc.pdf">pdf</a>`);
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const reservation = await listen((_req, res) => res.end());
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ debugPort: reservation.port, withProvider: false });
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
]);
} finally {
  await app.close();
  server.close();
  redirectTarget.server.close();
}
process.exitCode = failed ? 1 : 0;
