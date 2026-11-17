import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { launch, listen, startFixtures, startModel, run, sleep, nativeHitBounds } from './harness.mjs';

const waitFor = async (fn, ms = 15000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { const result = await fn(); if (result) return result; }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    await sleep(75);
  }
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
    ['Settings opens directly from its toolbar button and persists changes', async () => {
      assert.equal(await app.shell.locator('#btn-menu').count(), 0);
      assert.equal(await app.shell.locator('#btn-settings').getAttribute('aria-haspopup'), null);
      await app.shell.getByRole('button', { name: 'Settings', exact: true }).click();
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
    ['Settings stays accessible with River open on another working tab', async () => {
      const settings = await app.pageWith(/pages\/settings\.html/);
      const settingsId = (await app.invoke('agent.context')).page.tab_id;
      await app.invoke('nav.open', { url: base + '/live?settings=river', disposition: 'new_tab' });
      const working = await app.pageWith(/settings=river/);
      const workingId = (await app.invoke('agent.context')).page.tab_id;
      await working.locator('#nm').fill('Unsaved work beside River');
      await app.shell.evaluate(() => {
        window.__settingsPanel = '';
        window.__native.on('shell.panel', p => { window.__settingsPanel = p.panel; });
      });
      await app.invoke('shortcut', { name: 'ask_ai' });
      const pane = await app.pageWith(/sidebar/);
      try {
        await pane.waitForFunction(() => !!window.__agentUi);
        const back = pane.getByRole('button', { name: 'Back', exact: true });
        if (await back.count()) await back.click();
        await pane.locator('textarea').fill('Keep this River draft');
        const hit = await nativeHitBounds(app.shell, '#btn-settings', app.child.pid);
        assert.deepEqual([hit.x, hit.y, hit.w, hit.h], [0, 0, hit.clientW, hit.clientH], 'native input reaches the full-window shell, not River');
        await app.shell.getByRole('button', { name: 'Settings', exact: true }).click();
        await waitFor(async () => (await app.invoke('agent.context')).page.tab_id === settingsId, 5000);
        assert.equal(await app.shell.evaluate(() => window.__settingsPanel), 'ai', 'River remains open');
        assert.equal((await app.invoke('agent.context')).page.tab_id, settingsId, 'reuses the existing Settings tab');
        assert.equal(await pane.locator('textarea').inputValue(), 'Keep this River draft');
        await settings.locator('#density').focus();
        const focused = await settings.evaluate(() => document.hasFocus());
        await settings.locator('#density').selectOption('compact');
        await waitFor(() => JSON.parse(fs.readFileSync(path.join(app.data, 'settings.json'), 'utf8')).density === 'compact');
        assert.equal(await settings.evaluate(() => document.hasFocus()), focused, 'layout changes preserve Settings focus');
        await settings.locator('#density').selectOption('comfortable');
        await waitFor(() => JSON.parse(fs.readFileSync(path.join(app.data, 'settings.json'), 'utf8')).density === 'comfortable');
        await settings.reload();
        await settings.locator('#density').waitFor();
        assert.equal(await settings.locator('#density').inputValue(), 'comfortable');
        await app.invoke('tab.select', { tab_id: workingId });
        assert.equal(await working.locator('#nm').inputValue(), 'Unsaved work beside River');
        assert.equal(await pane.locator('textarea').inputValue(), 'Keep this River draft');
      } finally {
        await app.invoke('shortcut', { name: 'ask_ai' });
        await app.invoke('tab.select', { tab_id: settingsId });
      }
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
      const spoof = 'https://asset.shell/pages/settings.html#set=' +
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
    ['plain toolbar find searches live without another row or advanced controls', async () => {
      await app.invoke('nav.open', { url: base + '/?find=1', disposition: 'new_tab' });
      const page = await app.pageWith(/find=1/);
      await page.waitForSelector('#p1');
      await page.setContent('<p>Alpha alpha alphabet <b>alpha</b></p>');
      await app.shell.evaluate(() => {
        window.__findResults = [];
        window.__native.on('find.result', result => window.__findResults.push(result));
      });
      assert.deepEqual(await app.shell.evaluate(() => {
        const input = document.querySelector('#find-input');
        return [input.previousElementSibling.id, input.nextElementSibling.id];
      }), ['btn-ai', 'btn-downloads']);
      assert.equal(await app.shell.locator('#find-bar, #find-case, #find-word, #find-all, #find-next, #find-prev').count(), 0);
      const bounds = await nativeHitBounds(app.shell, '#find-input', app.child.pid);
      assert.deepEqual([bounds.x, bounds.y, bounds.w, bounds.h], [0, 0, bounds.clientW, bounds.clientH]);
      const before = await page.evaluate(() => ({ height: innerHeight, width: innerWidth }));
      await app.invoke('shortcut', { name: 'find' });
      await app.shell.waitForFunction(() => document.activeElement?.id === 'find-input');
      await app.shell.locator('#find-input').fill('alpha');
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.total === 4);
      await page.waitForFunction(() => CSS.highlights.get('browser-find')?.size === 4);
      await app.shell.locator('#find-input').press('Enter');
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.active === 2);
      await app.shell.locator('#find-input').press('Shift+Enter');
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.active === 1);
      await app.shell.locator('#find-input').fill('[not regex]');
      await app.shell.waitForFunction(() => window.__findResults.at(-1)?.query === '[not regex]' && window.__findResults.at(-1)?.total === 0);
      assert.equal(await app.shell.locator('#find-input').getAttribute('aria-invalid'), 'true');
      await app.shell.locator('#find-input').press('Escape');
      await page.waitForFunction(() => !CSS.highlights.has('browser-find'));
      assert.equal(await app.shell.locator('#find-input').inputValue(), '');
      assert.equal(await app.shell.locator('#find-input').isVisible(), true);
      assert.deepEqual(await page.evaluate(() => ({ height: innerHeight, width: innerWidth })), before);
    }],
    ['inline find keeps River open and follows the active page', async () => {
      const firstId = (await app.invoke('agent.context')).page.tab_id;
      const first = await app.pageWith(/find=1/);
      await app.invoke('shortcut', { name: 'ask_ai' });
      const river = await app.pageWith(/sidebar/);
      await river.locator('textarea').fill('Preserve this search draft');
      const before = await first.evaluate(() => ({ height: innerHeight, width: innerWidth }));
      await app.invoke('shortcut', { name: 'find' });
      await app.shell.locator('#find-input').fill('alpha');
      await first.waitForFunction(() => CSS.highlights.get('browser-find')?.size === 4);
      assert.deepEqual(await first.evaluate(() => ({ height: innerHeight, width: innerWidth })), before, 'find does not replace River or shift the page');
      assert.equal(await river.locator('textarea').inputValue(), 'Preserve this search draft');
      await app.invoke('nav.open', { url: base + '/?find=2', disposition: 'new_tab' });
      const second = await app.pageWith(/find=2/);
      await second.waitForSelector('#p1');
      await app.shell.waitForFunction(id => {
        const result = window.__findResults.at(-1);
        return result?.query === 'alpha' && result.tab_id !== id && result.total === 0;
      }, firstId).catch(async error => {
        const results = await app.shell.evaluate(() => window.__findResults.slice(-6));
        throw new Error(`${error.message}; recent find results: ${JSON.stringify(results)}`);
      });
      await app.invoke('tab.select', { tab_id: firstId });
      const restored = await app.pageWith(/find=1/);
      await app.shell.waitForFunction(id => window.__findResults.at(-1)?.tab_id === id, firstId);
      const matches = await restored.evaluate(() => (document.body.innerText.match(/alpha/gi) || []).length);
      assert.equal(await app.shell.evaluate(() => window.__findResults.at(-1).total), matches);
      await app.shell.locator('#find-input').fill('');
      await restored.waitForFunction(() => !CSS.highlights.has('browser-find-active'));
      await app.invoke('shortcut', { name: 'ask_ai' });
    }],
  ]);
} finally {
  await app.close();
  fixtures.close();
}

