import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { launch, listen, run, sleep, startFixtures, startModel } from './harness.mjs';

const waitFor = async (fn, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try { const result = await fn(); if (result) return result; }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    await sleep(75);
  }
  throw new Error('timed out waiting for condition');
};
const primary = await startModel((_messages, body) => ({ text: `ANSWER: PRIMARY_${body.model}`, delay: 300 }));
const secondary = await startModel((_messages, body) => ({ text: `ANSWER: SECONDARY_${body.model}`, delay: 300 }));
const fixtures = await startFixtures();
const downloadName = `river-settings-test-${process.pid}-${Date.now()}.txt`;
const attachment = await listen((_req, res) => {
  res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-disposition': `attachment; filename="${downloadName}"` });
  res.end('settings download fixture');
});
const reservation = await listen((_req, res) => res.end());
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ modelPort: primary.port, debugPort: reservation.port });
const config = () => JSON.parse(fs.readFileSync(path.join(app.data, 'ai.json'), 'utf8'));
let side, settings, workingId, backupId;
const openSettings = async (section = 'model') => {
  await app.invoke('shell_settings', { operation: 'open', section });
  settings = await app.pageWith(/pages\/settings\.html/);
  await settings.getByRole('tab', { name: section === 'model' ? 'Models' : 'General', exact: true }).waitFor();
};
const back = () => app.invoke('shell_tab', { operation: 'activate', tab_id: workingId });
const command = async text => {
  const count = primary.calls.length + secondary.calls.length;
  const turns = (await app.invoke('shell_ai', { operation: 'session_get' })).turns.length;
  await side.locator('textarea').fill(text);
  await side.locator('#ag-send, #ask-send').click();
  await sleep(180);
  assert.equal(primary.calls.length + secondary.calls.length, count, 'local command never calls a model');
  assert.equal((await app.invoke('shell_ai', { operation: 'session_get' })).turns.length, turns, 'local command adds no conversation turn');
};
const selected = () => side.evaluate(() => localStorage.getItem('model'));
const send = async (question, marker) => {
  const sequence = (await app.invoke('shell_ai', { operation: 'session_get' })).sequence;
  await side.locator('textarea').fill(question);
  await side.locator('#ag-send, #ask-send').click();
  await waitFor(async () => {
    const snapshot = await app.invoke('shell_ai', { operation: 'session_get' });
    return snapshot.sequence > sequence && snapshot.turns.every(turn => !['running', 'pending'].includes(turn.status));
  });
  if (marker) await side.waitForFunction(text => document.body.textContent.includes(text), marker);
};
let failed = 0;
try {
  await app.shell.waitForFunction(() => !!window.__native);
  await app.invoke('shell_page_navigate', { url: `http://127.0.0.1:${fixtures.port}/` });
  await app.pageWith(/127\.0\.0\.1:\d+\/$/);
  workingId = (await app.invoke('shell_agent', { operation: 'context_get' })).page.tab_id;
  failed += await run([
    ['saved chats reopen in River even before its pane has been created', async () => {
      await app.invoke('shell_settings', { operation: 'open', section: 'chats' });
      settings = await app.pageWith(/pages\/settings\.html/);
      await settings.getByRole('heading', { name: 'Saved chats', exact: true }).waitFor();
      assert.equal(app.ctx.pages().some(page => /sidebar\.html/.test(page.url())), false);
      await settings.evaluate(() => new Promise((resolve, reject) => {
        const req = 777;
        const stop = window.__native.on('agent.reply', result => {
          if (result.req !== req) return;
          stop();
          result.ok ? resolve(result.value) : reject(new Error(result.error));
        });
        window.__native.invoke('shell_conversation', { operation: 'save', req, id: 'unified-fixture', doc: {
          title: 'Reopen fixture chat', updated: Date.now(),
          messages: [{ role: 'user', content: 'Saved question' }, { role: 'assistant', content: 'Saved answer fixture' }],
        } }).catch(reject);
      }));
      await settings.getByRole('tab', { name: 'General', exact: true }).click();
      await settings.getByRole('tab', { name: 'Saved chats', exact: true }).click();
      await settings.getByRole('button', { name: /Reopen fixture chat/ }).click();
      side = await app.pageWith(/sidebar\.html/);
      await side.waitForFunction(() => window.__agentUi?.messages.some(message => message.content === 'Saved answer fixture'));
      assert.equal(primary.calls.length + secondary.calls.length, 0);
      await back();
    }],
    ['sample model needs only a name and endpoint, without a key, ID or route', async () => {
      await openSettings();
      const routes = config().routes;
      const calls = primary.calls.length + secondary.calls.length;
      await settings.getByRole('button', { name: 'Add model', exact: true }).click();
      await settings.getByLabel('Model name', { exact: true }).fill('sample-model');
      await settings.getByLabel('Endpoint', { exact: true }).fill('http://sample-model.invalid/v1/chat/completions');
      await settings.getByRole('button', { name: 'Save model', exact: true }).click();
      await waitFor(() => config().providers.length === 2);
      const sample = config().providers[1];
      assert.equal(sample.base_url, 'http://sample-model.invalid/v1');
      assert.ok(!sample.api_key_env);
      assert.deepEqual(config().routes, routes, 'no additional routes');
      assert.equal(primary.calls.length + secondary.calls.length, calls, 'configuration does not execute a model');
      await waitFor(async () => {
        await app.invoke('shell_ai', { operation: 'models_list' });
        const status = await app.invoke('shell_ai', { operation: 'status_get' });
        return status.models.some(entry => entry.manifest.model_id === 'sample-model' && entry.available);
      });
      await app.invoke('shell_ai', { operation: 'provider_delete', id: sample.id });
      await settings.reload();
      await settings.getByRole('button', { name: 'Add model', exact: true }).waitFor();
    }],
    ['multiple models on one endpoint retain existing metadata and saved credentials', async () => {
      await settings.getByRole('button', { name: 'Edit', exact: true }).click();
      await settings.getByLabel('API key', { exact: true }).fill('primary-key');
      await settings.locator('summary').filter({ hasText: 'Advanced' }).click();
      await settings.getByRole('button', { name: 'Add another model on this endpoint', exact: true }).click();
      await settings.getByLabel('Model name', { exact: true }).nth(1).fill('org/second-model');
      await settings.getByLabel('Display name', { exact: true }).nth(1).fill('Second model');
      await settings.getByLabel('Context window', { exact: true }).nth(1).fill('65536');
      await settings.getByRole('button', { name: 'Save model', exact: true }).click();
      await waitFor(() => config().providers[0].models.length === 2);
      assert.equal(config().providers[0].models[1].context_window, 65536);
      assert.ok(!JSON.stringify(config()).includes('primary-key'));
      await settings.getByRole('button', { name: 'Add model', exact: true }).click();
      await settings.getByLabel('Model name', { exact: true }).fill('fake-model');
      await settings.getByLabel('Endpoint', { exact: true }).fill(`http://127.0.0.1:${secondary.port}/v1`);
      await settings.getByRole('button', { name: 'Save model', exact: true }).click();
      await waitFor(() => config().providers.length === 2);
      backupId = config().providers[1].id;
      await back();
    }],
    ['/model picker and qualified commands switch real Agent calls without losing chat', async () => {
      await command('/model');
      await side.getByRole('listbox', { name: 'Models', exact: true }).waitFor();
      await side.getByLabel('Search models', { exact: true }).fill('org/second-model');
      await side.getByLabel('Search models', { exact: true }).press('ArrowDown');
      await side.locator('[data-model="custom/org/second-model"]').press('Enter');
      await send('Answer using the second model.', 'PRIMARY_org/second-model');
      assert.equal(primary.calls.at(-1).auth, 'Bearer primary-key');
      await command('/model fake-model');
      await side.getByRole('listbox').waitFor();
      assert.equal(await selected(), 'custom/org/second-model', 'ambiguous name is not silently selected');
      await side.getByLabel('Search models', { exact: true }).press('Escape');
      await command(`/model ${backupId}/fake-model`);
      await send('Use the backup endpoint.', 'SECONDARY_fake-model');
      assert.equal(secondary.calls.at(-1).auth, undefined, 'keyless endpoint actually works');
      assert.ok(secondary.calls.at(-1).body.messages.some(message => message.role === 'assistant' && message.content.includes('PRIMARY_org/second-model')));
      await command('/model custom/fake-model');
      await send('Return to the first model.', 'PRIMARY_fake-model');
      await side.reload();
      await side.locator('#ag-input').waitFor();
      assert.equal(await selected(), 'custom/fake-model');
    }],
    ['question mode shares /model and configured credentials without extra routes', async () => {
      await side.locator('label.switch').filter({ has: side.locator('#agent-switch') }).click();
      await command(`/model ${backupId}/fake-model`);
      const before = secondary.calls.length;
      await send('A general question using the backup model.');
      await waitFor(() => secondary.calls.length > before);
      assert.equal(secondary.calls.at(-1).body.model, 'fake-model');
      assert.ok(!secondary.calls.at(-1).body.tools);
      await command('/model default');
      assert.equal(await selected(), '');
      await side.locator('label.switch').filter({ has: side.locator('#agent-switch') }).click();
    }],
    ['failed edits leave saved models and credentials intact', async () => {
      await openSettings();
      await settings.getByRole('button', { name: 'Edit', exact: true }).first().click();
      await settings.locator('summary').filter({ hasText: 'Advanced' }).click();
      const before = config();
      await settings.getByLabel('API key', { exact: true }).fill('must-not-save');
      await settings.getByLabel('Model name', { exact: true }).nth(1).fill('fake-model');
      await settings.getByRole('button', { name: 'Save model', exact: true }).click();
      await settings.getByRole('status').filter({ hasText: 'duplicate model ID' }).waitFor();
      assert.deepEqual(config(), before);
      await settings.getByLabel('Model name', { exact: true }).nth(1).fill('org/second-model');
      const blocked = path.join(app.data, 'ai.json.tmp');
      fs.mkdirSync(blocked);
      try {
        await settings.getByRole('button', { name: 'Save model', exact: true }).click();
        await settings.getByRole('status').filter({ hasText: 'os error' }).waitFor();
        assert.deepEqual(config(), before);
      } finally { fs.rmdirSync(blocked); }
      await settings.getByRole('button', { name: 'Cancel', exact: true }).click();
      await back();
      await command('/model custom/fake-model');
      await send('The original credential still works.');
      assert.equal(primary.calls.at(-1).auth, 'Bearer primary-key');
    }],
    ['all settings and library sections reuse a single browser tab and preserve River drafts', async () => {
      await side.locator('textarea').fill('keep this River draft');
      await side.getByRole('button', { name: 'Settings', exact: true }).click();
      await settings.getByRole('tab', { name: 'General', exact: true }).waitFor();
      await settings.locator('#density').waitFor();
      const id = (await app.invoke('shell_agent', { operation: 'context_get' })).page.tab_id;
      for (const section of ['Models', 'Tools', 'Skills', 'MCP', 'Prompts', 'Instructions', 'History', 'Downloads', 'Saved chats']) {
        await settings.getByRole('tab', { name: section, exact: true }).click();
        await settings.getByRole('heading', { name: section, exact: true }).waitFor();
        assert.equal((await app.invoke('shell_agent', { operation: 'context_get' })).page.tab_id, id);
      }
      assert.equal(app.ctx.pages().filter(page => /pages\/settings\.html/.test(page.url())).length, 1);
      assert.equal(await side.locator('textarea').inputValue(), 'keep this River draft');
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'open_history' });
      await settings.getByRole('searchbox', { name: 'Search history', exact: true }).fill('127.0.0.1');
      await settings.locator('.ag-row-main').first().waitFor();
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'open_downloads' });
      await settings.getByRole('heading', { name: 'Downloads', exact: true }).waitFor();
      assert.equal((await app.invoke('shell_agent', { operation: 'context_get' })).page.tab_id, id);
      await settings.reload();
      await settings.getByRole('heading', { name: 'Downloads', exact: true }).waitFor();
      await back();
      const page = await app.pageWith(/127\.0\.0\.1:\d+\/$/);
      await page.evaluate(() => window.ipc?.postMessage(JSON.stringify({ id: 909, cmd: 'shell_ai', payload: { operation: 'provider_delete', id: 'custom' } })));
      await sleep(150);
      assert.ok(config().providers.some(provider => provider.id === 'custom'), 'ordinary page cannot use the Settings bridge');
    }],
    ['History deletion and completed download actions work inside Settings', async () => {
      const historyUrl = `http://127.0.0.1:${fixtures.port}/other?history-delete=1&details=${'long-details-'.repeat(60)}`;
      const historyTitle = 'A very long history title '.repeat(30).trim();
      await app.invoke('shell_page_navigate', { url: historyUrl });
      const historyPage = await app.pageWith(/history-delete=1/);
      await historyPage.waitForSelector('body');
      await historyPage.evaluate(title => { document.title = title; }, historyTitle);
      await waitFor(async () => (await app.invoke('shell_history', { operation: 'query', tokens: 'history-delete' })).rows.some(row => row.title === historyTitle));
      await app.invoke('shell_settings', { operation: 'open', section: 'history' });
      await settings.getByRole('searchbox', { name: 'Search history', exact: true }).fill('history-delete');
      const historyEntry = settings.locator('.browser-history-entry .ag-row-main').first();
      await historyEntry.waitFor();
      assert.equal(await historyEntry.textContent(), historyTitle);
      assert.equal(await historyEntry.locator('small').count(), 0, 'full URLs are not expanded in the list');
      assert.equal(await historyEntry.evaluate(button => {
        const title = button.querySelector('b');
        return getComputedStyle(title).textOverflow === 'ellipsis' && title.scrollWidth > title.clientWidth
          && button.closest('.ag-row').getBoundingClientRect().height < 60;
      }), true, 'long titles remain compact');
      await historyEntry.click();
      await waitFor(async () => (await app.invoke('shell_agent', { operation: 'context_get' })).page.url === historyUrl);
      await app.invoke('shell_settings', { operation: 'open', section: 'history' });
      await settings.getByRole('button', { name: 'Delete', exact: true }).click();
      await settings.getByText('No matching history.', { exact: true }).waitFor();
      const cdp = await app.browser.newBrowserCDPSession();
      await cdp.send('Browser.setDownloadBehavior', { behavior: 'default' });
      await back();
      await app.invoke('shell_page_navigate', { url: `http://127.0.0.1:${attachment.port}/file` });
      await app.invoke('shell_ui', { operation: 'shortcut', name: 'open_downloads' });
      const download = await waitFor(async () => (await app.invoke('shell_settings', { operation: 'state_get' })).downloads.find(download => download.state === 'completed'));
      try {
        await settings.getByText(download.file, { exact: true }).waitFor();
        assert.equal(path.basename(download.path), downloadName);
        assert.equal(fs.readFileSync(download.path, 'utf8'), 'settings download fixture');
        await settings.getByRole('button', { name: 'Remove from list', exact: true }).click();
        await settings.getByText('No downloads yet.', { exact: true }).waitFor();
        assert.ok(fs.existsSync(download.path), 'removing a list entry does not delete the file');
      } finally {
        if (path.basename(download.path) === downloadName && fs.existsSync(download.path)
          && fs.readFileSync(download.path, 'utf8') === 'settings download fixture') fs.unlinkSync(download.path);
        await cdp.detach();
      }
      await back();
    }],
    ['deleted model credentials do not resurrect through legacy configuration', async () => {
      fs.writeFileSync(path.join(app.data, 'ai.key'), 'legacy-key');
      await app.invoke('shell_ai', { operation: 'provider_delete', id: 'custom' });
      assert.equal(fs.existsSync(path.join(app.data, 'ai.key')), false);
      const status = await app.invoke('shell_ai', { operation: 'provider_set', base_url: `http://127.0.0.1:${primary.port}/v1`, model_id: 'fake-model' });
      assert.equal(status.provider.connections.find(connection => connection.id === 'custom').key_saved, false);
    }],
  ]);
} finally {
  await app.invoke('shell_agent', { operation: 'stop' }).catch(() => undefined);
  for (const id of config().providers.map(provider => provider.id)) await app.invoke('shell_ai', { operation: 'provider_delete', id }).catch(() => undefined);
  await app.close();
  primary.close();
  secondary.close();
  fixtures.close();
  attachment.server.close();
}
process.exitCode = failed ? 1 : 0;
