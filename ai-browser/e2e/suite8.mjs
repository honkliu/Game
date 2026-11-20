// Native PDF canvas text, selection and zoom; physical drags require an interactive desktop.
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';
import { launch, run, sleep, osDrag, nativePdfText } from './harness.mjs';

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
    res.end(makePdf(req.url.startsWith('/pdf/') ? 'Extensionless PDF revenue report' : 'Quarterly revenue grew twelve percent'));
  } else {
    res.setHeader('content-type', 'text/html');
    res.end('<body><p id="t" style="font:20px sans-serif;margin:30px">Plain web page text to select here</p></body>');
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const app = await launch({ debugPort: 9453, withProvider: false });
const viewer = () => app.ctx.pages().find((p) => /\/doc\.pdf$/.test(p.url()));
const waitFor = async (fn, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};
const selection = async () => (await app.invoke('shell_agent', { operation: 'context_get' })).page.selection || '';
const desktopInputAvailable = execFileSync('powershell', ['-NoProfile', '-NonInteractive', '-Command',
  'Add-Type -MemberDefinition \'[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();\' -Name PdfDesktop -Namespace Native; [Native.PdfDesktop]::GetForegroundWindow() -ne [IntPtr]::Zero'],
  { encoding: 'utf8' }).trim() === 'True';
if (!desktopInputAvailable) console.log('SKIP  native PDF OS drag: the interactive input desktop is unavailable');

async function pointerDrag(page, x1, y, x2) {
  await page.mouse.move(x1, y);
  await page.mouse.down();
  try { await page.mouse.move(x2, y, { steps: 12 }); } finally { await page.mouse.up(); }
}

async function dragNativeText(marker) {
  const { bounds } = await nativePdfText(app.child.pid, marker, marker, { clearSelection: true });
  const r = bounds[0];
  assert.ok(r?.width > 0 && r?.height > 0, 'native text has screen geometry');
  await osDrag(r.x + 1, r.y + r.height / 2, r.x + r.width - 1, r.y + r.height / 2, app.child.pid);
}

try {
process.exitCode = await run([
  ['2x2 canvas: a web page and a PDF', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2, urls: [`${base}/`, `${base}/doc.pdf`] });
    const v = await waitFor(() => viewer());
    await nativePdfText(app.child.pid, 'Quarterly revenue');
    assert.equal(await v.locator('.textLayer, .page canvas').count(), 0, 'native rendering, not PDF.js');
  }],
  ['web cell: a trusted pointer drag selects text and the context shows it', async () => {
    const page = app.ctx.pages().find((p) => p.url() === `${base}/`);
    const r = await page.locator('#t').boundingBox();
    await pointerDrag(page, r.x + 1, r.y + r.height / 2, r.x + r.width - 1);
    await waitFor(async () => /Plain web/.test(await selection()));
  }],
  ...(desktopInputAvailable ? [['PDF cell: an OS pointer drag selects native text and the context shows it', async () => {
    await dragNativeText('Quarterly revenue');
    await waitFor(async () => /Quarterly/.test(await selection()));
    const ctx = await app.invoke('shell_agent', { operation: 'context_get' });
    assert.match(ctx.page.selection || '', /Quarterly/, JSON.stringify(ctx.page));
  }]] : []),
  ['PDF cell: zoom buttons and ctrl+wheel really scale the pages', async () => {
    await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 1 });
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'zoom_reset' });
    const v = viewer();
    const height = async () => (await nativePdfText(app.child.pid, 'Quarterly revenue', 'Quarterly revenue')).bounds[0].height;
    const h0 = await height();
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'zoom_in' });
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'zoom_in' });
    assert.ok(await height() > h0 * 1.05);
    await v.mouse.move(160, 200);
    await v.keyboard.down('Control');
    try { await v.mouse.wheel(0, -120); } finally { await v.keyboard.up('Control'); }
    await sleep(500);
    assert.ok(await height() > h0 * 1.12);
    assert.ok(parseInt(await app.shell.locator('#zoom-label').textContent(), 10) > 120);
    await app.invoke('shell_ui', { operation: 'shortcut', name: 'zoom_reset' });
    await sleep(500);
    assert.ok(Math.abs(await height() - h0) < 1, 'Reset restores native text geometry');
  }],
  ['extensionless PDF url (like arxiv.org/pdf/ID) also gets the viewer and a selectable text', async () => {
    await app.invoke('shell_canvas', { operation: 'cell_open', cell: 2, url: `${base}/pdf/1234` });
    const v = await app.pageWith(/\/pdf\/1234$/);
    assert.ok(v, 'second viewer');
    await nativePdfText(app.child.pid, 'Extensionless PDF');
    await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 2 });
    await nativePdfText(app.child.pid, 'Extensionless PDF', 'Extensionless PDF');
    await waitFor(async () => /Extensionless/.test(await selection()));
    assert.match(await selection(), /Extensionless/);
    assert.ok(JSON.stringify(await app.invoke('shell_agent', { operation: 'context_get' })).includes('/pdf/1234'), 'tab keeps the real url');
  }],
  ['local file:// PDF: the inner native provider supplies document text and selection', async () => {
    const file = path.join(app.profile, 'ab-local-test.pdf');
    fs.writeFileSync(file, makePdf('Local PDF revenue report'));
    await app.invoke('shell_canvas', { operation: 'cell_open', cell: 3, url: pathToFileURL(file).href });
    const v = await app.pageWith(/ab-local-test\.pdf$/);
    await nativePdfText(app.child.pid, 'Local PDF');
    await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 3 });
    assert.ok(v);
    await waitFor(async () => (await app.invoke('shell_agent', { operation: 'context_get' })).content?.preview?.includes('Local PDF'));
    await nativePdfText(app.child.pid, 'Local PDF', 'Local PDF');
    await waitFor(async () => /Local PDF/.test(await selection()));
  }],
]);

} finally {
  await app.close();
  server.close();
}
