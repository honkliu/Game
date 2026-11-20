// Chat Application-storage tools run against real WebView2 pages and a scripted model.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { launch, listen, run, sleep } from './harness.mjs';

const waitFor = async (fn, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const result = await fn(); if (result) return result; await sleep(75); }
  throw new Error('timed out waiting for storage test');
};
const fixtures = await listen((_req, res) => {
  res.writeHead(200, { 'content-type': 'text/html', 'set-cookie': 'member_cookie=original; HttpOnly; SameSite=Lax; Path=/' });
  res.end('<!doctype html><h1>Application storage fixture</h1><p>Member records belong to this page.</p>');
});
let plan = [];
let results = [];
let advertised = [];
const model = await listen((req, res) => {
  let raw = '';
  req.on('data', chunk => { raw += chunk; });
  req.on('end', () => {
    if (req.url.endsWith('/models')) return res.end(JSON.stringify({ data: [{ id: 'fake-model' }] }));
    const body = JSON.parse(raw);
    advertised = body.tools?.map(tool => tool.function.name) || [];
    const lastUser = body.messages.findLastIndex(message => message.role === 'user');
    results = body.messages.slice(lastUser + 1).filter(message => message.role === 'tool').map(message => {
      try { return JSON.parse(message.content); } catch { return message.content; }
    });
    const step = plan[results.length];
    const delta = step ? { tool_calls: [{ index: 0, id: `storage_${Date.now()}_${results.length}`, type: 'function',
      function: { name: step[0], arguments: JSON.stringify(typeof step[1] === 'function' ? step[1](results) : step[1]) } }] } :
      { content: 'STORAGE-TEST-DONE' };
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    res.write(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: null }] })}\n\n`);
    res.write(`data: ${JSON.stringify({ choices: [{ delta: {}, finish_reason: step ? 'tool_calls' : 'stop' }] })}\n\n`);
    res.end('data: [DONE]\n\n');
  });
});
const reservation = await listen((_req, res) => res.end());
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ modelPort: model.port, debugPort: reservation.port });
const base = `http://127.0.0.1:${fixtures.port}`;
let side;
const target = result => ({ tab_id: result.tab_id, document_token: result.document_token });
const start = async steps => {
  plan = steps;
  results = [];
  const before = await side.evaluate(() => window.__agentUi.messages.length);
  await side.evaluate(() => window.__agentUi.send('Run the next Application storage test.'));
  return async () => {
    await waitFor(() => side.evaluate(before => {
      const messages = window.__agentUi.messages;
      return messages.length >= before + 2 && messages.at(-1)?.content === 'STORAGE-TEST-DONE';
    }, before));
    return results;
  };
};
const ask = async steps => (await start(steps))();
const allow = () => side.getByRole('button', { name: 'Allow', exact: true }).last().click();
const pressF12 = pid => {
  // Target the focused native WebView, not the top-level tao window. This also
  // exercises WebView2's accelerator path when the desktop is not foreground.
  const command = `
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class StorageKeys {
  [StructLayout(LayoutKind.Sequential)] public struct GuiInfo {
    public uint size, flags;
    public IntPtr active, focus, capture, menu, move, caret;
    public int left, top, right, bottom;
  }
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
  [DllImport("user32.dll")] public static extern bool IsChild(IntPtr parent, IntPtr child);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint message, IntPtr w, IntPtr l);
}
'@
$window = (Get-Process -Id ${pid}).MainWindowHandle
$owner = 0
$thread = [StorageKeys]::GetWindowThreadProcessId($window, [ref]$owner)
$info = New-Object StorageKeys+GuiInfo
$info.size = [Runtime.InteropServices.Marshal]::SizeOf($info)
if (![StorageKeys]::GetGUIThreadInfo($thread, [ref]$info) -or ![StorageKeys]::IsChild($window, $info.focus)) {
  throw 'No focused native child of the test browser'
}
if (![StorageKeys]::PostMessageW($info.focus, 256, [IntPtr]123, [IntPtr]0x580001)) { throw 'F12 keydown failed' }
if (![StorageKeys]::PostMessageW($info.focus, 257, [IntPtr]123, [IntPtr](-1067974655))) { throw 'F12 keyup failed' }
`;
  execFileSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', command], { stdio: 'pipe', timeout: 10000 });
};
let failed = 0;
try {
  await app.invoke('shell_canvas', { operation: 'open', cols: 2, rows: 1, urls: [`${base}/left`, `${base}/right`] });
  const left = await app.pageWith(/\/left$/);
  const right = await app.pageWith(/\/right$/);
  await Promise.all([left.waitForSelector('h1'), right.waitForSelector('h1')]);
  for (const [page, value] of [[left, 'left'], [right, 'right']]) {
    await page.evaluate(async value => {
      sessionStorage.setItem('member', JSON.stringify({ displayName: value }));
      localStorage.setItem('member', 'original');
      const db = await new Promise((resolve, reject) => {
        const open = indexedDB.open('members', 1);
        open.onupgradeneeded = () => {
          open.result.createObjectStore('records', { keyPath: 'id' });
          open.result.createObjectStore('keyed');
        };
        open.onsuccess = () => resolve(open.result);
        open.onerror = () => reject(open.error);
      });
      await new Promise((resolve, reject) => {
        const tx = db.transaction(['records', 'keyed'], 'readwrite');
        tx.objectStore('records').put({ id: 'member', displayName: 'Original' });
        tx.objectStore('keyed').put({ displayName: 'Keyed original' }, 'member');
        tx.oncomplete = resolve;
        tx.onerror = () => reject(tx.error);
      });
      db.close();
    }, value);
  }
  await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 0 });
  await app.invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' });
  side = await app.pageWith(/sidebar\.html/);
  await side.waitForSelector('#ag-input');
  failed += await run([
    ['cookie and storage tools are enabled and read the focused cell, including HttpOnly cookies', async () => {
      const values = await ask([
        ['shell_cookie', { operation: 'list', name: 'member_cookie' }],
        ['shell_storage', { operation: 'read', storage: 'session', key: 'member' }],
        ['shell_storage', { operation: 'read', storage: 'session', row: 0, col: 1, key: 'member' }],
      ]);
      for (const name of ['shell_cookie', 'shell_storage']) assert.ok(advertised.includes(name));
      assert.equal(values[0].cookies[0].value, 'original');
      assert.equal(values[0].cookies[0].httpOnly, true);
      assert.equal(await left.evaluate(() => document.cookie.includes('member_cookie')), false);
      assert.equal(JSON.parse(values[1].entries[0].value).displayName, 'left');
      assert.equal(JSON.parse(values[2].entries[0].value).displayName, 'right');
      assert.notEqual(values[1].tab_id, values[2].tab_id);
    }],
    ['a read tool cannot perform a write, and missing keys return an empty result', async () => {
      const values = await ask([
        ['shell_storage', { operation: 'read', storage: 'local', key: 'member', action: 'set', value: 'injected' }],
        ['shell_storage', { operation: 'read', storage: 'local', key: 'missing' }],
      ]);
      assert.equal(values[0].entries[0].value, 'original');
      assert.deepEqual(values[1].entries, []);
      assert.equal(await left.evaluate(() => localStorage.getItem('member')), 'original');
    }],
    ['invalid explicit targets fail instead of falling back to the focused cell', async () => {
      const values = await ask([
        ['shell_storage', { operation: 'read', storage: 'session', row: 9007199254740991, col: 0 }],
        ['shell_storage', { operation: 'read', storage: 'session', row: 0 }],
        ['shell_storage', { operation: 'read', storage: 'session', tab_id: 999999, cell: 0 }],
        ['shell_cookie', { operation: 'list', tab_id: 'invalid' }],
      ]);
      assert.match(values[0], /invalid cell row/);
      assert.match(values[1], /invalid cell column/);
      assert.match(values[2], /does not exist/);
      assert.match(values[3], /invalid type for `tab_id`/);
    }],
    ['approved storage writes remain pinned to the original cell after focus changes', async () => {
      const done = await start([
        ['shell_storage', { operation: 'read', storage: 'session', key: 'member' }],
        ['shell_storage', values => ({ ...target(values[0]), storage: 'session', operation: 'set', key: 'member', value: '{"displayName":"Alice"}' })],
      ]);
      await side.getByRole('button', { name: 'Allow', exact: true }).last().waitFor();
      assert.equal(JSON.parse(await left.evaluate(() => sessionStorage.getItem('member'))).displayName, 'left');
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 1 });
      await allow();
      const values = await done();
      assert.equal(values[1].verified, true);
      assert.equal(JSON.parse(await left.evaluate(() => sessionStorage.getItem('member'))).displayName, 'Alice');
      assert.equal(JSON.parse(await right.evaluate(() => sessionStorage.getItem('member'))).displayName, 'right');
      await app.invoke('shell_canvas', { operation: 'cell_focus', cell: 0 });
    }],
    ['denied writes do nothing and navigation invalidates approved document tokens', async () => {
      let done = await start([
        ['shell_storage', { operation: 'read', storage: 'local', key: 'member' }],
        ['shell_storage', values => ({ ...target(values[0]), storage: 'local', operation: 'set', key: 'member', value: 'denied' })],
      ]);
      await side.getByRole('button', { name: 'Deny', exact: true }).last().click();
      assert.match((await done())[1], /rejected/i);
      assert.equal(await left.evaluate(() => localStorage.getItem('member')), 'original');
      done = await start([
        ['shell_storage', { operation: 'read', storage: 'local', key: 'member' }],
        ['shell_storage', values => ({ ...target(values[0]), storage: 'local', operation: 'set', key: 'member', value: 'stale' })],
      ]);
      await side.getByRole('button', { name: 'Allow', exact: true }).last().waitFor();
      await left.reload();
      await left.waitForSelector('h1');
      await allow();
      assert.match((await done())[1], /target changed|not read/i);
      assert.equal(await left.evaluate(() => localStorage.getItem('member')), 'original');
    }],
    ['IndexedDB discovery and committed record updates work for inline and out-of-line keys', async () => {
      const done = await start([
        ['shell_storage', { operation: 'read', storage: 'indexeddb' }],
        ['shell_storage', { operation: 'read', storage: 'indexeddb', database: 'members' }],
        ['shell_storage', { operation: 'read', storage: 'indexeddb', database: 'members', object_store: 'records', key: 'member' }],
        ['shell_storage', values => ({ ...target(values[2]), storage: 'indexeddb', database: 'members', object_store: 'records', operation: 'set', key: 'member', value: { id: 'member', displayName: 'Alice' } })],
        ['shell_storage', values => ({ ...target(values[2]), storage: 'indexeddb', database: 'members', object_store: 'keyed', operation: 'set', key: 'member', value: { displayName: 'Keyed Alice' } })],
        ['shell_storage', { operation: 'read', storage: 'indexeddb', database: 'members', object_store: 'records', key: 'member' }],
      ]);
      await allow();
      await allow();
      const values = await done();
      assert.ok(values[0].databases.some(db => db.name === 'members'));
      assert.ok(values[1].object_stores.some(store => store.name === 'records' && store.key_path === 'id'));
      assert.equal(values[2].entries[0].value.displayName, 'Original');
      assert.equal(values[3].verified, true);
      assert.equal(values[4].after.displayName, 'Keyed Alice');
      assert.equal(values[5].entries[0].value.displayName, 'Alice');
    }],
    ['cookies can be modified and deleted with approval and verified read-back', async () => {
      const done = await start([
        ['shell_cookie', { operation: 'list', name: 'member_cookie' }],
        ['shell_cookie', values => ({ ...target(values[0]), operation: 'set', name: 'member_cookie', value: 'Alice', path: '/', httpOnly: true, sameSite: 'Lax' })],
        ['shell_cookie', values => ({ ...target(values[0]), operation: 'delete', name: 'member_cookie', path: '/' })],
        ['shell_cookie', { operation: 'list', name: 'member_cookie' }],
      ]);
      await allow();
      await allow();
      const values = await done();
      assert.equal(values[1].verified, true);
      assert.equal(values[1].cookies[0].httpOnly, true);
      assert.equal(values[1].cookies[0].value, 'Alice');
      assert.equal(values[2].verified, true);
      assert.deepEqual(values[3].cookies, []);
    }],
    ['storage deletes are committed and missing IndexedDB databases are not created', async () => {
      const done = await start([
        ['shell_storage', { operation: 'read', storage: 'local', key: 'member' }],
        ['shell_storage', values => ({ ...target(values[0]), storage: 'local', operation: 'delete', key: 'member' })],
        ['shell_storage', values => ({ ...target(values[0]), storage: 'indexeddb', database: 'members', object_store: 'records', operation: 'delete', key: 'member' })],
        ['shell_storage', { operation: 'read', storage: 'indexeddb', database: 'missing-database' }],
        ['shell_storage', { operation: 'read', storage: 'indexeddb' }],
      ]);
      await allow();
      await allow();
      const values = await done();
      assert.equal(values[1].after, null);
      assert.equal(values[2].after, null);
      assert.equal(values[2].verified, true);
      assert.match(values[3], /does not exist/i);
      assert.ok(!values[4].databases.some(db => db.name === 'missing-database'));
    }],
    ['native WebView F12 opens the focused cell inspector from page, omnibox and chat', async () => {
      const session = await app.browser.newBrowserCDPSession();
      try {
        for (const [cell, suffix, focus] of [[0, '/left', () => left.locator('h1').click()],
          [1, '/right', () => app.shell.locator('#omnibox').focus()],
          [0, '/left', () => side.locator('textarea').focus()]]) {
          await app.invoke('shell_canvas', { operation: 'cell_focus', cell });
          await focus();
          pressF12(app.child.pid);
          const inspector = await waitFor(async () => (await session.send('Target.getTargets')).targetInfos.find(target =>
            target.url.startsWith('devtools://') && target.title.includes(suffix)));
          assert.ok(!inspector.title.includes('asset.shell'));
          await session.send('Target.closeTarget', { targetId: inspector.targetId });
          await waitFor(async () => !(await session.send('Target.getTargets')).targetInfos.some(target => target.targetId === inspector.targetId));
        }
      } finally { await session.detach(); }
    }],
  ]);
} finally {
  await app.close();
  fixtures.server.close();
  model.server.close();
}
process.exitCode = failed ? 1 : 0;
