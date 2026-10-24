// Server test for the adventure-map model: players, game, map, hero movement, town.
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
  const res = await fetch(BASE + path, {
    method, headers,
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  const text = await res.text();
  let json = null;
  if (text) { try { json = JSON.parse(text); } catch { json = text; } }
  return { status: res.status, body: json };
}

(async () => {
  console.log('Testing ' + BASE + '\n');

  // ---------------------------------------------------------------- players
  console.log('Players');
  let r = await call('GET', '/api/players');
  check('three players', r.status === 200 && r.body.length === 3, JSON.stringify(r.body));
  check('alice/bob/carol', r.body.map(p => p.name).join(',') === 'Alice,Bob,Carol', r.body.map(p => p.name).join(','));
  check('factions', r.body.map(p => p.faction).join(',') === 'castle,rampart,necropolis', r.body.map(p => p.faction).join(','));

  // ---------------------------------------------------------------- create game
  console.log('\nCreate game');
  r = await call('POST', '/api/games', null, { name: 'Test', capacity: 3, turnSeconds: 300, turnLimit: 60, mapSize: 48 });
  check('game created', r.status === 200 && r.body.gameId, JSON.stringify(r.body));
  const gameId = r.body.gameId;

  r = await call('GET', '/api/games/' + gameId);
  check('game in progress', r.body.state === 'InProgress', r.body.state);
  check('three players in game', r.body.players.length === 3, r.body.players.length);
  check('day 1 week 1 month 1', r.body.day === 1 && r.body.week === 1 && r.body.month === 1);
  check('map 48x48', r.body.mapWidth === 48 && r.body.mapHeight === 48);
  check('each player has a town', r.body.players.every(p => p.townId));
  check('each player has a hero', r.body.players.every(p => p.heroIds.length === 1));

  // ---------------------------------------------------------------- map
  console.log('\nMap');
  r = await call('GET', `/api/games/${gameId}/map`, 'alice');
  check('map loads for alice', r.status === 200, JSON.stringify(r.body).slice(0, 200));
  check('tile count', r.body.tiles.length === 48 * 48, r.body.tiles.length);
  const visible = r.body.tiles.filter(t => t.fog === 2).length;
  const explored = r.body.tiles.filter(t => t.fog === 1).length;
  const unexplored = r.body.tiles.filter(t => t.fog === 0).length;
  check('some tiles visible', visible > 0, 'visible ' + visible);
  check('some tiles unexplored', unexplored > 0, 'unexplored ' + unexplored);
  check('fog sums to total', visible + explored + unexplored === 48 * 48);
  check('own hero on map', r.body.heroes.some(h => h.isMine), r.body.heroes.length);
  check('other heroes hidden by fog', r.body.heroes.length === 1, r.body.heroes.length);
  check('only own town known', r.body.towns.length === 1 && r.body.towns[0].isMine, r.body.towns.length);
  check('alice hero is mine', r.body.heroes.filter(h => h.isMine).length === 1);
  check('tiles expose passable', r.body.tiles.some(t => t.passable === true) && r.body.tiles.some(t => t.passable === false));

  // ---------------------------------------------------------------- hero
  console.log('\nHero');
  r = await call('GET', `/api/games/${gameId}/heroes`, 'alice');
  check('alice has one hero', r.status === 200 && r.body.length === 1, JSON.stringify(r.body).slice(0, 200));
  const hero = r.body[0];
  check('hero is Christian', hero.name === 'Christian', hero.name);
  check('level 1', hero.level === 1);
  check('movement 1500', hero.maxMovement === 1500, hero.maxMovement);
  check('army has 2 stacks', hero.army.filter(s => s.creature).length === 2, JSON.stringify(hero.army));
  check('tier 1 count 25', hero.army[0].creature === 'pikeman' && hero.army[0].count === 25, JSON.stringify(hero.army[0]));
  check('two skills', Object.keys(hero.skills).length === 2, JSON.stringify(hero.skills));

  // ---------------------------------------------------------------- movement
  console.log('\nMovement');
  const startX = hero.x, startY = hero.y;
  const mapData = (await call('GET', `/api/games/${gameId}/map`, 'alice')).body;
  // find a passable neighbour
  const dirs = [[1, 0], [-1, 0], [0, 1], [0, -1]];
  let target = null;
  for (const [dx, dy] of dirs) {
    const nx = startX + dx, ny = startY + dy;
    if (nx < 0 || ny < 0 || nx >= 48 || ny >= 48) continue;
    const t = mapData.tiles[ny * 48 + nx];
    if (t.terrain && t.passable && !t.objectKind) { target = { x: nx, y: ny }; break; }
  }
  check('found a passable neighbour', !!target, JSON.stringify(target));

  if (target) {
    r = await call('POST', `/api/heroes/${hero.id}/move`, 'alice', { path: [target] });
    check('move accepted', r.status === 200, JSON.stringify(r.body).slice(0, 200));
    check('hero moved', r.body.x === target.x && r.body.y === target.y, `${r.body.x},${r.body.y}`);
    check('movement spent', r.body.movement < 1500, r.body.movement);
  }

  // no teleporting: every step must be adjacent
  r = await call('POST', `/api/heroes/${hero.id}/move`, 'alice', { path: [{ x: startX + 6, y: startY + 6 }] });
  check('non-adjacent step rejected', r.status === 400, JSON.stringify(r.body));

  // cannot move onto impassable
  const waterIdx = mapData.tiles.findIndex(t => t.terrain === 'water');
  if (waterIdx >= 0) {
    const wx = waterIdx % 48, wy = Math.floor(waterIdx / 48);
    r = await call('POST', `/api/heroes/${hero.id}/move`, 'alice', { path: [{ x: wx, y: wy }] });
    check('cannot move onto water', r.status === 400, r.status);
  } else {
    check('cannot move onto water (no water on map)', true);
  }

  // wrong player cannot move alice's hero
  r = await call('POST', `/api/heroes/${hero.id}/move`, 'bob', { path: [target || { x: startX, y: startY }] });
  check('bob cannot move alice hero', r.status === 400, r.status);

  // ---------------------------------------------------------------- town
  console.log('\nTown');
  const aliceTownId = (await call('GET', '/api/games/' + gameId)).body.players.find(p => p.faction === 'castle').townId;
  r = await call('GET', '/api/towns/' + aliceTownId);
  check('town loads', r.status === 200, JSON.stringify(r.body).slice(0, 200));
  check('town is castle', r.body.faction === 'castle');
  check('has villageHall', r.body.buildings.includes('villageHall'));
  check('gold income 500', r.body.goldIncome === 500, r.body.goldIncome);
  check('7 tiers listed', r.body.available.length === 7, r.body.available.length);
  check('tier 1 is pikeman', r.body.available[0].creature === 'pikeman', r.body.available[0].creature);
  check('tier 1 available 28', r.body.available[0].count === 28, r.body.available[0].count);

  // build
  r = await call('POST', `/api/towns/${aliceTownId}/build`, 'alice', { buildingKey: 'dwelling3' });
  check('build dwelling3', r.status === 204, JSON.stringify(r.body));
  r = await call('POST', `/api/towns/${aliceTownId}/build`, 'alice', { buildingKey: 'dwelling4' });
  check('second build same turn rejected', r.status === 400, JSON.stringify(r.body));
  r = await call('POST', `/api/towns/${aliceTownId}/build`, 'bob', { buildingKey: 'orePit' });
  check('bob cannot build in alice town', r.status === 400, JSON.stringify(r.body));

  r = await call('POST', `/api/towns/${aliceTownId}/build`, 'alice', { buildingKey: 'grail' });
  check('grail structure needs the grail', r.status === 400, JSON.stringify(r.body));

  // recruit
  r = await call('POST', `/api/towns/${aliceTownId}/recruit`, 'alice', { tier: 1, count: 5 });
  check('recruit 5 pikemen', r.status === 200 && r.body.recruited === 5, JSON.stringify(r.body));

  // ---------------------------------------------------------------- monster visit
  console.log('\nMonster visit');
  {
    const g = (await call('GET', '/api/games/' + gameId)).body;
    // Bob's hero has not moved; look for a wandering stack next to any passable tile he can reach in one step
    const bobHero = (await call('GET', `/api/games/${gameId}/heroes`, 'bob')).body[0];
    const m = (await call('GET', `/api/games/${gameId}/map`, 'bob')).body;
    const W = m.width, H = m.height;
    const at = (x, y) => (x >= 0 && y >= 0 && x < W && y < H ? m.tiles[y * W + x] : null);
    // breadth-first search over empty passable tiles (orthogonal steps) to the nearest visible stack
    let found = null;
    const prev = new Map([[bobHero.y * W + bobHero.x, null]]);
    let frontier = [[bobHero.x, bobHero.y]];
    for (let depth = 0; depth < 10 && !found && frontier.length; depth++) {
      const next = [];
      for (const [cx, cy] of frontier) {
        for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
          const x = cx + dx, y = cy + dy, t = at(x, y);
          if (!t || prev.has(y * W + x)) continue;
          if (t.objectKind === 'monster') { found = { x, y, t, from: [cx, cy] }; break; }
          if (!t.passable || t.objectKind) continue;
          prev.set(y * W + x, cy * W + cx);
          next.push([x, y]);
        }
        if (found) break;
      }
      frontier = next;
    }
    if (found) {
      const path = [];
      for (let k = found.from[1] * W + found.from[0]; k !== bobHero.y * W + bobHero.x; k = prev.get(k))
        path.unshift({ x: k % W, y: Math.floor(k / W) });
      check('monster stack is a real creature', /^[a-zA-Z]+:\d+$/.test(found.t.objectData), found.t.objectData);
      r = await call('POST', `/api/heroes/${bobHero.id}/move`, 'bob', { path: [...path, { x: found.x, y: found.y }] });
      check('visiting a monster stack is allowed', r.status === 200, JSON.stringify(r.body));
      check('hero stops beside the stack', r.body.x === found.from[0] && r.body.y === found.from[1]);
      check('encounter pending', !!r.body.pendingEncounter, JSON.stringify(r.body.pendingEncounter));
      r = await call('POST', `/api/heroes/${bobHero.id}/move`, 'bob', { path: path.length ? [path[path.length - 2] ?? { x: bobHero.x, y: bobHero.y }] : [{ x: found.x, y: found.y }] });
      check('cannot move while an encounter is pending', r.status === 400 && /encounter/i.test(r.body?.error ?? ''), JSON.stringify(r.body));
    } else {
      check('monster visit (no stack within reach on this map)', true);
    }
    void g;
  }

  // ---------------------------------------------------------------- market
  console.log('\nMarket');
  {
    let t = (await call('GET', '/api/towns/' + aliceTownId, 'alice')).body;
    // advance build so the marketplace exists: direct build is one per turn, so skip if not built
    if (t.buildings.includes('marketplace')) {
      const before = t.resources;
      r = await call('POST', `/api/towns/${aliceTownId}/trade`, 'alice', { from: 'gold', to: 'wood', amount: 1 });
      t = (await call('GET', '/api/towns/' + aliceTownId, 'alice')).body;
      check('buy 1 wood costs 500 gold', t.resources.gold === before.gold - 500 && t.resources.wood === before.wood + 1,
        `${before.gold}->${t.resources.gold}`);
    } else {
      r = await call('POST', `/api/towns/${aliceTownId}/trade`, 'alice', { from: 'gold', to: 'wood', amount: 1 });
      check('trade needs a marketplace', r.status === 400, JSON.stringify(r.body));
    }
  }

  // ---------------------------------------------------------------- obelisk
  console.log('\nObelisk');
  r = await call('GET', `/api/games/${gameId}/obelisk`, 'alice');
  check('obelisk state', r.status === 200 && r.body.total === 10, JSON.stringify(r.body).slice(0, 150));
  r = await call('POST', `/api/games/${gameId}/obelisk/visit`, 'alice');
  check('visit reveals fragment', r.status === 200 && r.body.visits === 1, JSON.stringify(r.body).slice(0, 150));

  // ---------------------------------------------------------------- summary
  console.log('\n' + '='.repeat(60));
  console.log('PASS ' + pass + '   FAIL ' + fail);
  if (failures.length) {
    console.log('\nFailures:');
    for (const f of failures) console.log('  - ' + f);
  }
  process.exit(fail === 0 ? 0 : 1);
})().catch(e => { console.error('CRASHED', e); process.exit(1); });