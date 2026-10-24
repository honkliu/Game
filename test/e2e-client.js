// End-to-end browser test: drives the real React UI through a full game.
const { spawn } = require('child_process');
const http = require('http');
const path = require('path');
const fs = require('fs');
const WebSocket = require('ws');

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const PORT = 9400;
const ROOT = path.join(__dirname, '..');
const BASE = 'http://localhost:5199';
const OUT = path.join(ROOT, 'shots');
if (!fs.existsSync(OUT)) fs.mkdirSync(OUT, { recursive: true });

const get = (url) => new Promise((res, rej) => {
  http.get(url, (r) => { let d = ''; r.on('data', (c) => (d += c)); r.on('end', () => res(d)); }).on('error', rej);
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0;
const failures = [];
function check(name, cond, detail) {
  if (cond) { pass++; console.log('  ok   ' + name); }
  else { fail++; failures.push(name + (detail ? ' — ' + detail : '')); console.log('  FAIL ' + name + (detail ? ' — ' + detail : '')); }
}

(async () => {
  const chrome = spawn(CHROME, [
    '--headless=new', '--disable-gpu', '--remote-debugging-port=' + PORT,
    '--user-data-dir=' + path.join(ROOT, '.chrome-e2e'), '--window-size=1400,900',
    '--allow-file-access-from-files', 'about:blank'
  ], { stdio: 'ignore' });

  let targets = null;
  for (let i = 0; i < 40; i++) {
    try { targets = JSON.parse(await get('http://127.0.0.1:' + PORT + '/json/list')); if (targets.length) break; } catch (e) {}
    await sleep(250);
  }
  const ws = new WebSocket(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
  let id = 0; const pending = new Map(); const errors = [];
  await new Promise((r) => ws.on('open', r));
  ws.on('message', (raw) => {
    const m = JSON.parse(raw);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); return; }
    if (m.method === 'Runtime.exceptionThrown') {
      errors.push((m.params.exceptionDetails.exception || {}).description || m.params.exceptionDetails.text);
    }
  });
  const send = (method, params) => new Promise((res) => { const mid = ++id; pending.set(mid, res); ws.send(JSON.stringify({ id: mid, method, params: params || {} })); });
  const ev = async (expr) => {
    const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
    const res = r.result || {};
    if (res.exceptionDetails) errors.push(JSON.stringify(res.exceptionDetails.exception || res.exceptionDetails.text));
    return res.result ? res.result.value : undefined;
  };
  const shot = async (name) => {
    const r = await send('Page.captureScreenshot', { format: 'png' });
    if (r.result && r.result.data) fs.writeFileSync(path.join(OUT, name + '.png'), Buffer.from(r.result.data, 'base64'));
  };
  const clickText = async (text) => {
    const ok = await ev(`(function(){
      var els = Array.from(document.querySelectorAll('button, .link, .tab')).filter(function(e){ return e.offsetParent !== null; });
      var el = els.find(function(e){ return e.textContent.trim() === ${JSON.stringify(text)}; });
      if (!el) return false;
      el.click();
      return true;
    })()`);
    await sleep(400);
    return ok;
  };
  const typeInto = async (selector, value) => {
    await ev(`(function(){
      var el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return false;
      var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(el, ${JSON.stringify(value)});
      el.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    })()`);
    await sleep(120);
  };

  await send('Runtime.enable');
  await send('Page.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1400, height: 900, deviceScaleFactor: 1, mobile: false });

  console.log('Loading ' + BASE);
  await send('Page.navigate', { url: BASE });
  await sleep(2500);

  // ---------------------------------------------------------------- login screen
  console.log('\nS1  Login screen');
  const title = await ev(`document.body.innerText.includes('CITY OF TOWERS')`);
  check('title renders', title === true);
  const hasInputs = await ev(`document.querySelectorAll('input').length`);
  check('two inputs present', hasInputs === 2, 'got ' + hasInputs);
  await shot('e2e-01-login');

  // register
  const suffix = Date.now().toString(36).slice(-6);
  const userA = 'alice_' + suffix;
  const userB = 'bob_' + suffix;

  await typeInto('input:not([type=password])', userA);
  await typeInto('input[type=password]', 'password123');
  await clickText('Register');
  await sleep(1200);

  const onLobby = await ev(`document.body.innerText.includes('OPEN GAMES')`);
  check('registered and reached lobby', onLobby === true);
  await shot('e2e-02-lobby');

  // ---------------------------------------------------------------- create game
  console.log('\nS2  Lobby / create game');
  await clickText('Create Game');
  await sleep(400);
  const modalOpen = await ev(`document.body.innerText.includes('Turn length')`);
  check('create modal opens', modalOpen === true);
  await shot('e2e-03-create');

  await clickText('Create');
  await sleep(1200);
  const inRoom = await ev(`document.body.innerText.includes('WAITING FOR PLAYERS')`);
  check('reached game room', inRoom === true);
  await shot('e2e-04-room');

  const gameId = await ev(`location.hash.split('/')[2]`);
  check('game id in route', !!gameId, gameId);

  // ---------------------------------------------------------------- second player joins via API
  console.log('\nS3  Second player joins');
  const reg = await fetch(BASE + '/api/auth/register', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: userB, password: 'password123' })
  });
  const cookieB = (reg.headers.get('set-cookie') || '').split(';')[0];
  const join = await fetch(BASE + '/api/games/' + gameId + '/join', {
    method: 'POST', headers: { 'Content-Type': 'application/json', Cookie: cookieB }, body: '{}'
  });
  check('bob joined via API', join.status === 200, 'status ' + join.status);

  await sleep(3500); // room polls every 3s
  const twoPlayers = await ev(`document.body.innerText.includes('${userB}')`);
  check('bob appears in room (polling)', twoPlayers === true);
  await shot('e2e-05-room-two');

  // ---------------------------------------------------------------- start
  console.log('\nS4  Start game');
  const started = await clickText('Start');
  check('start button clicked', started === true);
  await sleep(2000);
  const inCity = await ev(`location.hash.includes('/city/')`);
  check('navigated to city view', inCity === true, await ev('location.hash'));
  const cityPlayerId = await ev(`location.hash.split('/')[4]`);
  check('player id in route', !!cityPlayerId, cityPlayerId);
  await shot('e2e-06-city');

  const hasResourceBar = await ev(`document.body.innerText.includes('WOOD') && document.body.innerText.includes('GOLD')`);
  check('resource bar renders', hasResourceBar === true);

  const hasBuild = await ev(`Array.from(document.querySelectorAll('button')).some(function(b){return b.textContent.trim()==='Build';})`);
  check('Build button visible for owner', hasBuild === true);

  // ---------------------------------------------------------------- build
  console.log('\nS5  Build dialog');
  await clickText('Build');
  await sleep(500);
  const buildOpen = await ev(`document.body.innerText.includes('You may build one structure per turn')`);
  check('build modal opens', buildOpen === true);
  await shot('e2e-07-build');

  const built = await ev(`(function(){
    var els = Array.from(document.querySelectorAll('div'));
    var row = els.find(function(e){ return e.textContent.trim().endsWith("Woodcutter's Lodge") && e.style.cursor === 'pointer'; });
    if (!row) return false;
    row.click();
    return true;
  })()`);
  check('selected woodcutter', built === true);
  await sleep(300);
  await ev("(function(){var bs=Array.from(document.querySelectorAll('.modal button')).filter(function(b){return b.textContent.trim()==='Build';});if(bs.length)bs[0].click();return bs.length;})()");
  await sleep(1500);
  const builtOk = await ev(`document.body.innerText.includes('Built Woodcutter')`);
  check('build succeeded (toast)', builtOk === true);
  await shot('e2e-08-built');

  const spriteOk = await ev("(function(){var imgs=Array.from(document.querySelectorAll('img'));return imgs.filter(function(i){return i.src.indexOf('/assets/buildings/')>=0 && i.naturalWidth>0;}).length;})()");
  check('building sprites load', spriteOk >= 1, 'loaded ' + spriteOk);

  // ---------------------------------------------------------------- tower
  console.log('\nS6  Magic Tower');
  await clickText('Tower');
  await sleep(1000);
  const inTower = await ev(`location.hash.includes('/tower')`);
  check('navigated to tower', inTower === true);
  const noTower = await ev(`document.body.innerText.includes('no Magic Tower yet')`);
  check('shows no-tower state', noTower === true);
  await shot('e2e-09-tower');

  // ---------------------------------------------------------------- obelisk
  console.log('\nS7  Obelisk');
  await ev(`location.hash = '#/game/${gameId}/obelisk'`);
  await sleep(1200);
  const inObelisk = await ev(`document.body.innerText.includes('THE OBELISK')`);
  check('obelisk screen renders', inObelisk === true);
  await shot('e2e-10-obelisk');

  const visited = await clickText('Visit the Obelisk');
  check('visit button clicked', visited === true);
  await sleep(1200);
  const fragmentShown = await ev(`document.body.innerText.includes('1 of 10') || document.body.innerText.includes('You have seen 1')`);
  check('fragment revealed', fragmentShown === true);
  await shot('e2e-11-fragment');

  // ---------------------------------------------------------------- wall
  console.log('\nS8  Wall');
  await ev(`location.hash = '#/game/${gameId}/city/${cityPlayerId}'`);
  await sleep(1200);
  await clickText('Wall');
  await sleep(1200);
  const wallOpen = await ev(`document.body.innerText.includes('WALL')`);
  check('wall panel opens', wallOpen === true);
  await shot('e2e-12-wall');

  // ---------------------------------------------------------------- summary
  console.log('\n' + '='.repeat(60));
  console.log('PASS ' + pass + '   FAIL ' + fail);
  if (failures.length) {
    console.log('\nFailures:');
    for (const f of failures) console.log('  - ' + f);
  }
  if (errors.length) {
    console.log('\nConsole errors:');
    for (const e of errors.slice(0, 8)) console.log('  ! ' + String(e).slice(0, 300));
  }

  ws.close();
  chrome.kill();
  process.exit(fail === 0 && errors.length === 0 ? 0 : 1);
})().catch((e) => { console.log('TEST CRASHED', e); process.exit(1); });