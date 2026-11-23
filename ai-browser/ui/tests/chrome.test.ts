import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, it, vi } from 'vitest';
import type { Listener, ShellClient } from '../src/mcp';

it('overlay starts its MCP connection without an initial outgoing command and restores state on reconnect', async () => {
  document.body.innerHTML = '<div id="root"></div>';
  const listeners = new Map<string, Listener>();
  const invoke = vi.fn(async () => null);
  const ready = vi.fn(() => Promise.resolve());
  const client = {
    get ready() { return ready(); },
    invoke,
    on(event: string, listener: Listener) { listeners.set(event, listener); },
  };
  const source = readFileSync(resolve(process.cwd(), '../crates/shell/ui/overlay.js'), 'utf8').replace(/^import[^\n]+\n/, '');
  new Function('bridge', source)(() => client);
  expect(ready).toHaveBeenCalledTimes(1);
  listeners.get('mcp.connected')!({});
  expect(invoke).toHaveBeenLastCalledWith('shell_ui', { operation: 'ready' });
  listeners.get('mcp.resync')!({});
  expect(invoke).toHaveBeenLastCalledWith('shell_ui', { operation: 'ready' });
});

it('native chrome sends canonical operations while retaining empty cells, monitor order, find and dragging', async () => {
  document.documentElement.innerHTML = readFileSync(resolve(process.cwd(), '../crates/shell/ui/index.html'), 'utf8');
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 0));
  const sent: { id: number; cmd: string; payload: Record<string, unknown> }[] = [];
  const layout = { cols: 2, rows: 1, display_ids: ['right', 'left'] };
  const windowState = { selected_layout: layout, displays: ['left', 'right'].map((id, i) => ({
    id, number: i + 1, resolution: [1920, 1080], scale: 1, name: id,
  })) };
  const listeners = new Map<string, Listener>();
  const client = {
    ready: Promise.resolve(),
    invoke: vi.fn(async (cmd, payload = {}) => {
      sent.push({ id: sent.length + 1, cmd, payload });
      return cmd === 'shell_window' && payload.operation === 'state_get' ? windowState : null;
    }),
    on: (event: string, listener: Listener) => { listeners.set(event, listener); return () => listeners.delete(event); },
  } as unknown as ShellClient;
  const click = (id: string) => document.getElementById(id)!.click();
  const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
  try {
    const source = readFileSync(resolve(process.cwd(), '../crates/shell/ui/app.js'), 'utf8').replace(/^import[^\n]+\n/, '');
    new Function('bridge', source)(() => client);
    await flush();
    sent.length = 0;
    click('new-tab');
    click('btn-back');
    click('win-drag');
    document.getElementById('win-drag')!.dispatchEvent(new MouseEvent('mousedown', { button: 0 }));
    expect(sent.map(({ cmd, payload }) => [cmd, payload])).toEqual([
      ['shell_tab', { operation: 'open' }],
      ['shell_page_navigation', { operation: 'back' }],
      ['shell_window', { operation: 'drag' }],
    ]);

    listeners.get('state.tabs')!({
      tabs: [{ id: 1, url: 'https://example.test', title: 'Page', active: true }], groups: [], active_tab_id: 1,
      canvas: { focus: 0, cells: [{ index: 0, tab_id: null, focused: true,
        frame: { x: 0, y: 0, w: 200, h: 200 }, body: { x: 0, y: 0, w: 200, h: 200 } }] },
    });
    const empty = document.querySelector<HTMLElement>('.cv-empty')!;
    expect(empty.textContent).toBe('');
    sent.length = 0;
    empty.click();
    expect(sent.map(({ cmd, payload }) => [cmd, payload])).toEqual([['shell_canvas', { operation: 'cell_focus', cell: 0 }]]);

    const input = document.getElementById('find-input') as HTMLInputElement;
    input.value = 'needle';
    sent.length = 0;
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true }));
    expect(sent.map(({ cmd, payload }) => [cmd, payload.operation])).toEqual([
      ['shell_page_find', 'query'], ['shell_page_find', 'previous'],
    ]);

    click('win-span-menu');
    await flush();
    expect(document.getElementById('monitor-picker')!.hidden).toBe(false);
    expect((document.getElementById('monitor-slot-0') as HTMLSelectElement).value).toBe('right');
    expect((document.getElementById('monitor-slot-1') as HTMLSelectElement).value).toBe('left');
    click('monitor-picker-apply');
    await flush();
    expect(sent.find(message => message.payload.operation === 'span_select')).toMatchObject({
      cmd: 'shell_window', payload: { operation: 'span_select', ...layout },
    });
    expect(document.getElementById('monitor-picker')!.hidden).toBe(true);
    const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    sent.length = 0;
    listeners.get('shell.focus_changed')!({ scope: 'cycle' });
    expect(sent.map(({ cmd, payload }) => [cmd, payload])).toEqual([
      ['shell_ui', { operation: 'shortcut', name: 'focus_omnibox' }],
    ]);
    hasFocus.mockReturnValue(true);
    listeners.get('shell.focus_changed')!({ scope: 'omnibox', select_all: true });
    expect(document.activeElement?.id).toBe('omnibox');
    listeners.get('shell.focus_changed')!({ scope: 'cycle' });
    expect(document.activeElement?.closest('.tab-row')).not.toBeNull();
    sent.length = 0;
    listeners.get('shell.focus_changed')!({ scope: 'cycle' });
    expect(sent.filter(message => message.payload.operation === 'page_focus')).toHaveLength(1);

    click('win-span-menu');
    await flush();
    input.value = 'keep this search';
    listeners.get('shell.shortcut')!({ name: 'escape' });
    listeners.get('shell.escape')!({});
    expect(document.getElementById('monitor-picker')!.hidden).toBe(true);
    expect(input.value).toBe('keep this search');
  } finally {
    await flush();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
