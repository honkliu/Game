// Edge-style split view: rounded panes, draggable dividers (programmatic and a real drag).
import assert from 'node:assert/strict';
import http from 'node:http';
import { execFileSync } from 'node:child_process';
import { launch, run, sleep, osDrag } from './harness.mjs';

const server = http.createServer((req, res) => {
  res.setHeader('content-type', 'text/html');
  res.end(`<body style="margin:0;background:${req.url.includes('b') ? '#fde' : '#def'}"><h1>${req.url}</h1></body>`);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const app = await launch({ debugPort: 9454, withProvider: false });
const state = async () => app.invoke('shell_canvas', { operation: 'state_get' });
const waitFor = async (fn, timeout = 15000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(150); }
  throw new Error('timed out');
};
const shot = (file) => {
  const sh = `Add-Type -AssemblyName System.Drawing,System.Windows.Forms
$p = Get-Process browser | Where-Object { $_.Path -like '*ai-target*' -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f); [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r); public struct RECT { public int L,T,R,B; }' -Name W -Namespace U
$r = New-Object U.W+RECT; [U.W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
$b = New-Object System.Drawing.Bitmap ($r.R-$r.L), ($r.B-$r.T)
$g = [System.Drawing.Graphics]::FromImage($b); $hdc = $g.GetHdc(); [U.W]::PrintWindow($p.MainWindowHandle, $hdc, 2) | Out-Null; $g.ReleaseHdc($hdc)
$b.Save('${file}')`;
  execFileSync('powershell', ['-NoProfile', '-Command', sh], { stdio: 'inherit' });
};

await run([
  ['a 1x2 canvas exposes one vertical divider between two touching, 4 px apart panes', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 1, urls: [`${base}/a`, `${base}/b`] });
    const s = await waitFor(async () => { const c = await state(); return c.cells?.length === 2 && c.dividers?.length === 1 && c; });
    const [a, b] = s.cells;
    assert.equal(b.body.x - (a.body.x + a.body.w), 4, JSON.stringify([a.body, b.body]));
    assert.ok(s.dividers[0].vertical);
  }],
  ['canvas.resize moves the boundary and keeps a minimum width', async () => {
    const before = await state();
    const x = before.cells[0].body.x + before.cells[0].body.w + 2;
    await app.invoke('shell_canvas', { operation: 'resize', vertical: true, index: 0, pos: x + 200 });
    const after = await waitFor(async () => { const c = await state(); return c.cells[0].body.w > before.cells[0].body.w + 150 && c; });
    assert.equal(after.cells[1].body.x - (after.cells[0].body.x + after.cells[0].body.w), 4);
    await app.invoke('shell_canvas', { operation: 'resize', vertical: true, index: 0, pos: 0 });
    const tiny = await waitFor(async () => { const c = await state(); return c.cells[0].body.w < before.cells[0].body.w / 2 && c; });
    assert.ok(tiny.cells[0].body.w > 50);
    await app.invoke('shell_canvas', { operation: 'resize', reset: true });
    await waitFor(async () => Math.abs((await state()).cells[0].body.w - before.cells[0].body.w) <= 1);
  }],
  ['dragging the divider with the mouse resizes the panes (pointer events on the shell)', async () => {
    const s = await state();
    const d = s.dividers[0].rect;
    const x = d.x + d.w / 2, y = d.y + d.h / 2;
    await app.shell.mouse.move(x, y);
    await app.shell.mouse.down();
    await app.shell.mouse.move(x + 120, y, { steps: 6 });
    await app.shell.mouse.move(x + 240, y, { steps: 6 });
    await app.shell.mouse.up();
    const after = await waitFor(async () => { const c = await state(); return c.cells[0].body.w > s.cells[0].body.w + 150 && c; });
    assert.equal(after.cells[1].body.x - (after.cells[0].body.x + after.cells[0].body.w), 4);
  }],
  ['double-clicking a divider restores equal panes', async () => {
    const s = await state();
    const d = s.dividers[0].rect;
    await app.shell.mouse.dblclick(d.x + d.w / 2, d.y + d.h / 2);
    await waitFor(async () => { const c = await state(); return Math.abs(c.cells[0].body.w - c.cells[1].body.w) <= 1; });
  }],
  ['the grid button next to + opens a layout picker that creates the chosen canvas', async () => {
    await app.shell.evaluate(() => document.getElementById('new-canvas').click());
    const items = await waitFor(async () => { const i = await app.shell.evaluate(() => [...document.querySelectorAll('#item-menu .menu-item')].map((b) => b.textContent)); return i.length >= 8 && i; });
    assert.ok(items.some((l) => /2 × 2/.test(l)));
    await sleep(600);
    shot(process.env.SHOT_MENU || 'menu.png');
    await app.shell.evaluate(() => [...document.querySelectorAll('#item-menu .menu-item')].find((b) => /2 × 2/.test(b.textContent)).click());
    await waitFor(async () => (await state()).cells?.length === 4);
  }],
  ['in a 2x2 each row resizes its columns independently', async () => {
    await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2, urls: [`/a`, `/b`, `/a`, `/b`] });
    const s = await waitFor(async () => { const c = await state(); return c.cells?.length === 4 && c.dividers.filter((d) => d.vertical).length === 2 && c; });
    const top = s.dividers.find((d) => d.vertical && d.row === 0);
    await app.invoke('shell_canvas', { operation: 'resize', vertical: true, index: 0, row: 0, pos: top.rect.x + 250 });
    const a = await waitFor(async () => { const c = await state(); return c.cells[0].body.w > s.cells[0].body.w + 150 && c; });
    assert.equal(a.cells[2].body.w, s.cells[2].body.w, 'bottom-left unchanged');
    assert.equal(a.cells[3].body.w, s.cells[3].body.w, 'bottom-right unchanged');
  }],
  ['every tab header has its own colour', async () => {
    await app.invoke('shell_tab', { operation: 'open', url: base + '/c' });
    await app.invoke('shell_tab', { operation: 'open', url: base + '/d' });
    const cols = await waitFor(async () => { const c = await app.shell.evaluate(() => [...document.querySelectorAll('.tab-row')].map((r) => getComputedStyle(r).backgroundColor)); return c.length >= 3 && c; });
    assert.equal(new Set(cols).size, cols.length, JSON.stringify(cols));
  }],
  ['pane corners are rounded (screenshot saved)', async () => {
    await sleep(500);
    shot(process.env.SHOT || 'split.png');
  }],
]);

await app.close();
server.close();
