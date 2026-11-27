// Native browser shortcuts, scoped to the private test window (no desktop input).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
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
const waitFor = async (fn, description = 'shortcut did not update browser state') => {
  const end = Date.now() + 10000;
  while (Date.now() < end) { if (await fn()) return; await sleep(50); }
  throw new Error(description);
};
const keysExe = path.join(app.profile, 'native-keys.exe');
const compileKeys = `
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
public class BrowserKeys {
  [StructLayout(LayoutKind.Sequential)] public struct GuiInfo {
    public uint size, flags;
    public IntPtr active, focus, capture, menu, move, caret;
    public int left, top, right, bottom;
  }
  [StructLayout(LayoutKind.Sequential)] public struct Message {
    public IntPtr window;
    public uint id;
    public IntPtr w, l;
    public uint time;
    public int x, y;
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
  [DllImport("user32.dll")] static extern bool PeekMessageW(out Message message, IntPtr window, uint first, uint last, uint remove);
  [DllImport("user32.dll")] static extern bool TranslateMessage(ref Message message);
  [DllImport("user32.dll")] static extern IntPtr DispatchMessageW(ref Message message);
  static void Pump(int milliseconds) {
    var timer = Stopwatch.StartNew();
    while (timer.ElapsedMilliseconds < milliseconds) {
      Message message;
      while (PeekMessageW(out message, IntPtr.Zero, 0, 0, 1)) {
        TranslateMessage(ref message); DispatchMessageW(ref message);
      }
      Thread.Sleep(1);
    }
  }
  public static void Main(string[] args) {
    IntPtr window = Process.GetProcessById(int.Parse(args[0])).MainWindowHandle;
    int key = int.Parse(args[1]), modifiers = int.Parse(args[2]);
    uint owner;
    uint target = GetWindowThreadProcessId(window, out owner), current = GetCurrentThreadId();
    var info = new GuiInfo(); info.size = (uint)Marshal.SizeOf(info);
    if (!GetGUIThreadInfo(target, ref info) || !IsChild(window, info.focus)) throw new Exception("No test WebView has focus");
    if (!AttachThreadInput(current, target, true)) throw new Exception("Cannot attach test input queue");
    var saved = new byte[256];
    try {
      if (!GetKeyboardState(saved)) throw new Exception("Cannot read test keyboard state");
      var state = (byte[])saved.Clone();
      state[0x11] = (byte)((modifiers & 1) != 0 ? 0x80 : 0);
      state[0x10] = (byte)((modifiers & 2) != 0 ? 0x80 : 0);
      state[0x12] = (byte)((modifiers & 4) != 0 ? 0x80 : 0);
      if (!SetKeyboardState(state)) throw new Exception("Cannot set test keyboard state");
      int scan = (int)MapVirtualKeyW((uint)key, 0) << 16;
      if (!PostMessageW(info.focus, 256, (IntPtr)key, (IntPtr)(scan | 1))) throw new Exception("Test keydown failed");
      Pump(250);
      PostMessageW(info.focus, 257, (IntPtr)key, (IntPtr)unchecked(scan | (int)0xc0000001));
      Pump(50);
    } finally { SetKeyboardState(saved); AttachThreadInput(current, target, false); }
  }
}
'@ -OutputAssembly '${keysExe.replaceAll("'", "''")}' -OutputType ConsoleApplication
`;
const press = (key, shift = false, ctrl = true, alt = false) => {
  execFileSync(keysExe, [String(app.child.pid), String(key), String(Number(ctrl) | Number(shift) << 1 | Number(alt) << 2)], { stdio: 'pipe', timeout: 10000 });
};

