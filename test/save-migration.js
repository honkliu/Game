// Uses only the isolated audit database, never the normal play database.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { DatabaseSync } = require('node:sqlite');

const source = process.argv[2], database = process.argv[3];
assert.ok(source && database && source !== database, 'Supply separate isolated source and migration databases.');
fs.copyFileSync(source, database);
const serverDir = path.resolve(__dirname, '..', 'server', 'CityOfTowers');
let server, base;

async function start() {
  let output = '';
  server = spawn('dotnet', [path.join(serverDir, 'bin', 'Debug', 'net10.0', 'CityOfTowers.dll'),
    '--urls', 'http://127.0.0.1:0'], {
    cwd: serverDir,
    env: { ...process.env, ASPNETCORE_ENVIRONMENT: 'Development',
      ConnectionStrings__Default: `Data Source=${database}`, Logging__LogLevel__Default: 'Warning',
      'Logging__LogLevel__Microsoft.Hosting.Lifetime': 'Information' },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let error;
  server.on('error', e => { error = e; });
  server.stdout.on('data', data => { output += data; });
  server.stderr.on('data', data => { output += data; });
  for (let i = 0; i < 120; i++) {
    const address = output.match(/Now listening on:\s+(http:\/\/127\.0\.0\.1:\d+)/)?.[1];
    if (address && (await fetch(`${address}/api/players`)).ok) { base = address; return; }
    if (error || server.exitCode !== null) throw error || new Error(output);
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Migration server readiness timed out: ${output}`);
}

async function stop() {
  if (server && server.exitCode === null) {
    const closed = once(server, 'close');
    server.kill();
    await closed;
  }
  server = null;
}

async function request(route, body, status = 200) {
  const response = await fetch(`${base}${route}`, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { 'X-Player-Id': 'alice', 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  const result = response.status === 204 ? null : await response.json();
  assert.equal(response.status, status, JSON.stringify(result));
  return result;
}

(async () => {
  try {
    let db = new DatabaseSync(database);
    const hero = db.prepare("SELECT h.* FROM Heroes h JOIN Players p ON p.Id = h.PlayerId WHERE p.Faction = 'castle' LIMIT 1").get();
    assert.ok(hero, 'The isolated integration suite must create an Alice hero.');
    // Keep one fresh game so unrelated startup pruning/timers cannot alter the migration evidence.
    for (const table of ['Heroes', 'Players', 'Towns'])
      db.prepare(`DELETE FROM ${table} WHERE GameId != ?`).run(hero.GameId);
    db.prepare('DELETE FROM Games WHERE Id != ?').run(hero.GameId);
    db.prepare('UPDATE Games SET TurnStartedAt = ?, State = 1, TurnLimit = 0, TurnSeconds = 86400').run('2099-01-01 00:00:00');
    db.exec("UPDATE Heroes SET Experience = 0, Level = 1, PendingEncounterJson = NULL; UPDATE Players SET IsAi = 0 WHERE Faction = 'rampart'");
    for (const [table, columns] of [['Heroes', ['PendingTreasureJson', 'AdventureEffectsJson', 'ArtifactLayoutJson', 'PendingLevelJson']],
      ['Players', ['MineRemaindersJson']], ['Games', ['EndAgreementJson']]])
      for (const column of columns) db.exec(`ALTER TABLE ${table} DROP COLUMN ${column}`);
    const queries = ['Games', 'Players', 'Towns', 'Heroes'].map(table => {
      const columns = db.prepare(`SELECT name FROM pragma_table_info('${table}')`).all().map(row => `"${row.name}"`).join(',');
      return `SELECT ${columns} FROM ${table} ORDER BY Id`;
    });
    const before = queries.map(query => db.prepare(query).all());
    db.close();
    await start();
    db = new DatabaseSync(database);
    assert.deepEqual(queries.map(query => db.prepare(query).all()), before, 'Migration must preserve all existing core rows.');
    assert.ok(db.prepare('SELECT PendingTreasureJson, AdventureEffectsJson FROM Heroes').all()
      .every(row => row.PendingTreasureJson === null && row.AdventureEffectsJson === '{}'));
    assert.ok(db.prepare('SELECT ArtifactLayoutJson, PendingLevelJson FROM Heroes').all()
      .every(row => row.ArtifactLayoutJson === '{}' && row.PendingLevelJson === null));
    assert.ok(db.prepare('SELECT EndAgreementJson FROM Games').all().every(row => row.EndAgreementJson === '[]'));
    assert.ok(db.prepare('SELECT MineRemaindersJson FROM Players').all().every(row => row.MineRemaindersJson === '{}'));
    db.close();
    console.log('PASS existing-schema migration preserves games, heroes, players and towns');

    const fixture = await request(`/api/heroes/${hero.Id}/_test/adventure`, { spells: ['fly', 'haste'], treasure: true });
    await request(`/api/heroes/${hero.Id}/cast`, { spell: 'haste' });
    await request(`/api/heroes/${hero.Id}/cast`, { spell: 'fly' });
    await request(`/api/heroes/${hero.Id}/move`, { path: [{ x: fixture.x, y: fixture.y + 1 }] });
    const pending = await request(`/api/heroes/${hero.Id}`);
    assert.ok(pending.pendingTreasure && pending.flying && pending.hasteBonus === 300);
    await stop();
    await start();
    assert.deepEqual(await request(`/api/heroes/${hero.Id}`), pending, 'Pending treasure and day effects must survive a process restart.');
    console.log('PASS restart preserves pending treasure, active Fly/Haste, mana and movement');
    const game = await request(`/api/games/${hero.GameId}`);
    const gold = game.players.find(player => player.name === 'Alice').resources.gold;
    await request(`/api/heroes/${hero.Id}/treasure`, { choice: 'gold' });
    const resolved = await request(`/api/heroes/${hero.Id}`);
    const updated = await request(`/api/games/${hero.GameId}`);
    assert.equal(resolved.pendingTreasure, null);
    assert.equal(resolved.experience, pending.experience);
    assert.equal(updated.players.find(player => player.name === 'Alice').resources.gold, gold + 1500);
    console.log('PASS resumed treasure resolves exactly one reward after idempotent schema startup');

    await request(`/api/heroes/${hero.Id}/_test/adventure`, {
      meeting: 'own', spells: ['visions', 'dimensionDoor', 'townPortal'],
      skills: { scholar: 3, wisdom: 3, earth: 3 },
      army: [{ creature: 'pikeman', count: 25 }, { creature: 'archer', count: 8 }]
    });
    await request(`/api/heroes/${hero.Id}/_test/artifacts`, ['artifact1', 'artifact2']);
    await request(`/api/heroes/${hero.Id}/artifacts`, { from: 'leftHand', to: 'backpack:0' });
    const friends = await request(`/api/games/${hero.GameId}/heroes`);
    const friend = friends.find(h => h.id.toLowerCase() !== hero.Id.toLowerCase() && h.name === 'Companion');
    assert.ok(friend, 'The fixture must create a physically adjacent owned hero.');
    await request(`/api/heroes/${hero.Id}/exchange`, { otherHeroId: friend.id, fromSlot: 0, toSlot: 2, count: 5 });
    await request(`/api/heroes/${hero.Id}/artifacts`, { from: 'head', to: 'head', toHeroId: friend.id });
    await request(`/api/heroes/${hero.Id}/dismiss-stack`, { slot: 1, confirmed: true });
    await request(`/api/heroes/${hero.Id}/_test/adventure`, { meeting: 'own' });
    const expanded = await request(`/api/games/${hero.GameId}/heroes`);
    const discarded = expanded.find(h => !friends.some(existing => existing.id === h.id));
    assert.ok(discarded);
    await request(`/api/heroes/${discarded.id}/dismiss`, { confirmed: true }, 204);
    const mineHero = await request(`/api/heroes/${hero.Id}/_test/adventure`, { objectKind: 'mine', objectData: 'mercury' });
    await request(`/api/heroes/${hero.Id}/move`, { path: [{ x: mineHero.x, y: mineHero.y - 1 }] });
    await request(`/api/games/${hero.GameId}/mine-sharing`, { x: mineHero.x, y: mineHero.y - 1, shared: true }, 204);
    await request(`/api/games/${hero.GameId}/endturn`, {});
    await request(`/api/heroes/${hero.Id}/cast`, { spell: 'visions' });
    const location = await request(`/api/heroes/${hero.Id}`);
    await request(`/api/heroes/${hero.Id}/cast`, { spell: 'dimensionDoor', x: location.x, y: location.y + 1 });
    await request(`/api/heroes/${hero.Id}/_test/adventure`, { experience: 3000 });
    await request(`/api/games/${hero.GameId}/agreement`, { agree: true });
    const durableHero = await request(`/api/heroes/${hero.Id}`);
    const durableFriend = await request(`/api/heroes/${friend.id}`);
    const durableGame = await request(`/api/games/${hero.GameId}`);
    assert.equal(durableHero.pendingLevel.level, 2);
    assert.equal(durableHero.doorCasts, 1);
    assert.equal(durableHero.visions, true);
    assert.equal(durableHero.army[0].count, 20);
    assert.equal(durableHero.army[1].creature, null);
    assert.equal(durableHero.backpack[0].id, 14);
    assert.equal(durableFriend.worn.head.id, 21);
    assert.equal(durableGame.state, 'InProgress');
    assert.equal(durableGame.endAgreement.length, 1);
    db = new DatabaseSync(database);
    const carry = db.prepare('SELECT MineRemaindersJson FROM Players WHERE Id = ?').get(hero.PlayerId).MineRemaindersJson;
    assert.equal(JSON.parse(carry).mercury, 1);
    const map = JSON.parse(db.prepare('SELECT MapJson FROM Games').get().MapJson);
    assert.equal(map.tiles[(mineHero.y - 1) * map.width + mineHero.x].mineShared, true);
    db.close();
    await stop(); await start();
    assert.deepEqual(await request(`/api/heroes/${hero.Id}`), durableHero);
    assert.deepEqual(await request(`/api/heroes/${friend.id}`), durableFriend);
    assert.deepEqual(await request(`/api/games/${hero.GameId}`), durableGame);
    assert.equal((await fetch(`${base}/api/heroes/${discarded.id}`)).status, 404);
    db = new DatabaseSync(database);
    assert.equal(db.prepare('SELECT MineRemaindersJson FROM Players WHERE Id = ?').get(hero.PlayerId).MineRemaindersJson, carry);
    assert.equal(JSON.parse(db.prepare('SELECT MapJson FROM Games').get().MapJson)
      .tiles[(mineHero.y - 1) * map.width + mineHero.x].mineShared, true);
    db.close();
    console.log('PASS restart preserves army exchange, troop/hero dismissal, equipment placement, shared mines, fractional carry, Visions, Door counters and agreement votes');
    const chosen = durableHero.pendingLevel.choices[0].key;
    await request(`/api/heroes/${hero.Id}/level-up`, { skill: chosen });
    const queued = await request(`/api/heroes/${hero.Id}`);
    assert.equal(queued.pendingLevel.level, 3);
    await stop(); await start();
    assert.deepEqual(await request(`/api/heroes/${hero.Id}`), queued);
    await request(`/api/heroes/${hero.Id}/level-up`, { skill: queued.pendingLevel.choices[0].key });
    assert.equal((await request(`/api/heroes/${hero.Id}`)).pendingLevel, null);
    console.log('PASS restart preserves queued level choices and resolves them without replaying experience or primary gains');
    const monster = await request(`/api/heroes/${hero.Id}/_test/adventure`, { objectKind: 'monster', objectData: 'pikeman:5' });
    await request(`/api/heroes/${hero.Id}/move`, { path: [{ x: monster.x, y: monster.y - 1 }] });
    const encounter = await request(`/api/heroes/${hero.Id}`);
    assert.ok(encounter.encounterOffer);
    await stop(); await start();
    assert.deepEqual(await request(`/api/heroes/${hero.Id}`), encounter);
    await request(`/api/heroes/${hero.Id}/encounter`, { action: 'pass' });
    assert.equal((await request(`/api/heroes/${hero.Id}`)).pendingEncounter, null);
    console.log('PASS restart preserves a native creature offer and allows one resumed decision');
  } finally { await stop(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