const noteExample = '广喜，念<sup>注1</sup>鬼，曰："此教我先威<sup>注2</sup>众耳。"';
const replyFixtures = await startFixtures();
const replyUrl = `http://127.0.0.1:${replyFixtures.port}/?markdown-link=1&source=river`;
const model = await startModel((_messages, _body, n) => ({
  text: `RECOVERED_RESULT_${n}\n\n${noteExample}\n\n[**打开测试页面**][page]\n\n[page]: ${replyUrl}`,
  delay: 700,
}));
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
    ['River renders superscript notes from model replies as raised text', async () => {
      const pane = await ai.pageWith(/sidebar/);
      assert.deepEqual(await pane.locator('.rich sup').allTextContents(), ['注1', '注2']);
      assert.equal(await pane.locator('.rich').evaluateAll(nodes => nodes.some(node => node.textContent.includes('<sup>'))), false);
      assert.equal(await pane.locator('.rich sup').first().evaluate(node => {
        const style = getComputedStyle(node);
        return style.verticalAlign === 'super' && parseFloat(style.fontSize) < parseFloat(getComputedStyle(node.parentElement).fontSize);
      }), true);
    }],
    ['a rendered reference link opens a browser tab without navigating River', async () => {
      const pane = await ai.pageWith(/sidebar/);
      const paneUrl = pane.url();
      const link = pane.getByRole('link', { name: '打开测试页面', exact: true });
      assert.equal(await link.getAttribute('href'), replyUrl);
      await link.locator('strong').click();
      const destination = await ai.pageWith(/markdown-link=1/);
      await destination.waitForSelector('#p1');
      assert.equal((await ai.invoke('agent.context')).page.url, replyUrl);
      assert.equal(pane.url(), paneUrl);
      assert.equal(await link.count(), 1);
    }],
  ]);
} finally {
  await ai.close();
  model.close();
  replyFixtures.close();
}
process.exitCode = failed ? 1 : 0;