let failed;
try {
  execFileSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', compileKeys], { stdio: 'pipe', timeout: 30000 });
  await app.shell.evaluate(() => {
    window.__nativeShortcuts = [];
    window.__nativePanel = 'none';
    window.__shell.on('shell.shortcut', shortcut => window.__nativeShortcuts.push(shortcut));
    window.__shell.on('shell.panel', ({ panel }) => { window.__nativePanel = panel; });
  });
  failed = await run([
    ['native Ctrl+Tab switches rail tabs, not cells, and Ctrl+Shift+Tab reverses', async () => {
      const first = await app.invoke('shell_tab', { operation: 'open', url: url('standalone') });
      const grid = await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 2,
        urls: [0, 1, 2, 3].map(cell => url(`cell-${cell}`)) });
      await Promise.all([0, 1, 2, 3].map(async cell => {
        const page = await app.pageWith(new RegExp(`/cell-${cell}$`));
        await page.waitForSelector('h1');
      }));
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 1 });
      const page = await app.pageWith(/\/cell-1$/);
      await page.locator('#edit').focus();
      press(9, true);
      await waitFor(async () => (await tabs()).active_tab === first.tab_id);
      assert.equal(await canvas(), null);
      press(9);
      await waitFor(async () => (await canvas())?.id === grid.canvas_id);
      assert.equal((await canvas()).focus, 1);
    }],
    ['Ctrl+W pops a three-cell stack before advancing, even when the page traps keys', async () => {
      await app.invoke('shell_canvas', { operation: 'open', cols: 3, rows: 1, urls: [url('stack-source')] });
      const opener = await app.pageWith(/\/stack-source$/);
      await opener.waitForSelector('h1');
      for (const [name, cell] of [['stack-older', 1], ['stack-neighbor', 2], ['stack-latest', 1]]) {
        await opener.evaluate(u => window.open(u, '_blank'), url(name));
        await waitFor(async () => (await canvas()).cells[cell].url === url(name));
        const page = await app.pageWith(new RegExp(`/${name}$`));
        await page.waitForSelector('h1');
        if (name === 'stack-older') await page.locator('#edit').fill('preserve the covered page');
      }
      const older = await app.pageWith(/\/stack-older$/);
      const latest = await app.pageWith(/\/stack-latest$/);
      const before = await canvas();
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 1 });
      press(87);
      await waitFor(async () => (await canvas()).cells[1].url === url('stack-older'));
      const restored = await canvas();
      assert.equal(restored.focus, 1);
      for (const cell of [0, 2]) assert.equal(restored.cells[cell].tab_id, before.cells[cell].tab_id);
      assert.equal(await older.locator('#edit').inputValue(), 'preserve the covered page');
      await waitFor(() => latest.isClosed());
      assert.ok(!older.isClosed());
      for (const cell of [2, 0]) {
        press(87);
        await waitFor(async () => (await canvas()).focus === cell);
        const state = await canvas();
        assert.ok(state.cells[cell].tab_id !== null);
        if (cell === 2) {
          assert.equal(state.cells[1].tab_id, null);
          await waitFor(() => older.isClosed());
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
    ['Ctrl+T opens one new tab from a trapped page, browser chrome and River', async () => {
      for (const scope of ['page', 'chrome', 'River']) {
        await app.invoke('shell_tab', { operation: 'open', url: url(`new-tab-${scope}`) });
        const page = await app.pageWith(new RegExp(`/new-tab-${scope}$`));
        await page.waitForSelector('#edit');
        if (scope === 'page') {
          await page.locator('#edit').fill('page traps keyboard events');
        } else if (scope === 'chrome') {
          await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
          await app.shell.locator('#omnibox').fill('unfinished address');
        } else {
          await app.shell.locator('#btn-ai').click();
          const side = await app.pageWith(/sidebar\.html/);
          await side.locator('#ag-input').fill('unfinished River prompt');
        }
        const before = await tabs();
        press(84);
        await waitFor(async () => (await tabs()).tabs.length === before.tabs.length + 1);
        const after = await tabs();
        assert.notEqual(after.active_tab, before.active_tab, `${scope}: new tab becomes active`);
        assert.equal(after.tabs.find(tab => tab.tab_id === after.active_tab).url, 'native://newtab');
        await sleep(350);
        assert.equal((await tabs()).tabs.length, before.tabs.length + 1, `${scope}: one keypress opens exactly one tab`);
      }
    }],
    ...['page', 'chrome', 'River', 'palette'].map(scope => [
      `the complete native keymap dispatches exactly once from ${scope}`, async () => {
        const bindings = [
          [0x72, false, false, false, 'find_next'], [0x74, false, false, false, 'reload'],
          [0x75, false, false, false, 'cycle_focus'],
          [0x72, true, false, false, 'find_prev'], [0x74, true, false, false, 'reload_ignoring_cache'],
          [0x25, false, false, true, 'page_back'], [0x27, false, false, true, 'page_forward'],
          [0x24, false, false, true, 'home'], [0x44, false, false, true, 'focus_omnibox'],
          [0x25, false, true, true, 'rapid_prev_tab'], [0x27, false, true, true, 'rapid_next_tab'],
          [0x09, true, true, false, 'prev_tab'], [0x54, true, true, false, 'reopen_closed'],
          [0x4b, true, true, false, 'close_palette'], [0x48, true, true, false, 'open_history_today'],
          [0x47, true, true, false, 'find_prev'], [0x44, true, true, false, 'bookmark_all_tabs'],
          [0x52, true, true, false, 'reload_ignoring_cache'], [0x45, true, true, false, 'toggle_rail'],
          [0x50, true, true, false, 'toggle_pin'], [0x41, true, true, false, 'ask_ai'],
          [0x54, false, true, false, 'new_tab'], [0x57, false, true, false, 'close_tab'],
          [0x09, false, true, false, 'next_tab'], [0x30, false, true, false, 'zoom_reset'],
          ...Array.from({ length: 9 }, (_, i) => [0x31 + i, false, true, false, 'tab_index', i + 1]),
          [0x21, false, true, false, 'next_tab'], [0x22, false, true, false, 'prev_tab'],
          [0x4c, false, true, false, 'focus_omnibox'], [0x4b, false, true, false, 'open_palette'],
          [0x46, false, true, false, 'find'], [0x47, false, true, false, 'find_next'],
          [0x4a, false, true, false, 'open_downloads'], [0x48, false, true, false, 'open_history'],
          [0x52, false, true, false, 'reload'], [0x44, false, true, false, 'bookmark'],
          [0x55, false, true, false, 'view_source'], [0xbc, false, true, false, 'open_settings'],
          [0xbb, false, true, false, 'zoom_in'], [0xbb, true, true, false, 'zoom_in'],
          [0x6b, false, true, false, 'zoom_in'], [0x6b, true, true, false, 'zoom_in'],
          [0xbd, false, true, false, 'zoom_out'], [0x6d, false, true, false, 'zoom_out'],
        ];
        for (const [key, shift, ctrl, alt, name, args] of bindings) {
          const label = `${scope}: ${ctrl ? 'Ctrl+' : ''}${shift ? 'Shift+' : ''}${alt ? 'Alt+' : ''}0x${key.toString(16)} -> ${name}`;
          const before = await tabs();
          const opened = await app.invoke('shell_tab', { operation: 'open', url: url(`map-${scope}-${key}-${name}`) });
          const page = await app.pageWith(new RegExp(`/map-${scope}-${key}-${name}$`));
          await page.waitForSelector('#edit');
          await app.invoke('shell_ui', { operation: 'shortcut', name: 'close_palette' });
          if (scope === 'page') {
            await page.locator('#edit').focus();
          } else if (scope === 'chrome') {
            await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
          } else if (scope === 'River') {
            if (await app.shell.evaluate(() => window.__nativePanel !== 'ai')) {
              await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
            }
            const side = await app.pageWith(/sidebar\.html/);
            await side.locator('#ag-input').focus();
          } else {
            await app.invoke('shell_ui', { operation: 'shortcut', name: 'open_palette' });
            const overlay = await app.pageWith(/overlay\.html/);
            await overlay.locator('#pal-input').waitFor();
          }
          await app.shell.evaluate(() => { window.__nativeShortcuts = []; });
          try { press(key, shift, ctrl, alt); }
          catch (error) { throw new Error(`${label}: ${error.message}`); }
          await waitFor(() => app.shell.evaluate(() => window.__nativeShortcuts.length > 0), label);
          await sleep(100);
          const events = await app.shell.evaluate(() => window.__nativeShortcuts);
          assert.equal(events.length, 1, `${label}: no duplicate dispatch`);
          assert.equal(events[0].name, name, label);
          if (args !== undefined) assert.equal(events[0].args, args, label);
          if (['focus_omnibox', 'find'].includes(name)) {
            const id = name === 'find' ? 'find-input' : 'omnibox';
            await app.shell.waitForFunction(id => document.hasFocus() && document.activeElement?.id === id, id);
          } else if (name.startsWith('zoom_')) {
            const percent = name === 'zoom_in' ? '110%' : name === 'zoom_out' ? '91%' : '100%';
            await app.shell.waitForFunction(percent => document.querySelector('#zoom-label').textContent === percent, percent);
          } else if (['open_downloads', 'open_history', 'open_history_today', 'open_settings'].includes(name)) {
            await waitFor(async () => {
              const state = await tabs();
              return state.tabs.find(tab => tab.tab_id === state.active_tab)?.url === 'native://settings';
            }, label);
          } else if (name === 'home') {
            await waitFor(async () => (await tabs()).tabs.find(tab => tab.tab_id === opened.tab_id)?.url === 'native://newtab', label);
          } else if (name === 'close_tab') {
            await waitFor(async () => !(await tabs()).tabs.some(tab => tab.tab_id === opened.tab_id), label);
          }
          await app.invoke('shell_ui', { operation: 'shortcut', name: 'close_palette' });
          for (const tab of (await tabs()).tabs) {
            if (!before.tabs.some(old => old.tab_id === tab.tab_id)) {
              await app.invoke('shell_tab', { operation: 'close', tab_id: tab.tab_id });
            }
          }
        }
      },
    ]),
    ['F6 cycles native focus through page, address bar and rail', async () => {
      const opened = await app.invoke('shell_tab', { operation: 'open', url: url('focus-cycle') });
      const page = await app.pageWith(/\/focus-cycle$/);
      await page.locator('#edit').focus();
      press(0x75, false, false);
      await app.shell.waitForFunction(() => document.hasFocus() && document.activeElement?.id === 'omnibox');
      press(0x75, false, false);
      await app.shell.waitForFunction(() => document.hasFocus() && !!document.activeElement?.closest('.tab-row'));
      press(0x75, false, false);
      await page.waitForFunction(() => document.hasFocus());
      assert.equal((await tabs()).active_tab, opened.tab_id);
    }],
    ['Space and editing keys reach page inputs; Space opens the focused rail item menu', async () => {
      await app.invoke('shell_tab', { operation: 'open', url: url('typing') });
      const page = await app.pageWith(/\/typing$/);
      await page.locator('#edit').focus();
      await page.evaluate(() => {
        window.__keys = [];
        addEventListener('keydown', event => window.__keys.push(event.key));
      });
      for (const [key, ctrl] of [[0x20, false], [0x1b, false], [0x41, false], [0x41, true], [0x43, true], [0x56, true], [0x58, true], [0x5a, true]]) {
        const before = await page.evaluate(() => window.__keys.length);
        press(key, false, ctrl);
        await page.waitForFunction(before => window.__keys.length > before, before);
      }
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
      await app.shell.locator('.tab-row').first().focus();
      press(0x20, false, false);
      await app.shell.locator('#item-menu').waitFor({ state: 'visible' });
      press(0x1b, false, false);
      await app.shell.locator('#item-menu').waitFor({ state: 'hidden' });
    }],
    ['Escape dismisses the focused palette and find controls locally', async () => {
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'open_palette' });
      const overlay = await app.pageWith(/overlay\.html/);
      await overlay.locator('#pal-input').waitFor();
      press(0x1b, false, false);
      await overlay.locator('#pal-input').waitFor({ state: 'detached' });
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'find' });
      await app.shell.locator('#find-input').fill('needle');
      press(0x1b, false, false);
      await waitFor(async () => await app.shell.locator('#find-input').inputValue() === '');
    }],
    ['F12 opens the page inspector from page, chrome, River and palette focus', async () => {
      const session = await app.browser.newBrowserCDPSession();
      try {
        for (const scope of ['page', 'chrome', 'River', 'palette']) {
          await app.invoke('shell_tab', { operation: 'open', url: url(`inspect-${scope}`) });
          const page = await app.pageWith(new RegExp(`/inspect-${scope}$`));
          await page.waitForSelector('#edit');
          if (scope === 'page') await page.locator('#edit').focus();
          else if (scope === 'chrome') await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
          else if (scope === 'River') {
            if (await app.shell.evaluate(() => window.__nativePanel !== 'ai')) {
              await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
            }
            const side = await app.pageWith(/sidebar\.html/);
            await side.locator('#ag-input').focus();
          } else {
            await app.invoke('shell_ui', { operation: 'shortcut', name: 'open_palette' });
            const overlay = await app.pageWith(/overlay\.html/);
            await overlay.locator('#pal-input').waitFor();
          }
          press(0x7b, false, false);
          let inspector;
          await waitFor(async () => {
            inspector = (await session.send('Target.getTargets')).targetInfos.find(target =>
              target.url.startsWith('devtools://') && target.title.includes(`/inspect-${scope}`));
            return !!inspector;
          }, `${scope}: F12 did not open the page inspector`);
          await session.send('Target.closeTarget', { targetId: inspector.targetId });
          await app.invoke('shell_ui', { operation: 'shortcut', name: 'close_palette' });
        }
      } finally { await session.detach(); }
    }],
    ['Ctrl+Shift+W closes the private test window', async () => {
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' });
      press(0x57, true);
      await waitFor(() => app.child.exitCode !== null, 'Ctrl+Shift+W did not close the test window');
    }],
  ].filter(([name]) => !process.env.BROWSER_SHORTCUT_FILTER || name.includes(process.env.BROWSER_SHORTCUT_FILTER)));
} finally {
  await app.close();
  fixtures.server.close();
}
process.exitCode = failed ? 1 : 0;
