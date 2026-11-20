import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, it, vi } from 'vitest';

it('native chrome sends canonical operations while retaining empty cells, monitor order, find and dragging', async () => {
  document.documentElement.innerHTML = readFileSync(resolve(process.cwd(), '../crates/shell/ui/index.html'), 'utf8');
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 0));
  const sent: { id: number; cmd: string; payload: Record<string, unknown> }[] = [];
  const layout = { cols: 2, rows: 1, display_ids: ['right', 'left'] };
  const windowState = { selected_layout: layout, displays: ['left', 'right'].map((id, i) => ({
    id, number: i + 1, resolution: [1920, 1080], scale: 1, name: id,
  })) };
  window.ipc = { postMessage(raw) {
    const message = JSON.parse(raw);
    sent.push(message);
    queueMicrotask(() => window.__native!._reply({ id: message.id, ok: true,
      data: message.cmd === 'shell_window' && message.payload.operation === 'state_get' ? windowState : null }));
  } };
  const click = (id: string) => document.getElementById(id)!.click();
  const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
  try {
    new Function(readFileSync(resolve(process.cwd(), '../crates/shell/ui/app.js'), 'utf8'))();
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

    window.__native!._dispatch({ event: 'state.tabs', payload: {
      tabs: [{ id: 1, url: 'https://example.test', title: 'Page', active: true }], groups: [], active_tab_id: 1,
      canvas: { focus: 0, cells: [{ index: 0, tab_id: null, focused: true,
        frame: { x: 0, y: 0, w: 200, h: 200 }, body: { x: 0, y: 0, w: 200, h: 200 } }] },
    } });
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
  } finally {
    await flush();
    vi.unstubAllGlobals();
    delete window.ipc;
    delete window.__native;
  }
});
