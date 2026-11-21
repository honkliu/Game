// Native browser shortcuts, scoped to the private test window (no desktop input).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { launch, listen, run, sleep } from './harness.mjs';

const fixtures = await listen((_req, res) => {
  res.setHeader('content-type', 'text/html');
  res.end('<!doctype html><h1>Shortcut fixture</h1><input id="edit"><script>addEventListener("keydown", e => e.preventDefault(), true)</script>');
});
const reservation = await listen((_req, res) => res.end());
const debugPort = reservation.port;
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ debugPort, withProvider: false });
const url = name => `http://127.0.0.1:${fixtures.port}/${name}`;
const canvas = () => app.invoke('shell_canvas', { operation: 'state_get' });
const tabs = () => app.invoke('shell_tab', { operation: 'list' });
const waitFor = async fn => {
  const end = Date.now() + 10000;
  while (Date.now() < end) { if (await fn()) return; await sleep(50); }
  throw new Error('shortcut did not update browser state');
};
const press = (key, shift = false) => {
  const command = `
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public class BrowserKeys {
  [StructLayout(LayoutKind.Sequential)] public struct GuiInfo {
    public uint size, flags;
    public IntPtr active, focus, capture, menu, move, caret;
    public int left, top, right, bottom;
  }
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
  [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent, IntPtr child);
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint from, uint to, bool attach);
  [DllImport("user32.dll")] static extern bool GetKeyboardState(byte[] state);
  [DllImport("user32.dll")] static extern bool SetKeyboardState(byte[] state);
  [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint message, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint key, uint kind);
  public static void Press(IntPtr window, int key, bool shift) {
    uint owner;
    uint target = GetWindowThreadProcessId(window, out owner), current = GetCurrentThreadId();
    var info = new GuiInfo(); info.size = (uint)Marshal.SizeOf(info);
    if (!GetGUIThreadInfo(target, ref info) || !IsChild(window, info.focus)) throw new Exception("No test WebView has focus");
    if (!AttachThreadInput(current, target, true)) throw new Exception("Cannot attach test input queue");
    var saved = new byte[256];
    try {
      if (!GetKeyboardState(saved)) throw new Exception("Cannot read test keyboard state");
      var state = (byte[])saved.Clone();
      state[0x11] = 0x80; state[0x10] = (byte)(shift ? 0x80 : 0); state[0x12] = 0;
      if (!SetKeyboardState(state)) throw new Exception("Cannot set test keyboard state");
      int scan = (int)MapVirtualKeyW((uint)key, 0) << 16;
      if (!PostMessageW(info.focus, 256, (IntPtr)key, (IntPtr)(scan | 1))) throw new Exception("Test keydown failed");
      Thread.Sleep(250);
      PostMessageW(info.focus, 257, (IntPtr)key, (IntPtr)unchecked(scan | (int)0xc0000001));
      Thread.Sleep(50);
    } finally { SetKeyboardState(saved); AttachThreadInput(current, target, false); }
  }
}
'@
[BrowserKeys]::Press((Get-Process -Id ${app.child.pid}).MainWindowHandle, ${key}, ${shift ? '$true' : '$false'})
`;
  execFileSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', command], { stdio: 'pipe', timeout: 10000 });
};

let failed;
try {
  failed = await run([
    ['native Ctrl+Tab switches rail tabs, not cells, and Ctrl+Shift+Tab reverses', async () => {
      const first = await app.invoke('shell_tab', { operation: 'open', url: url('standalone') });
      const grid = await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2,
        urls: [0, 1, 2, 3].map(cell => url(`cell-${cell}`)) });
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 1 });
      const page = await app.pageWith(/\/cell-1$/);
      await page.waitForSelector('h1');
      press(9, true);
      await waitFor(async () => (await tabs()).active_tab === first.tab_id);
      assert.equal(await canvas(), null);
      press(9);
      await waitFor(async () => (await canvas())?.id === grid.canvas_id);
      assert.equal((await canvas()).focus, 1);
    }],
    ['Ctrl+W clears every layer and advances 1, 2, 3, 0 even when the page traps keys', async () => {
      const opener = await app.pageWith(/\/cell-0$/);
      await opener.evaluate(u => window.open(u, '_blank'), url('covered-1'));
      await waitFor(async () => (await canvas()).cells[1].url === url('covered-1'));
      for (const cell of [2, 3, 0]) {
        press(87);
        await waitFor(async () => (await canvas()).focus === cell);
        const state = await canvas();
        assert.ok(state.cells[cell].tab_id !== null);
        if (cell === 2) {
          assert.equal(state.cells[1].tab_id, null);
          assert.ok(!app.ctx.pages().some(page => /\/(cell-1|covered-1)$/.test(page.url())));
        }
      }
      press(87);
      await waitFor(async () => (await canvas()).cells.every(cell => cell.tab_id === null));
      press(87);
      await waitFor(async () => (await canvas()) === null);
    }],
    ['Ctrl+W closes a standalone tab from an editable page', async () => {
      const opened = await app.invoke('shell_tab', { operation: 'open', url: url('editable') });
      const page = await app.pageWith(/\/editable$/);
      await page.locator('#edit').fill('keep shortcut handling at browser level');
      press(87);
      await waitFor(async () => !(await tabs()).tabs.some(tab => tab.tab_id === opened.tab_id));
    }],
    ['Ctrl+Tab also works from browser chrome and wraps at the end of the rail', async () => {
      const state = await tabs();
      const last = state.tabs.at(-1);
      await app.invoke('shell_tab', { operation: 'activate', tab_id: last.tab_id });
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
      await app.shell.locator('#omnibox').focus();
      press(9);
      await waitFor(async () => (await tabs()).active_tab === state.tabs[0].tab_id);
    }],
    ['Ctrl+W closes the current tab while the address field contains text', async () => {
      const opened = await app.invoke('shell_tab', { operation: 'open', url: url('chrome-close') });
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
      await app.shell.locator('#omnibox').fill('unfinished address');
      press(87);
      await waitFor(async () => !(await tabs()).tabs.some(tab => tab.tab_id === opened.tab_id));
    }],
  ]);
} finally {
  await app.close();
  fixtures.server.close();
}
process.exitCode = failed ? 1 : 0;
