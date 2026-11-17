import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { launch, listen, startFixtures, startModel, run, sleep } from './harness.mjs';

const waitFor = async (fn, ms = 15000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { const result = await fn(); if (result) return result; await sleep(75); }
  throw new Error('timed out waiting for condition');
};
async function debugPort() {
  const reservation = await listen((_req, res) => res.end());
  await new Promise(resolve => reservation.server.close(resolve));
  return reservation.port;
}

const fixtures = await startFixtures();
const base = `http://127.0.0.1:${fixtures.port}`;
const searchTemplate = `${base}/search?q=%s&source=a+b`;
const app = await launch({ debugPort: await debugPort(), withProvider: false });
let failed = 0;
try {
  await app.shell.waitForFunction(() => !!window.__native);
  failed += await run([
    ['new tab has history but no duplicate search control', async () => {
      const page = await app.pageWith(/pages\/newtab\.html/);
      assert.ok(page);
      assert.equal(await page.locator('.mirror').count(), 0);
      assert.equal(await page.locator('#recent').count(), 1);
      await app.invoke('nav.open', { url: base + '/other' });
      await app.pageWith(/\/other$/);
      await app.invoke('nav.open', { url: 'native://newtab' });
      const restored = await app.pageWith(/pages\/newtab\.html/);
      assert.ok(restored, 'new tab also loads in an existing view');
      await restored.locator('#recent a').first().waitFor();
    }],
    ['Settings opens from the real browser menu and persists changes', async () => {
      await app.shell.locator('#btn-menu').click();
      await app.shell.getByRole('menuitem', { name: 'Settings', exact: true }).click();
      const page = await app.pageWith(/pages\/settings\.html/);
      assert.ok(page, 'Settings document opened');
      await page.locator('#search_template').waitFor();
      const template = searchTemplate;
      await page.locator('#search_template').fill(template);
      await page.locator('#search_template').dispatchEvent('change');
      const file = path.join(app.data, 'settings.json');
      await waitFor(() => fs.existsSync(file) && JSON.parse(fs.readFileSync(file, 'utf8')).search_template === template);
      await page.reload();
      await page.waitForFunction(expected => document.querySelector('#search_template').value === expected, template);
      await page.locator('#sleep_after_minutes').fill('7');
      await page.locator('#sleep_after_minutes').dispatchEvent('change');
      await waitFor(() => JSON.parse(fs.readFileSync(file, 'utf8')).sleep_after_minutes === 7);
      await app.invoke('settings.open');
      assert.equal(app.ctx.pages().filter(p => /pages\/settings\.html/.test(p.url())).length, 1, 'reuses Settings tab');
    }],
    ['Settings reports a failed save without applying an unpersisted value', async () => {
      const page = await app.pageWith(/pages\/settings\.html/);
      const blocked = path.join(app.data, 'settings.json.tmp');
      fs.mkdirSync(blocked);
      try {
        await page.locator('#search_template').fill(`${base}/not-saved?q=%s`);
        await page.locator('#search_template').dispatchEvent('change');
        await page.getByRole('status').filter({ hasText: 'Could not save setting:' }).waitFor();
        assert.equal(JSON.parse(fs.readFileSync(path.join(app.data, 'settings.json'), 'utf8')).search_template, searchTemplate);
        const result = await app.invoke('omnibox.query', { text: 'save failure probe' });
        assert.ok(result.rows.some(row => row.kind === 'search' && row.url.startsWith(`${base}/search?`)));
      } finally {
        fs.rmdirSync(blocked);
      }
      await page.reload();
      await page.waitForFunction(expected => document.querySelector('#search_template').value === expected, searchTemplate);
    }],
    ['ordinary web pages cannot change settings through page IPC', async () => {
      await app.invoke('nav.open', { url: base + '/listen', disposition: 'new_tab' });
      assert.ok(await app.pageWith(/\/listen$/));
      await app.invoke('nav.open', { url: base + '/live', disposition: 'new_tab' });
      const page = await app.pageWith(/\/live$/);
      await page.waitForSelector('#nm');
      await page.evaluate(() => window.ipc.postMessage(JSON.stringify({
        ai: 'settings', key: 'search_template', value: 'https://untrusted.example/?q=%s'
      })));
      await sleep(200);
      assert.equal(JSON.parse(fs.readFileSync(path.join(app.data, 'settings.json'), 'utf8')).search_template,
        searchTemplate);
      const spoof = 'http://asset.shell/pages/settings.html#set=' +
        encodeURIComponent('search_template=' + encodeURIComponent('https://untrusted.example/?q=%s'));
      await page.evaluate(url => {
        const link = document.createElement('a');
        link.href = url;
        link.textContent = 'Settings link';
        document.body.appendChild(link);
      }, spoof);
      await page.getByRole('link', { name: 'Settings link', exact: true }).click();
      await page.locator('#search_template').waitFor();
      assert.equal(JSON.parse(fs.readFileSync(path.join(app.data, 'settings.json'), 'utf8')).search_template,
        searchTemplate, 'following a settings URL does not apply fragment-based writes');
      await app.invoke('nav.open', { url: base + '/live' });
      await page.locator('#nm').waitFor();
    }],
    ['visited URLs autocomplete in URL mode and complete inline without losing typed characters', async () => {
      const current = await app.pageWith(/\/live$/);
      assert.ok(current);
      await current.evaluate(() => { window.__selectionMarker = true; });
      const prefix = base + '/li';
      const result = await app.invoke('omnibox.query', { text: prefix });
      const completion = result.completion;
      assert.ok([base + '/live', base + '/listen'].includes(completion));
      const alternative = completion === base + '/live' ? base + '/listen' : base + '/live';
      assert.ok(result.rows.some(row => row.kind === 'history' && row.url === base + '/live'));
      const input = app.shell.locator('#omnibox');
      await input.focus();
      await input.fill('');
      const started = performance.now();
      await input.pressSequentially(prefix, { delay: 2 });
      assert.ok(performance.now() - started < 2000, 'address entry stays responsive while history is queried');
      await app.shell.waitForFunction(expected => {
        const input = document.querySelector('#omnibox');
        return input.value === expected && input.selectionStart < input.selectionEnd;
      }, completion);
      await input.press('Backspace');
      await sleep(150);
      assert.equal(await input.inputValue(), prefix, 'deletion does not immediately reinsert the suggestion');
      await input.fill('');
      await input.pressSequentially(prefix, { delay: 2 });
      await app.shell.waitForFunction(expected => document.querySelector('#omnibox').value === expected, completion);
      for (let n = 0; n < 10; n++) {
        await input.press('ArrowDown');
        if ((await app.shell.locator('#omnibox-dropdown [aria-selected="true"]').innerText()).includes(alternative)) break;
      }
      assert.ok((await app.shell.locator('#omnibox-dropdown [aria-selected="true"]').innerText()).includes(alternative));
      await input.press('Enter');
      await waitFor(async () => current.url() === alternative && !await current.evaluate(() => window.__selectionMarker));
      await input.focus();
      await input.fill('a fresh search query');
      await input.press('Enter');
      const search = await app.pageWith(/\/search\?q=/);
      assert.ok(search, 'submitted search reaches the configured local search engine');
      const url = new URL(search.url());
      assert.equal(url.searchParams.get('q'), 'a fresh search query');
      assert.ok(search.url().endsWith('&source=a+b'), 'search template punctuation is preserved');
    }],
    ['every canvas cell zooms and returns through toolbar and command palette', async () => {
      const urls = Array.from({ length: 6 }, (_, index) => `${base}/?cell=${index}`);
      await app.invoke('canvas.create', { cols: 3, rows: 2, urls });
      await waitFor(() => urls.every(url => app.ctx.pages().some(page => page.url() === url)));
      for (let cell = 0; cell < 6; cell++) {
        await app.invoke('canvas.focus', { cell });
        await app.shell.locator('#cv-zoom').click();
        const zoomed = await waitFor(async () => {
          const state = await app.invoke('canvas.state');
          return state.zoomed && state.focus === cell ? state : null;
        });
        assert.deepEqual(zoomed.cells.filter(c => !c.hidden).map(c => c.index), [cell]);
        const page = app.ctx.pages().find(page => page.url() === urls[cell]);
        await waitFor(async () => Math.abs(await page.evaluate(() => innerWidth) - zoomed.cells[cell].body.w) <= 2);
        await app.invoke('shell.open_palette');
        const overlay = await app.pageWith(/overlay\.html/);
        await overlay.locator('#pal-input').fill('> canvas-zoom');
        await overlay.getByRole('option').filter({ hasText: 'canvas-zoom' }).first().waitFor();
        await overlay.locator('#pal-input').press('Enter');
        await waitFor(async () => !(await app.invoke('canvas.state')).zoomed);
        const restored = await app.invoke('canvas.state');
        assert.equal(restored.focus, cell);
        assert.equal(restored.cells.filter(c => !c.hidden).length, 6);
      }
    }],
    ['warm tabs preserve edited forms and JavaScript state', async () => {
      await app.invoke('nav.open', { url: base + '/live?warm=1', disposition: 'new_tab' });
      const page = await app.pageWith(/warm=1/);
      await page.locator('#nm').fill('Keep this unsaved value');
      await page.evaluate(() => { window.__warmMarker = 123; });
      const original = (await app.invoke('agent.context')).page.tab_id;
      for (let n = 0; n < 5; n++) await app.invoke('nav.open', { url: `${base}/other?warm=${n}`, disposition: 'new_tab' });
      await app.invoke('tab.select', { tab_id: original });
      assert.equal(await page.locator('#nm').inputValue(), 'Keep this unsaved value');
      assert.equal(await page.evaluate(() => window.__warmMarker), 123);
    }],
    ['find counts literal matches, navigates them, and supports whole-word matching', async () => {
      await app.invoke('nav.open', { url: base + '/?find=1', disposition: 'new_tab' });
      const page = await app.pageWith(/find=1/);
      await page.waitForSelector('#p1');
      await page.setContent('<p>Alpha alpha alphabet <b>alpha</b></p>');
      await app.shell.evaluate(() => {
        window.__findResults = [];
        window.__native.on('find.result', result => window.__findResults.push(result));
      });
      await app.shell.locator('#btn-menu').click();
      await app.shell.getByRole('menuitem', { name: 'Find in page', exact: true }).click();
      await app.shell.locator('#find-input').fill('alpha');
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.total === 4);
      await app.invoke('find.next', { query: 'alpha' });
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.active === 2);
      await Promise.all([
        app.invoke('find.next', { query: 'alpha' }),
        app.invoke('find.next', { query: 'alpha' }),
      ]);
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.active === 4);
      await app.shell.locator('#find-word').click();
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.total === 3);
      await app.shell.locator('#find-case').click();
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.total === 2);
      await app.invoke('shortcut', { name: 'toggle_highlight_all' });
      await page.waitForFunction(() => CSS.highlights.get('browser-find')?.size === 2);
      await app.invoke('shortcut', { name: 'toggle_highlight_all' });
      await page.waitForFunction(() => !CSS.highlights.has('browser-find'));
      await app.shell.locator('#find-close').click();
      await app.shell.locator('#find-bar').waitFor({ state: 'hidden' });
    }],
  ]);
} finally {
  await app.close();
  fixtures.close();
}

const model = await startModel((_messages, _body, n) => ({ text: `RECOVERED_RESULT_${n}`, delay: 700 }));
const ai = await launch({ modelPort: model.port, debugPort: await debugPort() });
try {
  failed += await run([
    ['AI completion survives closing and reloading the sidebar', async () => {
      await ai.invoke('shortcut', { name: 'ask_ai' });
      let pane = await ai.pageWith(/sidebar/);
      await pane.waitForFunction(() => !!window.__agentUi);
      await pane.evaluate(() => window.__agentUi.send('Give a brief greeting.'));
      await waitFor(() => model.calls.length > 0);
      await ai.invoke('shortcut', { name: 'ask_ai' });
      await sleep(1600);
      await ai.invoke('shortcut', { name: 'ask_ai' });
      await pane.reload();
      await pane.waitForFunction(() => document.body.textContent.includes('RECOVERED_RESULT_1'));
      const state = await ai.invoke('ai.session');
      assert.ok(state.turns.some(turn => turn.status === 'done'));
      assert.equal(await pane.getByRole('button', { name: 'Stop', exact: true }).count(), 0);
    }],
  ]);
} finally {
  await ai.close();
  model.close();
}
process.exitCode = failed ? 1 : 0;
