// Windows PDFs use WebView2's native renderer, UIA document text and engine selection.
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { launch, listen, run, sleep, nativePdfText, startModel } from './harness.mjs';

function makePdf(text, { image = false } = {}) {
  const pages = Array.isArray(text) ? text : [text];
  const objs = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${pages.map((_, i) => `${4 + i * 2} 0 R`).join(' ')}] /Count ${pages.length} >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  pages.forEach((text, i) => {
    const stream = `${image ? 'q 20 0 0 20 72 735 cm /Logo Do Q\n' : ''}BT /F1 24 Tf 72 700 Td ${image ? '(Native selection report) Tj 0 -32 Td (Example authors) Tj 0 -32 Td (Abstract) Tj 0 -32 Td ' : ''}(${text}) Tj ET`;
    objs.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents ${5 + i * 2} 0 R /Resources << /Font << /F1 3 0 R >> ${image ? `/XObject << /Logo ${4 + pages.length * 2} 0 R >>` : ''} >> >>`,
      `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    );
  });
  if (image) objs.push('<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length 7 >>\nstream\nFF0000>\nendstream');
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
    res.writeHead(302, { location: `http://localhost:${redirectTarget.port}/doc.pdf` });
    res.end();
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
  if (req.url.endsWith('.pdf') || req.url === '/extensionless-document') {
    res.setHeader('content-type', 'application/pdf');
    const text = req.url === '/find.pdf' ? ['Revenue report for the first page', 'revenue report on the second page', 'Final REVENUE report on page three'] :
      req.url === '/multipage.pdf' ? [
      'FIRST-PAGE native document introduction',
      'SECOND-PAGE detailed revenue report',
      'THIRD-PAGE offscreen conclusion sentinel',
    ] : req.url === '/left.pdf' ? 'LEFT-NATIVE-PDF independent financial report' :
      req.url === '/right.pdf' ? 'RIGHT-NATIVE-PDF separate canvas document' :
      'Quarterly revenue grew twelve percent';
    res.end(makePdf(text, { image: req.url === '/image.pdf' }));
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
const model = await startModel(() => 'Native PDF content received.');
const app = await launch({ debugPort: reservation.port, modelPort: model.port });
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
async function waitPdf(marker) {
  return nativePdfText(app.child.pid, marker);
}

async function nativePdfEvaluate(page, expression) {
  const cdp = await app.ctx.newCDPSession(page);
  let sessionId;
  try {
    const { targetInfo } = await cdp.send('Target.getTargetInfo');
    const { targetInfos } = await cdp.send('Target.getTargets');
    const target = targetInfos.find(target => target.parentId === targetInfo.targetId &&
      target.url === 'chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/edge_pdf/index.html');
    assert.ok(target, 'the native PDF target belongs to this page');
    ({ sessionId } = await cdp.send('Target.attachToTarget', { targetId: target.targetId, flatten: false }));
    let receive;
    let timeout;
    const reply = new Promise((resolve, reject) => {
      timeout = setTimeout(() => reject(new Error('Native selection oracle timed out')), 5000);
      receive = event => {
        if (event.sessionId !== sessionId) return;
        const message = JSON.parse(event.message);
        if (message.id !== 1) return;
        resolve(message);
      };
      cdp.on('Target.receivedMessageFromTarget', receive);
    });
    try {
      await cdp.send('Target.sendMessageToTarget', { sessionId,
        message: JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: {
          expression, returnByValue: true, awaitPromise: true,
        } }),
      });
      const message = await reply;
      assert.ok(!message.error && !message.result?.exceptionDetails, JSON.stringify(message));
      assert.equal(typeof message.result.result.value, 'string');
      return message.result.result.value;
    } finally {
      clearTimeout(timeout);
      cdp.off('Target.receivedMessageFromTarget', receive);
    }
  } finally {
    if (sessionId) await cdp.send('Target.detachFromTarget', { sessionId });
    await cdp.detach();
  }
}

