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
  return { status: res.status, body: json, headers: res.headers };
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
  check('tier 1 opens with a week of growth', r.body.available[0].count === r.body.available[0].growth && r.body.available[0].growth === 14,
    JSON.stringify(r.body.available[0]));
  check('starting town has a fort and no marketplace', r.body.buildings.includes('fort') && !r.body.buildings.includes('marketplace'),
    r.body.buildings.join(','));

  // build
  r = await call('POST', `/api/towns/${aliceTownId}/build`, 'alice', { buildingKey: 'marketplace' });
  check('build marketplace', r.status === 204, JSON.stringify(r.body));
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
      check('one marketplace owned', t.marketCount === 1, t.marketCount);
      // HoMM3 rates with one marketplace: 1 wood costs 2500 gold, sells for 25, and 5 mercury buy 1 wood
      r = await call('POST', `/api/towns/${aliceTownId}/trade`, 'alice', { from: 'gold', to: 'wood', amount: 1 });
      t = (await call('GET', '/api/towns/' + aliceTownId, 'alice')).body;
      check('buy 1 wood costs 2500 gold', t.resources.gold === before.gold - 2500 && t.resources.wood === before.wood + 1,
        `${before.gold}->${t.resources.gold}`);
      r = await call('POST', `/api/towns/${aliceTownId}/trade`, 'alice', { from: 'wood', to: 'gold', amount: 1 });
      const t2 = (await call('GET', '/api/towns/' + aliceTownId, 'alice')).body;
      check('sell 1 wood for 25 gold', t2.resources.gold === t.resources.gold + 25 && t2.resources.wood === t.resources.wood - 1,
        `${t.resources.gold}->${t2.resources.gold}`);
      r = await call('POST', `/api/towns/${aliceTownId}/trade`, 'alice', { from: 'mercury', to: 'wood', amount: 1 });
      const t3 = (await call('GET', '/api/towns/' + aliceTownId, 'alice')).body;
      check('5 mercury buy 1 wood', t3.resources.mercury === t2.resources.mercury - 5 && t3.resources.wood === t2.resources.wood + 1,
        `${t2.resources.mercury}->${t3.resources.mercury}`);
    } else {
      r = await call('POST', `/api/towns/${aliceTownId}/trade`, 'alice', { from: 'gold', to: 'wood', amount: 1 });
      check('trade needs a marketplace', r.status === 400, JSON.stringify(r.body));
    }
  }

  // ---------------------------------------------------------------- HoMM3 town rules
  console.log('\nTown rules');
  {
    const town = () => call('GET', '/api/towns/' + aliceTownId, 'alice').then(x => x.body);
    const nextDay = () => call('POST', `/api/games/${gameId}/_test/nextday`);
    const build = async key => { await nextDay(); return call('POST', `/api/towns/${aliceTownId}/build`, 'alice', { buildingKey: key }); };

    r = await call('GET', '/api/buildings?faction=castle');
    check('castle tree has no Homestead', r.body.every(b => b.name !== 'Homestead') && r.body.some(b => b.name === 'Stables'),
      r.body.map(b => b.key).join(','));
    check('castle guild stops at level 4', !r.body.some(b => b.key === 'mageGuild5'));
    await nextDay();
    r = await call('POST', `/api/towns/${aliceTownId}/build`, 'alice', { buildingKey: 'dwelling3' });
    check('Griffin Tower needs the Barracks', r.status === 400 && /Barracks/.test(r.body.error), JSON.stringify(r.body));

    // the recruited pikemen went to the garrison, as in HoMM3
    let t = await town();
    check('recruits join the garrison', t.garrison[0].creature === 'pikeman' && t.garrison[0].count === 5, JSON.stringify(t.garrison));

    // walk the hero back into town
    r = await call('POST', `/api/heroes/${hero.id}/move`, 'alice', { path: [{ x: startX, y: startY }] });
    t = await town();
    check('hero is in town', t.heroInTown && t.heroInTown.id === hero.id, JSON.stringify(t.heroInTown));
    check('might hero has no spellbook', t.heroInTown && t.heroInTown.spellbook === false);

    // army exchange: garrison -> hero merges into the pikemen; a split goes to an empty slot
    const army = (from, fromSlot, to, toSlot, count = 0) =>
      call('POST', `/api/towns/${aliceTownId}/army`, 'alice', { fromArmy: from, fromSlot, toArmy: to, toSlot, count });
    r = await army('garrison', 0, 'hero', 0);
    check('merge garrison pikemen into hero', r.status === 200 && r.body.hero[0].count === 30 && !r.body.garrison[0].creature, JSON.stringify(r.body));
    r = await army('hero', 0, 'garrison', 3, 10);
    check('split 10 into the garrison', r.status === 200 && r.body.hero[0].count === 20 && r.body.garrison[3].count === 10, JSON.stringify(r.body));
    r = await army('hero', 0, 'hero', 1, 0);
    check('swap two hero stacks', r.status === 200 && r.body.hero[1].creature === 'pikeman' && r.body.hero[0].creature !== 'pikeman', JSON.stringify(r.body.hero));
    r = await army('garrison', 3, 'hero', 0, 0);
    check('swap a garrison stack onto a different hero stack', r.status === 200 && r.body.garrison[3].creature !== 'pikeman' && r.body.hero[0].creature === 'pikeman',
      JSON.stringify(r.body));
    r = await army('hero', 1, 'garrison', 6, 0);
    r = await army('hero', 0, 'garrison', 5, 0);
    check('a hero must keep one stack', r.status === 400 && /one stack/.test(r.body.error), JSON.stringify(r.body));
    r = await army('garrison', 6, 'hero', 1, 0);

    // blacksmith sells the castle's ballista
    r = await build('blacksmith');
    check('build blacksmith', r.status === 204, JSON.stringify(r.body));
    const gold0 = (await town()).resources.gold;
    r = await call('POST', `/api/towns/${aliceTownId}/blacksmith`, 'alice');
    t = await town();
    check('buy a ballista for 2500', r.status === 200 && t.heroInTown.warMachines.includes('ballista') && t.resources.gold === gold0 - 2500,
      JSON.stringify(r.body));
    r = await call('POST', `/api/towns/${aliceTownId}/blacksmith`, 'alice');
    check('only one ballista per hero', r.status === 400, JSON.stringify(r.body));

    // upgraded dwelling: recruit halberdiers from the same pool, upgrade a stack for the price difference
    r = await build('upg1');
    check('build upgraded Guardhouse', r.status === 204, JSON.stringify(r.body));
    t = await town();
    check('town reports upgraded dwelling', t.available[0].upgradeBuilt && t.available[0].upgraded === 'halberdier', JSON.stringify(t.available[0]));
    r = await call('POST', `/api/towns/${aliceTownId}/recruit`, 'alice', { tier: 1, count: 2, creature: 'halberdier' });
    check('recruit halberdiers', r.status === 200 && r.body.creature === 'halberdier', JSON.stringify(r.body));
    t = await town();
    const pikeSlot = t.heroInTown.army.findIndex(s => s.creature === 'pikeman');
    const pikes = t.heroInTown.army[pikeSlot].count;
    const g1 = t.resources.gold;
    r = await call('POST', `/api/towns/${aliceTownId}/upgrade`, 'alice', { army: 'hero', slot: pikeSlot });
    t = await town();
    check('upgrade pikemen to halberdiers', r.status === 200 && t.heroInTown.army[pikeSlot].creature === 'halberdier', JSON.stringify(r.body));
    check('upgrade costs the difference', t.resources.gold === g1 - 15 * pikes, `${g1} -> ${t.resources.gold}`);

    // mage guild: buy a spellbook, then learn the guild's level-1 spells
    r = await build('mageGuild1');
    check('build mage guild', r.status === 204, JSON.stringify(r.body));
    t = await town();
    check('guild level 1 holds five spells', t.mageGuild.length === 5, JSON.stringify(t.mageGuild));
    r = await call('POST', `/api/towns/${aliceTownId}/spellbook`, 'alice');
    t = await town();
    check('buy a spellbook and learn the guild spells', r.status === 200 && t.heroInTown.spellbook && r.body.learned.length === 5,
      JSON.stringify(r.body));
    const hs = (await call('GET', `/api/games/${gameId}/heroes`, 'alice')).body[0];
    check('hero dto hides gear from artifacts', hs.spellbook && hs.warMachines.includes('ballista') && !hs.artifacts.includes('ballista'),
      JSON.stringify({ a: hs.artifacts, w: hs.warMachines }));
  }

  // ---------------------------------------------------------------- artifacts
  console.log('\nArtifacts');
  {
    // map keys artifact{n} are HoMM3 id 7 + (7n % 60): 0 Centaur's Axe, 1 Shield of the Yawning Dead,
    // 2 Helm of Chaos, 3 Tunic of the Cyclops King, 10 Shield of the Damned, 17 Statesman's Medal, 38 Celestial Necklace
    const carol = (await call('GET', `/api/games/${gameId}/heroes`, 'carol')).body[0];
    const give = keys => call('POST', `/api/heroes/${carol.id}/_test/artifacts`, null, keys).then(x => x.body);
    let h = await give(['artifact1']);
    check('a shield goes to the shield hand, not the head',
      h.worn.leftHand?.id === 14 && h.worn.leftHand.name === 'Shield of the Yawning Dead' && !h.worn.head && Object.keys(h.worn).length === 1,
      JSON.stringify(h.worn));
    check('shield adds its defense', h.defense === h.baseStats.defense + 3 && h.attack === h.baseStats.attack, JSON.stringify([h.attack, h.defense, h.baseStats]));
    h = await give(['artifact1', 'artifact2', 'artifact0', 'artifact3', 'artifact10']);
    check('helm on the head, axe in the weapon hand, tunic on the torso',
      h.worn.head?.id === 21 && h.worn.rightHand?.id === 7 && h.worn.torso?.id === 28 && h.worn.leftHand?.id === 14, JSON.stringify(h.worn));
    check('a second shield goes to the backpack', h.backpack.length === 1 && h.backpack[0].id === 17 && h.backpack[0].slot === 'leftHand',
      JSON.stringify(h.backpack));
    check('backpack artifacts give no bonus', h.defense === h.baseStats.defense + 3 && h.attack === h.baseStats.attack + 2
      && h.knowledge === h.baseStats.knowledge + 3 && h.power === h.baseStats.power + 4, JSON.stringify([h.attack, h.defense, h.power, h.knowledge, h.baseStats]));
    h = await give(['artifact17', 'artifact38']);
    check('two neck artifacts: one worn, one in the backpack', h.worn.neck?.id === 66 && h.backpack.length === 1 && h.backpack[0].id === 33,
      JSON.stringify({ w: h.worn, b: h.backpack }));
    h = await give([]);

    // walk carol's hero to the nearest artifact on the map and pick it up
    await call('POST', `/api/games/${gameId}/_test/explore`, 'carol');
    let picked = null, target = null;
    for (let day = 0; day < 20 && !picked; day++) {
      const m = (await call('GET', `/api/games/${gameId}/map`, 'carol')).body;
      const me = (await call('GET', `/api/games/${gameId}/heroes`, 'carol')).body[0];
      const W = m.width, H = m.height;
      const prev = new Map([[me.y * W + me.x, -1]]);
      let q = [me.y * W + me.x], goal = null;
      while (q.length && goal == null) {
        const nq = [];
        for (const k of q) {
          const cx = k % W, cy = Math.floor(k / W);
          for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
            const x = cx + dx, y = cy + dy;
            if (x < 0 || y < 0 || x >= W || y >= H || prev.has(y * W + x)) continue;
            const t = m.tiles[y * W + x];
            if (t.objectKind === 'artifact') { prev.set(y * W + x, k); goal = y * W + x; break; }
            if (!t.passable || t.objectKind || m.heroes.some(o => o.x === x && o.y === y)) continue;
            prev.set(y * W + x, k);
            nq.push(y * W + x);
          }
          if (goal != null) break;
        }
        q = nq;
      }
      if (goal == null) break;
      target = { x: goal % W, y: Math.floor(goal / W), key: m.tiles[goal].objectData };
      const path = [];
      for (let k = goal; k !== me.y * W + me.x; k = prev.get(k)) path.unshift({ x: k % W, y: Math.floor(k / W) });
      r = await call('POST', `/api/heroes/${carol.id}/move`, 'carol', { path });
      const msg = r.headers.get('x-move-message');
      if (msg && msg.startsWith('Found the')) picked = { msg, hero: r.body };
      else await call('POST', `/api/games/${gameId}/_test/nextday`);
    }
    if (picked) {
      const art = [...Object.values(picked.hero.worn), ...picked.hero.backpack].find(a => a.key === target.key);
      check('picking up names the artifact', art && picked.msg === `Found the ${art.name}.`, picked.msg);
      check('picked-up artifact is worn in its own slot', art && picked.hero.worn[art.slot === 'ring' ? 'ring1' : art.slot === 'misc' ? 'misc1' : art.slot]?.key === target.key,
        JSON.stringify(picked.hero.worn));
      const m = (await call('GET', `/api/games/${gameId}/map`, 'carol')).body;
      check('artifact removed from the map', m.tiles[target.y * m.width + target.x].objectKind !== 'artifact');
    } else {
      check('walk to a map artifact (none reachable on this map)', !target, JSON.stringify(target));
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