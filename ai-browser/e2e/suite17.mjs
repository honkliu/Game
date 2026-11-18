// Native two-monitor caption button; changes only this test browser's window.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { launch, startFixtures, run, sleep, nativeHitBounds } from './harness.mjs';

const fx = await startFixtures();
const debugPort = 16000 + Math.floor(Math.random() * 30000);
let app = await launch({ debugPort, withProvider: false });
const state = () => app.invoke('window.state');
const waitFor = async fn => {
  const end = Date.now() + 15000;
  while (Date.now() < end) {
    const result = await fn();
    if (result) return result;
    await sleep(100);
  }
  const { bounds, client_bounds, span_bounds, spanning, maximized, minimized } = await state();
  throw new Error(`timed out waiting for window state: ${JSON.stringify({ bounds, client_bounds, span_bounds, spanning, maximized, minimized, initial: initial?.bounds })}`);
};
const same = (a, b) => ['x', 'y', 'width', 'height'].every(key => Math.abs(a[key] - b[key]) <= 1);
const click = selector => app.shell.evaluate(selector => document.querySelector(selector).click(), selector);
let initial;
let chosen;
const restart = async edit => {
  const profilePath = app.profile;
  await app.invoke('window.close').catch(() => {});
  await waitFor(() => app.child.exitCode !== null);
  await app.browser.close();
  if (edit) edit(path.join(app.data, 'settings.json'));
  app = await launch({ debugPort, withProvider: false, profilePath });
  await waitFor(() => app.invoke('window.state'));
};
let failed;
try {
  failed = await run([
    ['a distinct two-monitor button sits between maximize and close', async () => {
      initial = await state();
      assert.deepEqual(await app.shell.locator('#win-ctl button').evaluateAll(buttons => buttons.map(button => button.id)), ['win-min', 'win-max', 'win-span', 'win-span-menu', 'win-close']);
      await waitFor(async () => await app.shell.locator('#win-span').isDisabled() === !initial.can_span);
      assert.equal(await app.shell.locator('#win-span').getAttribute('aria-pressed'), 'false');
      await nativeHitBounds(app.shell, '#win-span', app.child.pid);
      if (!initial.can_span) {
        await assert.rejects(app.invoke('window.toggle_span'), /second display/);
        assert.ok(same((await state()).bounds, initial.bounds));
      } else {
        const displays = initial.displays.filter(display => display.id);
        chosen = [displays.at(-1), displays.at(-2)].map(display => display.id);
        await click('#win-span');
        await app.shell.locator('#monitor-picker').waitFor({ state: 'visible' });
        assert.ok(same((await state()).bounds, initial.bounds), 'first click does not guess any monitors');
        assert.equal(await app.shell.locator('#monitor-picker-apply').isDisabled(), true);
        await app.shell.locator('#monitor-slot-0').press('Escape');
        await app.shell.locator('#monitor-picker').waitFor({ state: 'hidden' });
        assert.equal((await state()).selected_layout, null);
      }
    }],
    ['one click fills two displays and a second restores the original window', async () => {
      if (!initial.can_span) return console.log('SKIP  second display not available; geometry cases covered by unit tests');
      const base = `http://127.0.0.1:${fx.port}`;
      await app.invoke('canvas.create', { cols: 2, rows: 1, urls: [`${base}/?span=0`, `${base}/?span=1`] });
      await click('#win-span-menu');
      await app.shell.locator('#monitor-picker').waitFor({ state: 'visible' });
      await nativeHitBounds(app.shell, '#monitor-slot-0', app.child.pid);
      await app.shell.locator('#monitor-slot-0').selectOption(chosen[0]);
      await app.shell.locator('#monitor-slot-1').selectOption(chosen[0]);
      assert.equal(await app.shell.locator('#monitor-picker-apply').isDisabled(), true);
      await app.shell.locator('#monitor-slot-1').selectOption(chosen[1]);
      await app.shell.locator('#monitor-picker-apply').click();
      await app.shell.locator('#monitor-picker').waitFor({ state: 'hidden' });
      initial.span_bounds = (await state()).span_bounds;
      const spanned = await waitFor(async () => { const s = await state(); return s.spanning && same(s.client_bounds, initial.span_bounds) && s; });
      assert.deepEqual(spanned.selected_layout.display_ids, chosen, 'never sort the user assignments');
      assert.equal(spanned.maximized, false);
      await waitFor(async () => await app.shell.locator('#win-span').getAttribute('aria-pressed') === 'true');
      await sleep(500);
      const viewport = await app.shell.evaluate(() => ({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio }));
      assert.ok(Math.abs(viewport.width * viewport.dpr - spanned.client_bounds.width) <= 2, JSON.stringify({ viewport, bounds: spanned.client_bounds }));
      assert.ok(Math.abs(viewport.height * viewport.dpr - spanned.client_bounds.height) <= 2, JSON.stringify({ viewport, bounds: spanned.client_bounds }));
      const canvas = await app.invoke('canvas.state');
      assert.ok(canvas.cells.every(cell => cell.body.w > 100 && cell.body.x + cell.body.w <= viewport.width + 1));
      await click('#win-span');
      await waitFor(async () => { const s = await state(); return !s.spanning && same(s.bounds, initial.bounds); });
      await waitFor(async () => await app.shell.locator('#win-span').getAttribute('aria-pressed') === 'false');
    }],
    ['the picker supports explicit 2x2 slots without overwriting a saved layout on cancel', async () => {
      if (!initial.can_span) return;
      await click('#win-span-menu');
      await app.shell.locator('#monitor-picker').waitFor({ state: 'visible' });
      await app.shell.locator('#monitor-layout').selectOption('2x2');
      assert.equal(await app.shell.locator('#monitor-slots select').count(), 4);
      assert.deepEqual(await app.shell.locator('#monitor-slots select').evaluateAll(selects => selects.map(select => select.getAttribute('aria-label'))), ['Top left monitor', 'Top right monitor', 'Bottom left monitor', 'Bottom right monitor']);
      assert.equal(await app.shell.locator('#monitor-picker-apply').isDisabled(), true);
      await click('#monitor-picker-cancel');
      assert.deepEqual((await state()).selected_layout, { cols: 2, rows: 1, display_ids: chosen });
      await assert.rejects(app.invoke('window.span_select', { cols: 2, rows: 1, display_ids: [chosen[0], chosen[0]], span: false }), /different connected/);
      assert.deepEqual((await state()).selected_layout.display_ids, chosen);
    }],
    ['spanning from maximized restores maximize and retains the original normal bounds', async () => {
      if (!initial.can_span) return;
      await click('#win-max');
      const maximized = await waitFor(async () => { const s = await state(); return s.maximized && s; });
      await click('#win-span');
      await waitFor(async () => (await state()).spanning);
      await click('#win-span');
      await waitFor(async () => { const s = await state(); return s.maximized && !s.spanning && same(s.bounds, maximized.bounds); });
      await click('#win-max');
      await waitFor(async () => { const s = await state(); return !s.maximized && same(s.bounds, initial.bounds); });
    }],
    ['normal maximize exits spanning and still restores the original normal window', async () => {
      if (!initial.can_span) return;
      await click('#win-span');
      await waitFor(async () => (await state()).spanning);
      await click('#win-max');
      await waitFor(async () => { const s = await state(); return s.maximized && !s.spanning; });
      await click('#win-max');
      await waitFor(async () => { const s = await state(); return !s.maximized && same(s.bounds, initial.bounds); });
    }],
    ['minimizing does not discard the span restore point', async () => {
      if (!initial.can_span) return;
      await click('#win-span');
      await waitFor(async () => (await state()).spanning);
      await app.invoke('window.minimize');
      await waitFor(async () => (await state()).minimized);
      assert.equal((await state()).spanning, true);
      await app.invoke('window.toggle_span');
      await waitFor(async () => { const s = await state(); return !s.spanning && !s.minimized && same(s.bounds, initial.bounds); });
    }],
    ['the extra control leaves the vertical toolbar clickable', async () => {
      await app.invoke('settings.update', { key: 'vertical_tabs', value: true });
      await nativeHitBounds(app.shell, '#win-span', app.child.pid);
      const boxes = await app.shell.evaluate(() => {
        const controls = document.getElementById('win-ctl').getBoundingClientRect();
        const settings = document.getElementById('btn-settings').getBoundingClientRect();
        return { controls: controls.left, settings: settings.right };
      });
      assert.ok(boxes.settings <= boxes.controls, JSON.stringify(boxes));
      await nativeHitBounds(app.shell, '#win-drag', app.child.pid);
      assert.equal(await app.shell.locator('#win-drag').isVisible(), true);
    }],
    ['a crowded restored header keeps a drag handle and tabs distinguish clicks from window drags', async () => {
      await app.invoke('settings.update', { key: 'vertical_tabs', value: false });
      if (initial.can_span) {
        await click('#win-span');
        await waitFor(async () => (await state()).spanning);
        await click('#win-span');
        await waitFor(async () => { const s = await state(); return !s.spanning && same(s.bounds, initial.bounds); });
      }
      const result = await app.shell.evaluate(() => {
        window.__native._dispatch({ event: 'state.tabs', payload: {
          tabs: Array.from({ length: 80 }, (_, i) => ({ id: 10000 + i, title: `Crowded tab ${i}`, url: 'https://example.test/', active: i === 0 })), groups: [], canvas: null,
        } });
        const row = document.querySelector('#rail-list .tab-row');
        const ipc = window.chrome.webview;
        const original = ipc.postMessage;
        const commands = [];
        ipc.postMessage = body => {
          const message = JSON.parse(body);
          if (['window.drag', 'tab.select', 'tab.close'].includes(message.cmd)) {
            commands.push(message.cmd);
            window.__native._reply({ id: message.id, ok: true, data: null });
          } else original.call(ipc, body);
        };
        const pointer = (target, type, x, y = 16, extra = {}) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, button: 0, buttons: type === 'pointerup' ? 0 : 1, pointerId: 7, clientX: x, clientY: y, ...extra }));
        try {
          pointer(row, 'pointerdown', 20);
          pointer(row, 'pointermove', 22);
          pointer(row, 'pointerup', 22);
          row.click();
          pointer(row, 'pointerdown', 20);
          pointer(window, 'pointermove', 40);
          pointer(window, 'pointerup', 40);
          row.click();
          const close = row.querySelector('.tab-close');
          pointer(close, 'pointerdown', 20);
          pointer(window, 'pointermove', 40);
          pointer(close, 'pointerup', 40);
          close.click();
          pointer(row, 'pointerdown', 20);
          pointer(window, 'pointerup', 20);
          pointer(window, 'pointermove', 40);
          pointer(row, 'pointerdown', 20, 16, { button: 2, buttons: 2 });
          pointer(window, 'pointermove', 40);
          pointer(window, 'pointerup', 40);
          document.getElementById('win-drag').dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, detail: 1 }));
          const drag = document.getElementById('win-drag').getBoundingClientRect();
          const controls = document.getElementById('win-ctl').getBoundingClientRect();
          const list = document.getElementById('rail-list');
          return { commands, width: drag.width, right: drag.right, controls: controls.left, overflow: list.scrollWidth > list.clientWidth };
        } finally { ipc.postMessage = original; }
      });
      assert.deepEqual(result.commands, ['tab.select', 'window.drag', 'tab.close', 'window.drag']);
      assert.ok(result.width >= 48 && result.right <= result.controls + 1, JSON.stringify(result));
      assert.equal(result.overflow, true);
      await nativeHitBounds(app.shell, '#win-drag', app.child.pid);
      await app.invoke('shell.ready');
    }],
    ['exact assignments persist across restart and disconnected displays reopen the picker', async () => {
      if (!initial.can_span) return;
      const reversed = [...chosen].reverse();
      await app.invoke('window.span_select', { cols: 2, rows: 1, display_ids: reversed, span: false });
      await restart();
      assert.deepEqual((await state()).selected_layout.display_ids, reversed);
      const before = (await state()).bounds;
      await click('#win-span');
      await waitFor(async () => (await state()).spanning);
      assert.equal(await app.shell.locator('#monitor-picker').isVisible(), false);
      await click('#win-span');
      await waitFor(async () => { const s = await state(); return !s.spanning && same(s.bounds, before); });
      await restart(file => {
        const settings = JSON.parse(fs.readFileSync(file, 'utf8'));
        settings.span_layout.display_ids[0] = 'disconnected-test-monitor';
        fs.writeFileSync(file, JSON.stringify(settings));
      });
      assert.equal((await state()).selection_missing, true);
      await click('#win-span');
      await app.shell.locator('#monitor-picker').waitFor({ state: 'visible' });
      assert.equal(await app.shell.locator('#monitor-picker-apply').isDisabled(), true);
      assert.equal((await state()).spanning, false);
      await click('#monitor-picker-cancel');
    }],
  ]);
} finally {
  if ((await state().catch(() => null))?.spanning) await app.invoke('window.toggle_span').catch(() => {});
  await app.close();
  fx.close();
}
process.exitCode = failed ? 1 : 0;