// Independent native-engine oracle: do not compare UIA-selected text with itself.
async function engineSelection(page, { selectAll = false } = {}) {
  return nativePdfEvaluate(page, `new Promise((resolve, reject) => {
    const previous = viewer.handleSelectedTextReply;
    const restore = () => { viewer.handleSelectedTextReply = previous; clearTimeout(timeout); };
    const timeout = setTimeout(() => { restore(); reject(new Error('Native PDF reply timed out')); }, 2000);
    viewer.handleSelectedTextReply = function(text) {
      restore(); resolve(text); return previous.call(this, text);
    };
    ${selectAll ? "viewer.pluginController_.postMessage({ type: 'selectAll' });" : ''}
    viewer.pluginController_.postMessage({ type: 'getSelectedText' });
  })`);
}

async function preview() {
  return app.shell.evaluate(() => new Promise((resolve, reject) => {
    const req = 980001;
    const timeout = setTimeout(() => { off(); reject(new Error('context preview timed out')); }, 15000);
    const off = window.__native.on('agent.reply', result => {
      if (result.req !== req) return;
      clearTimeout(timeout); off();
      result.ok ? resolve(result.value) : reject(new Error(result.error));
    });
    window.__native.invoke('agent.call', { op: 'context.preview', req }).catch(reject);
  }));
}

async function assertRiverSelection(marker) {
  await app.invoke('shortcut', { name: 'ask_ai' });
  const river = await app.pageWith(/sidebar\.html/);
  try {
    await river.locator('#sel-card').waitFor();
    const quote = marker.slice(0, 600) + (marker.length > 600 ? '\u2026' : '');
    try {
      await river.waitForFunction(expected =>
        document.querySelector('#sel-card .sel-quote')?.textContent === expected, quote);
    } catch (error) {
      assert.equal(await river.locator('#sel-card .sel-quote').textContent(), quote, 'River visibly displays the native selection');
      throw error;
    }
    const before = model.calls.length;
    await river.locator('#sel-card button').filter({ hasText: 'Explain' }).click();
    for (let attempt = 0; attempt < 100 && model.calls.length === before; attempt++) await sleep(100);
    assert.ok(model.calls.length > before, 'Explain sends a real model request');
    const messages = model.calls[before].body.messages;
    assert.ok(messages[0].content.includes(marker), 'the model receives the native selected text');
    assert.match(JSON.stringify(messages.at(-1)), /Explain the selected text/);
    await river.waitForFunction(() => window.__agentUi.messages.at(-1)?.content === 'Native PDF content received.');
  } finally {
    await app.invoke('shortcut', { name: 'ask_ai' });
  }
}

