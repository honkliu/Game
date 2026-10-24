// Server tests for the social layer: encounters, messages, trades, projects, annotations.
const BASE = process.argv[2] || 'http://localhost:5199';

let pass = 0, fail = 0;
const failures = [];
function check(name, cond, detail) {
  if (cond) { pass++; console.log('  ok   ' + name); }
  else { fail++; failures.push(name + (detail ? ' — ' + detail : '')); console.log('  FAIL ' + name + (detail ? ' — ' + detail : '')); }
}

async function call(method, path, playerId, body) {
  const headers = { 'Content-Type': 'application/json' };
  if (playerId) headers['X-Player-Id'] = playerId;
  const res = await fetch(BASE + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  let json = null;
  if (text) { try { json = JSON.parse(text); } catch { json = text; } }
  return { status: res.status, body: json };
}

(async () => {
  console.log('Testing ' + BASE + '\n');

  // create a fresh game
  let r = await call('POST', '/api/games', null, { name: 'Social Test', capacity: 3, turnSeconds: 300, turnLimit: 0, mapSize: 48 });
  const gameId = r.body.gameId;
  check('game created', !!gameId);

  r = await call('GET', '/api/games/' + gameId);
  const players = r.body.players;
  const alice = players.find(p => p.faction === 'castle');
  const bob = players.find(p => p.faction === 'rampart');
  check('three players', players.length === 3);

  // ---------------------------------------------------------------- messages
  console.log('\nMessages');
  r = await call('POST', `/api/games/${gameId}/messages`, 'alice', { toPlayerId: bob.playerId, body: 'Hello Bob' });
  check('send message', r.status === 204, JSON.stringify(r.body));

  r = await call('GET', `/api/games/${gameId}/messages`, 'bob');
  check('bob receives message', r.status === 200 && r.body.length === 1, JSON.stringify(r.body));
  check('message body', r.body[0].body === 'Hello Bob');
  check('message from alice', r.body[0].from === 'Alice');

  r = await call('POST', `/api/games/${gameId}/messages`, 'alice', { toPlayerId: bob.playerId, body: '' });
  check('empty message rejected', r.status === 400);

  // ---------------------------------------------------------------- trades
  console.log('\nTrades');
  r = await call('POST', `/api/games/${gameId}/trades`, 'alice', {
    toPlayerId: bob.playerId, give: 'wood', giveAmount: 5, want: 'ore', wantAmount: 5
  });
  check('offer trade', r.status === 204, JSON.stringify(r.body));

  r = await call('GET', `/api/games/${gameId}/trades`, 'bob');
  check('bob sees offer', r.status === 200 && r.body.length === 1, JSON.stringify(r.body));
  const offerId = r.body[0].id;
  check('offer is incoming', r.body[0].incoming === true);
  check('offer pending', r.body[0].status === 'pending');

  const bobBefore = (await call('GET', '/api/games/' + gameId)).body.players.find(p => p.faction === 'rampart').resources;
  const aliceBefore = (await call('GET', '/api/games/' + gameId)).body.players.find(p => p.faction === 'castle').resources;

  r = await call('POST', `/api/trades/${offerId}/respond`, 'bob', { accept: true });
  check('bob accepts', r.status === 204, JSON.stringify(r.body));

  const after = (await call('GET', '/api/games/' + gameId)).body.players;
  const bobAfter = after.find(p => p.faction === 'rampart').resources;
  const aliceAfter = after.find(p => p.faction === 'castle').resources;
  check('alice lost 5 wood', aliceAfter.wood === aliceBefore.wood - 5, `${aliceBefore.wood} -> ${aliceAfter.wood}`);
  check('bob gained 5 wood', bobAfter.wood === bobBefore.wood + 5, `${bobBefore.wood} -> ${bobAfter.wood}`);
  check('bob lost 5 ore', bobAfter.ore === bobBefore.ore - 5, `${bobBefore.ore} -> ${bobAfter.ore}`);
  check('alice gained 5 ore', aliceAfter.ore === aliceBefore.ore + 5, `${aliceBefore.ore} -> ${aliceAfter.ore}`);

  // cannot accept twice
  r = await call('POST', `/api/trades/${offerId}/respond`, 'bob', { accept: true });
  check('cannot accept twice', r.status === 400);

  // ---------------------------------------------------------------- annotations
  console.log('\nAnnotations');
  r = await call('POST', `/api/games/${gameId}/annotations`, 'alice', { x: 10, y: 12, text: 'Rich ore here' });
  check('add annotation', r.status === 204, JSON.stringify(r.body));

  r = await call('GET', `/api/games/${gameId}/annotations`, 'bob');
  check('bob sees annotation', r.status === 200 && r.body.length === 1, JSON.stringify(r.body));
  check('annotation text', r.body[0].text === 'Rich ore here');
  check('annotation author', r.body[0].author === 'Alice');

  // ---------------------------------------------------------------- projects
  console.log('\nProjects');
  r = await call('GET', `/api/games/${gameId}/projects`, 'alice');
  check('projects list', r.status === 200 && Array.isArray(r.body), JSON.stringify(r.body).slice(0, 100));
  check('three projects seeded', r.body.length === 3, r.body.map(p => p.name).join(','));
  const festival = r.body.find(p => p.name === 'Festival of Banners');
  check('festival exists', !!festival);
  if (festival) {
    const xpBefore = (await call('GET', `/api/games/${gameId}/heroes`, 'alice')).body[0].experience;
    r = await call('POST', `/api/projects/${festival.id}/contribute`, 'alice', { gold: 1500, wood: 5, ore: 0 });
    check('alice contributes', r.status === 204, JSON.stringify(r.body));
    r = await call('POST', `/api/projects/${festival.id}/contribute`, 'bob', { gold: 1500, wood: 0, ore: 5 });
    check('bob completes festival', r.status === 204, JSON.stringify(r.body));
    r = await call('GET', `/api/games/${gameId}/projects`, 'alice');
    check('festival completed', r.body.find(p => p.id === festival.id).completed === true);
    const aliceHero = (await call('GET', `/api/games/${gameId}/heroes`, 'alice')).body[0];
    check('contributor hero gained experience', aliceHero.experience > xpBefore || aliceHero.level > 1, `${xpBefore} -> ${aliceHero.experience}`);
    r = await call('POST', `/api/projects/${festival.id}/contribute`, 'carol', { gold: 1, wood: 0, ore: 0 });
    check('completed project rejects more', r.status === 400);
  }

  // ---------------------------------------------------------------- tower content
  console.log('\nTower content');
  const aliceTownId = alice.townId;
  r = await call('POST', `/api/towns/${aliceTownId}/posts`, 'alice', { title: 'First', body: 'Hello world', imageUrl: null });
  check('post without tower rejected', r.status === 400 && /no Magic Tower/.test(r.body.error), JSON.stringify(r.body));

  r = await call('POST', `/api/towns/${aliceTownId}/wall`, 'bob', { body: 'Too early', parentId: null });
  check('undiscovered town rejects wall post', r.status === 400, JSON.stringify(r.body));
  r = await call('POST', `/api/games/${gameId}/_test/explore`, 'bob');
  check('test explore (development only)', r.status === 204, r.status);

  r = await call('POST', `/api/towns/${aliceTownId}/wall`, 'bob', { body: 'Nice town!', parentId: null });
  check('bob posts on alice wall', r.status === 200, JSON.stringify(r.body));

  r = await call('GET', `/api/towns/${aliceTownId}/wall`, 'alice');
  check('wall has message', r.body.length === 1 && r.body[0].body === 'Nice town!');
  check('wall author is Bob', r.body[0].author === 'Bob');

  r = await call('POST', `/api/towns/${aliceTownId}/tower`, 'bob', { body: 'Great city' });
  check('tower guestbook', r.status === 200, JSON.stringify(r.body));

  // ---------------------------------------------------------------- encounters
  console.log('\nEncounters');
  r = await call('GET', `/api/games/${gameId}/heroes`, 'alice');
  const hero = r.body[0];
  r = await call('POST', `/api/heroes/${hero.id}/encounter`, 'alice', { action: 'observe' });
  check('encounter with no pending is rejected', r.status === 400, JSON.stringify(r.body));

  // ---------------------------------------------------------------- summary
  console.log('\n' + '='.repeat(60));
  console.log('PASS ' + pass + '   FAIL ' + fail);
  if (failures.length) {
    console.log('\nFailures:');
    for (const f of failures) console.log('  - ' + f);
  }
  process.exit(fail === 0 ? 0 : 1);
})().catch(e => { console.error('CRASHED', e); process.exit(1); });