let failed = 0;
try {
failed = await run([
  ['a PDF url uses WebView2 native rendering and exposes native document text', async () => {
    await app.invoke('nav.open', { url: `${base}/doc.pdf`, disposition: 'current' });
    const page = await app.pageWith(/\/doc\.pdf$/);
    assert.ok(page);
    assert.match((await waitPdf('Quarterly revenue')).text, /Quarterly revenue/);
    assert.equal(await page.locator('.textLayer, .page canvas').count(), 0, 'not our PDF.js renderer');
    const session = await app.ctx.newCDPSession(page);
    const targets = await session.send('Target.getTargets');
    await session.detach();
    assert.ok(targets.targetInfos.some(target => target.type === 'webview' && /edge_pdf/.test(target.url)));
  }],
  ['the tab keeps the real PDF url, not the viewer url', async () => {
    const s = JSON.stringify(await app.invoke('agent.context', {}));
    assert.ok(s.includes('/doc.pdf'), s);
    assert.ok(!s.includes('pdf.localhost'), s);
  }],
  ['selecting PDF text reaches the AI selection pipeline', async () => {
    const result = await nativePdfText(app.child.pid, 'Quarterly revenue', 'Quarterly revenue');
    assert.equal(result.selected, 'Quarterly revenue');
    await app.shell.waitForFunction(async () => (await window.__native.invoke('agent.context')).page.selection === 'Quarterly revenue');
    const ctx = await app.invoke('agent.context', {});
    assert.match(ctx.page.selection || '', /Quarterly/, JSON.stringify(ctx.page));
  }],
  ['native PDF selection is visible in River and Explain sends it to the model', async () => {
    await assertRiverSelection('Quarterly revenue');
  }],
  ['PDF selection matches the native engine when accessibility offsets differ', async () => {
    await app.invoke('nav.open', { url: `${base}/image.pdf`, disposition: 'current' });
    try {
      const page = await app.pageWith(/\/image\.pdf$/);
      const accessibility = await nativePdfText(app.child.pid, 'Quarterly revenue', 'revenue grew');
      const actual = await engineSelection(page);
      assert.ok(actual.trim(), 'native selection exists');
      assert.notEqual(actual, accessibility.selected, 'fixture reproduces the accessibility offset mismatch');
      await app.shell.waitForFunction(async expected =>
        (await window.__native.invoke('agent.context')).page.selection === expected, actual);
      await assertRiverSelection(actual);
      const multiline = await engineSelection(page, { selectAll: true });
      assert.match(multiline, /Example authors[\r\n]+Abstract[\r\n]+Quarterly revenue/);
      await app.shell.waitForFunction(async expected =>
        (await window.__native.invoke('agent.context')).page.selection === expected, multiline);
      await assertRiverSelection(multiline);
    } finally {
      await app.invoke('nav.open', { url: `${base}/doc.pdf`, disposition: 'current' });
      await waitPdf('Quarterly revenue');
    }
  }],
  ['River reads native PDF content without viewer chrome', async () => {
    const content = JSON.stringify(await preview());
    assert.match(content, /Quarterly revenue grew twelve percent/);
    assert.ok(!content.includes('Page number') && !content.includes('Rotate, Ctrl'));
  }],
  ['native selection clears when clicking away from text', async () => {
    const page = await app.pageWith(/\/doc\.pdf$/);
    await page.mouse.click(50, 400);
    await app.shell.waitForFunction(async () => !(await window.__native.invoke('agent.context')).page.selection);
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
    await sleep(600);
    assert.equal((await nativePdfText(app.child.pid, 'Quarterly revenue')).selected.trim(), '', 'stopping Find removes its native selection');
  }],
  ['PDF loading uses the browser HttpOnly login cookie', async () => {
    await app.invoke('nav.open', { url: base + '/sign-in' });
    const signedIn = await app.pageWith(/\/sign-in$/);
    assert.ok(signedIn);
    await signedIn.locator('h1').waitFor();
    assert.equal(await signedIn.evaluate(() => document.cookie), '', 'login cookie is not available to page JavaScript');
    await app.invoke('nav.open', { url: base + '/authenticated.pdf' });
    assert.ok(await app.pageWith(/\/authenticated\.pdf$/));
    await waitPdf('Quarterly revenue');
    assert.match(authenticatedCookie, /pdf_session=allowed/);
  }],
  ['PDF redirects do not forward login cookies to a different origin', async () => {
    await app.invoke('nav.open', { url: base + '/redirect.pdf' });
    await waitPdf('Redirected document');
    assert.equal(redirectedCookie, '');
  }],
  ['HTML sign-in responses remain ordinary readable pages', async () => {
    await app.invoke('nav.open', { url: `${base}/login.pdf` });
    const page = await app.pageWith(/\/login\.pdf$/);
    await page.locator('h1').waitFor();
    assert.match(JSON.stringify(await preview()), /Please sign in/);
  }],
  ['extensionless PDFs use native rendering and River text extraction', async () => {
    await app.invoke('nav.open', { url: `${base}/extensionless-document` });
    await waitPdf('Quarterly revenue');
    assert.match(JSON.stringify(await preview()), /Quarterly revenue/);
    assert.equal((await app.invoke('agent.context')).page.url, `${base}/extensionless-document`);
  }],
  ['River reads all native PDF pages including offscreen content', async () => {
    await app.invoke('nav.open', { url: `${base}/multipage.pdf` });
    await waitPdf('FIRST-PAGE');
    const content = JSON.stringify(await preview());
    for (const marker of ['FIRST-PAGE', 'SECOND-PAGE', 'THIRD-PAGE']) assert.ok(content.includes(marker), marker);
    await nativePdfText(app.child.pid, 'FIRST-PAGE', true);
    await app.shell.waitForFunction(async () =>
      (await window.__native.invoke('agent.context')).page.selection?.includes('THIRD-PAGE'));
    const selected = (await app.invoke('agent.context')).page.selection;
    for (const marker of ['FIRST-PAGE', 'SECOND-PAGE', 'THIRD-PAGE']) {
      assert.equal(selected.split(marker).length - 1, 1, 'multi-page selections are not duplicated');
    }
  }],
  ['native PDF find cycles case-insensitive matches across pages and wraps', async () => {
    await app.invoke('nav.open', { url: `${base}/find.pdf` });
    await waitPdf('Revenue report');
    await app.shell.evaluate(() => { window.__pdfFindResults = []; });
    await app.invoke('find.start', { query: 'revenue' });
    await app.shell.waitForFunction(() => window.__pdfFindResults.at(-1)?.total === 3);
    for (const active of [2, 3, 1]) {
      await app.invoke('find.next', { query: 'revenue' });
      await app.shell.waitForFunction(expected => window.__pdfFindResults.at(-1)?.active === expected, active);
    }
    await app.invoke('find.prev', { query: 'revenue' });
    await app.shell.waitForFunction(() => window.__pdfFindResults.at(-1)?.active === 3);
    assert.equal((await app.invoke('agent.context')).page.selection, null, 'find matches do not become River selections');
    await app.invoke('find.stop');
  }],
  ['browser zoom changes native PDF text geometry without raster canvases', async () => {
    await app.invoke('nav.open', { url: `${base}/doc.pdf` });
    await waitPdf('Quarterly revenue');
    const before = await nativePdfText(app.child.pid, 'Quarterly revenue', 'Quarterly revenue');
    await app.shell.locator('#zoom-in').click();
    await sleep(400);
    const after = await nativePdfText(app.child.pid, 'Quarterly revenue', 'Quarterly revenue');
    assert.ok(after.bounds[0].height > before.bounds[0].height * 1.04,
      `text scales natively: ${before.bounds[0].height} -> ${after.bounds[0].height}`);
    await app.shell.locator('#zoom-label').click();
  }],
  ['native PDF Ctrl+scroll zooms in and out with the browser toolbar in sync', async () => {
    const page = await app.pageWith(/\/doc\.pdf$/);
    const measure = async () => {
      const pdf = await nativePdfText(app.child.pid, 'Quarterly revenue', 'Quarterly revenue');
      return { height: pdf.bounds[0]?.height, zoom: parseInt(await app.shell.locator('#zoom-label').textContent(), 10) };
    };
    const wheel = async deltaY => {
      await page.mouse.move(350, 250);
      await page.keyboard.down('Control');
      try { await page.mouse.wheel(0, deltaY); } finally { await page.keyboard.up('Control'); }
      await sleep(600);
    };
    const before = await measure();
    await wheel(-120);
    const enlarged = await measure();
    assert.ok(enlarged.height > before.height * 1.04, `Ctrl+scroll up: ${JSON.stringify({ before, enlarged })}`);
    assert.ok(enlarged.zoom > before.zoom, `toolbar follows Ctrl+scroll: ${JSON.stringify({ before, enlarged })}`);
    await wheel(120);
    const reduced = await measure();
    assert.ok(reduced.height < enlarged.height / 1.04, `Ctrl+scroll down: ${JSON.stringify({ enlarged, reduced })}`);
    assert.ok(reduced.zoom < enlarged.zoom, `toolbar follows zoom out: ${JSON.stringify({ enlarged, reduced })}`);
    assert.ok(Math.abs(reduced.height / before.height - reduced.zoom / before.zoom) < 0.04, 'toolbar percentage matches actual PDF text scaling');
    await page.mouse.wheel(0, 60);
    await sleep(300);
    const scrolled = await measure();
    assert.equal(scrolled.zoom, reduced.zoom, 'ordinary scrolling does not change toolbar zoom');
    assert.ok(Math.abs(scrolled.height - reduced.height) < 1, 'ordinary scrolling does not zoom PDF text');
    await app.shell.locator('#zoom-label').click();
    const reset = await measure();
    assert.equal(reset.zoom, 100, 'Reset restores 100% in the toolbar');
    assert.ok(Math.abs(reset.height - before.height) < 1, 'Reset restores the original native PDF text size');
  }],
  ['dropping a local PDF replaces the page and preserves toolbar Back and Forward', async () => {
    const previousUrl = base + '/before-drop';
    await app.invoke('nav.open', { url: previousUrl });
    const page = await app.pageWith(/\/before-drop$/);
    await page.locator('h1').waitFor();
    const before = await app.invoke('agent.context');
    await dropFile(page, 'h1');
    await page.waitForURL(localUrl);
    await waitPdf('Local dropped PDF');
    assert.match(JSON.stringify(await preview()), /Local dropped PDF document/);
    await nativePdfText(app.child.pid, 'Local dropped PDF', 'Local dropped PDF');
    await app.shell.waitForFunction(async () =>
      (await window.__native.invoke('agent.context')).page.selection === 'Local dropped PDF');
    await assertRiverSelection('Local dropped PDF');
    const after = await app.invoke('agent.context');
    assert.equal(after.page.tab_id, before.page.tab_id, 'opens in the same page, not another tab');
    assert.equal(after.tabs.length, before.tabs.length);
    assert.equal(after.page.url, localUrl);
    await app.shell.locator('#btn-back').click();
    await page.waitForURL(previousUrl);
    await page.locator('h1').waitFor();
    await app.shell.locator('#btn-forward').click();
    await page.waitForURL(localUrl);
    await waitPdf('Local dropped PDF');
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
    await right.waitForURL(localUrl);
    await waitPdf('Local dropped PDF');
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
    await right.waitForURL(localUrl);
    await waitPdf('Local dropped PDF');
    assert.equal(left.url(), urls[0]);
  }],
  ['River reads and selects only the focused native PDF canvas cell', async () => {
    await app.invoke('canvas.create', { cols: 2, rows: 1, urls: [base + '/left.pdf', base + '/right.pdf'] });
    await waitPdf('LEFT-NATIVE-PDF');
    await waitPdf('RIGHT-NATIVE-PDF');
    for (const [cell, marker, other] of [[0, 'LEFT-NATIVE-PDF', 'RIGHT-NATIVE-PDF'], [1, 'RIGHT-NATIVE-PDF', 'LEFT-NATIVE-PDF']]) {
      await app.invoke('canvas.focus', { cell });
      const content = JSON.stringify(await preview());
      assert.ok(content.includes(marker), content);
      assert.ok(!content.includes(other), content);
      await nativePdfText(app.child.pid, marker, marker);
      await app.shell.waitForFunction(async expected =>
        (await window.__native.invoke('agent.context')).page.selection === expected, marker);
    }
  }],
  ['Ctrl+scroll in another PDF canvas cell focuses and zooms only that cell', async () => {
    const height = async marker => (await nativePdfText(app.child.pid, marker, marker)).bounds[0].height;
    const beforeLeft = await height('LEFT-NATIVE-PDF');
    const beforeRight = await height('RIGHT-NATIVE-PDF');
    await app.invoke('canvas.focus', { cell: 0 });
    const beforeZoom = parseInt(await app.shell.locator('#zoom-label').textContent(), 10);
    await app.invoke('canvas.focus', { cell: 1 });
    await app.shell.evaluate(() => {
      window.__canvasZoomEvents = [];
      window.__native.on('tab.zoom_changed', value => window.__canvasZoomEvents.push(value));
    });
    const left = await app.pageWith(/\/left\.pdf$/);
    await left.mouse.move(160, 240);
    await left.keyboard.down('Control');
    try { await left.mouse.wheel(0, -120); } finally { await left.keyboard.up('Control'); }
    await app.shell.waitForFunction(async () =>
      (await window.__native.invoke('agent.context')).page.url.endsWith('/left.pdf'));
    const afterLeft = await height('LEFT-NATIVE-PDF');
    const afterRight = await height('RIGHT-NATIVE-PDF');
    const label = await app.shell.locator('#zoom-label').textContent();
    const events = await app.shell.evaluate(() => window.__canvasZoomEvents);
    assert.ok(parseInt(label, 10) > beforeZoom, JSON.stringify({ beforeZoom, label, events, beforeLeft, afterLeft, beforeRight, afterRight }));
    assert.ok(afterLeft > beforeLeft * 1.04);
    assert.ok(Math.abs(afterRight - beforeRight) < 1, 'neighbor PDF retains its own zoom');
    await app.shell.locator('#zoom-label').click();
    assert.equal(parseInt(await app.shell.locator('#zoom-label').textContent(), 10), 100);
    assert.ok(Math.abs(await height('LEFT-NATIVE-PDF') / beforeLeft - 100 / beforeZoom) < 0.05, 'Reset sets the targeted PDF to actual 100%');
    assert.ok(Math.abs(await height('RIGHT-NATIVE-PDF') - beforeRight) < 1, 'Reset does not change the neighboring PDF');
    await app.invoke('nav.open', { url: base + '/after-native-pdf' });
    const page = await app.pageWith(/\/after-native-pdf$/);
    await page.locator('h1').waitFor();
    assert.equal((await app.invoke('agent.context')).page.selection, null, 'navigation clears the old native selection');
    assert.match(JSON.stringify(await preview()), /Page before a drop/);
  }],
  ['identical PDF URLs in separate canvas cells retain distinct native selections', async () => {
    const before = new Set(app.ctx.pages());
    await app.invoke('canvas.create', { cols: 2, rows: 1, urls: [base + '/doc.pdf', base + '/doc.pdf'] });
    await waitPdf('Quarterly revenue');
    let pages = [];
    for (let attempt = 0; attempt < 100; attempt++) {
      pages = app.ctx.pages().filter(page => !before.has(page) && page.url() === base + '/doc.pdf');
      if (pages.length === 2) break;
      await sleep(100);
    }
    assert.equal(pages.length, 2);
    await nativePdfText(app.child.pid, 'Quarterly revenue', 'revenue');
    const selections = await Promise.all(pages.map(page => engineSelection(page)));
    const selected = selections.findIndex(text => text === 'revenue');
    assert.notEqual(selected, -1, JSON.stringify(selections));
    const other = 1 - selected;
    const full = await engineSelection(pages[other], { selectAll: true });
    assert.equal(full, 'Quarterly revenue grew twelve percent');
    assert.equal(await engineSelection(pages[selected]), 'revenue', 'the neighbor keeps its partial selection');
    for (const [cell, expected] of [[selected, 'revenue'], [other, full], [selected, 'revenue']]) {
      await app.invoke('canvas.focus', { cell });
      await app.shell.waitForFunction(async text =>
        (await window.__native.invoke('agent.context')).page.selection === text, expected);
    }
  }],
  ['an unavailable native selection protocol clears River rather than using inaccurate UIA text', async () => {
    await app.invoke('nav.open', { url: base + '/unavailable.pdf' });
    const page = await app.pageWith(/\/unavailable\.pdf$/);
    await nativePdfText(app.child.pid, 'Quarterly revenue', 'revenue');
    await app.shell.waitForFunction(async () =>
      (await window.__native.invoke('agent.context')).page.selection === 'revenue');
    await nativePdfEvaluate(page, `globalThis.__savedPdfPort = viewer.pluginController_.messagePort_;
      viewer.pluginController_.messagePort_ = null; 'unavailable'`);
    try {
      await nativePdfText(app.child.pid, 'Quarterly revenue', 'twelve percent');
      await app.shell.waitForFunction(async () =>
        (await window.__native.invoke('agent.context')).page.selection === null);
    } finally {
      await nativePdfEvaluate(page, `viewer.pluginController_.messagePort_ = globalThis.__savedPdfPort;
        delete globalThis.__savedPdfPort; 'restored'`);
    }
    await nativePdfText(app.child.pid, 'Quarterly revenue', 'revenue grew');
    await app.shell.waitForFunction(async () =>
      (await window.__native.invoke('agent.context')).page.selection === 'revenue grew');
  }],
  ['a delayed native PDF selection reply cannot repopulate River after navigation', async () => {
    const page = await app.pageWith(/\/unavailable\.pdf$/);
    await nativePdfEvaluate(page, `(() => {
      const controller = viewer.pluginController_;
      const previous = controller.postMessage;
      globalThis.__selectionReads = 0;
      controller.postMessage = function(message) {
        if (message.type !== 'getSelectedText') return previous.call(this, message);
        ++globalThis.__selectionReads;
        setTimeout(() => previous.call(this, message), 800);
      };
      return 'delayed';
    })()`);
    await nativePdfText(app.child.pid, 'Quarterly revenue', 'percent');
    assert.equal(await nativePdfEvaluate(page, 'String(globalThis.__selectionReads)'), '1', 'a native read is in flight');
    await app.invoke('nav.open', { url: base + '/selection-after-navigation', disposition: 'current' });
    await page.waitForURL(base + '/selection-after-navigation');
    await page.locator('h1').waitFor();
    await sleep(1000);
    assert.equal((await app.invoke('agent.context')).page.selection, null);
  }],
]);
} finally {
  await app.close();
  server.close();
  redirectTarget.server.close();
  model.close();
  fs.unlinkSync(localPath);
  fs.rmdirSync(localDirectory);
}
process.exitCode = failed ? 1 : 0;
