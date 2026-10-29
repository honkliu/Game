// Run with node test\movement.js; fixtures do not use the running server or play database.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { spawnSync } = require('node:child_process');
const ts = require('../client/node_modules/typescript');

const source = fs.readFileSync(path.join(__dirname, '..', 'client', 'src', 'views', 'AdventureMap.tsx'), 'utf8');
const parsed = ts.createSourceFile('AdventureMap.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const findPathNode = parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'findPath');
const costNode = parsed.statements.find(node => ts.isVariableStatement(node)
  && node.declarationList.declarations.some(declaration => declaration.name.getText(parsed) === 'terrainCost'));
assert.ok(findPathNode && costNode, 'Client movement functions must exist');
const interactionFunctions = ['stepCost', 'targetTile', 'reachableSteps', 'adventureCursor', 'adventureCursorCss'].map(name =>
  parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name).getText(parsed)).join('\n');
const compiled = ts.transpileModule(`${costNode.getText(parsed)}\n${findPathNode.getText(parsed)}\n${interactionFunctions}`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 }
}).outputText;
const context = vm.createContext({});
vm.runInContext(compiled, context);

function map() {
  return {
    width: 5, height: 5, heroes: [{ x: 1, y: 1 }], towns: [],
    tiles: Array.from({ length: 25 }, () => ({ terrain: 'grass', passable: true, fog: 2, objectKind: null }))
  };
}
for (const kind of ['block', 'town', 'mine', 'monster', 'obelisk', 'shrine', 'tree']) {
  const m = map();
  m.tiles[7] = { ...m.tiles[7], passable: false, objectKind: kind };
  const route = context.findPath(m, 1, 1, 2, 2);
  assert.ok(route && route.length > 2, `Client must detour around ${kind} corner`);
  m.tiles[7].passable = true;
  assert.ok(context.findPath(m, 1, 1, 2, 2).length > 2, `Stale passability must not permit cutting through ${kind}`);
  console.log(`PASS client detours around ${kind} corner`);
}
{
  const m = map();
  m.heroes.push({ x: 2, y: 1 });
  assert.ok(context.findPath(m, 1, 1, 2, 2).length > 2, 'Client must detour around another hero');
  console.log('PASS client detours around occupied corners');
}
{
  const m = map();
  m.tiles[7] = { ...m.tiles[7], passable: false, objectKind: 'town' };
  const route = context.findPath(m, 1, 1, 3, 1);
  assert.ok(route && route.every(p => p.x !== 2 || p.y !== 1), 'Town cannot be a transit tile');
  assert.equal(context.findPath(m, 1, 1, 2, 1).length, 2, 'Town gate remains a visitable destination');
  assert.equal(context.findPath(map(), 1, 1, 2, 2).length, 2, 'Open diagonal remains allowed');
  console.log('PASS client routes around towns but can visit gates and use open diagonals');
}
{
  const m = map();
  m.tiles[7] = { ...m.tiles[7], passable: false, objectKind: 'shrine' };
  assert.equal(context.findPath(m, 1, 1, 2, 1).length, 2, 'Shrine remains a visitable destination');
  assert.ok(context.findPath(m, 1, 1, 3, 1).every(p => p.x !== 2 || p.y !== 1), 'Shrine is not a transit tile');
  console.log('PASS client visits shrines without routing through them');
}
{
  assert.equal(vm.runInContext("terrainCost('sand')", context), 150);
  for (const [level, cost] of [[0, 175], [1, 150], [2, 125], [3, 100]]) {
    assert.equal(vm.runInContext(`terrainCost('swamp', ${level})`, context), cost);
    assert.equal(vm.runInContext(`terrainCost('grass', ${level})`, context), 100);
  }
  console.log('PASS client terrain costs include sand and all Pathfinding levels');
}
for (const kind of ['town', 'monster', 'mine', 'block', 'tree']) {
  const m = map();
  m.tiles[7] = { ...m.tiles[7], passable: false, objectKind: kind };
  const ground = context.findPath(m, 1, 1, 3, 1);
  const flight = context.findPath(m, 1, 1, 3, 1, 0, 40);
  assert.ok(ground.every(p => p.x !== 2 || p.y !== 1), `Walking cannot cross ${kind}`);
  assert.equal(flight.length, 3, `Flight crosses ${kind} directly`);
  assert.equal(context.findPath(m, 1, 1, 3, 1).length, ground.length, 'Flight must not change later ground routing');
  console.log(`PASS client flight crosses ${kind} without weakening walking collision`);
}
{
  const m = map(), path = [{ x: 1, y: 1 }, { x: 2, y: 1 }];
  m.tiles[6].terrain = 'snow';
  assert.equal(context.reachableSteps(m, path, 125), 0, 'Leaving snow must cost the source terrain');
  m.tiles[6].terrain = 'grass'; m.tiles[7].terrain = 'snow';
  assert.equal(context.reachableSteps(m, path, 100), 1, 'Entering snow from grass costs 100');
  assert.equal(context.stepCost('swamp', true, 1500), 247);
  assert.equal(context.stepCost('snow', true, 1500, 0, 'snow'), 141);
  assert.equal(context.stepCost('grass', true, 125), 125);
  assert.equal(context.stepCost('grass', true, 99), 141);
  console.log('PASS client source-terrain, native-terrain, diagonal and last-step charges');
}
{
  const m = map();
  for (let y = 0; y < 5; y++) m.tiles[y * 5 + 2] = { ...m.tiles[y * 5 + 2], terrain: 'water', passable: false };
  const hero = { movement: 140, maxMovement: 1500, hasteBonus: 0, nativeTerrain: 'grass' };
  assert.equal(context.findPath(m, 1, 1, 3, 1, 0, 40, hero), null,
    'Tomorrow cannot continue flying across a river when no legal landing can be reached today');
  hero.movement = 280;
  assert.equal(context.findPath(m, 1, 1, 3, 1, 0, 40, hero).length, 3, 'A complete same-day flight remains legal');
  const land = map();
  land.tiles[7] = { ...land.tiles[7], objectKind: 'tree', passable: false };
  hero.movement = 0;
  assert.ok(context.findPath(land, 1, 1, 3, 1, 0, 40, hero).every(p => p.x !== 2 || p.y !== 1),
    'An exhausted Fly route must detour on foot tomorrow');
  console.log('PASS future-day flight expiry replans on ground and never strands a hero over water');
}
for (const kind of ['resource', 'treasure', 'artifact', 'monster', 'mine', 'town', 'shrine', 'obelisk', 'grail']) {
  const m = map();
  m.heroes[0].id = 'selected';
  m.tiles[7] = { ...m.tiles[7], objectKind: kind, objectData: kind === 'resource' ? 'gold:500' : null,
    passable: ['resource', 'treasure', 'artifact', 'grail'].includes(kind) };
  const hero = { id: 'selected', x: 1, y: 1, movement: 100, maxMovement: 1500, hasteBonus: 0, skills: {}, flying: false };
  const cursor = context.adventureCursor(m, hero, 2, 1);
  assert.equal(cursor.action, 'visit', `${kind} must use a visit action, not ordinary walking`);
  assert.equal(cursor.frame, 9, `${kind} must use the native visiting horse`);
  hero.movement = 0;
  if (kind !== 'town') assert.equal(context.adventureCursor(m, hero, 2, 1).frame, 15, `${kind} must show next-day visit when movement is exhausted`);
  if (!m.tiles[7].passable) {
    hero.movement = 100; hero.flying = true; hero.flightPenalty = 40;
    assert.equal(context.adventureCursor(m, hero, 2, 1).frame, 9, `Adjacent ${kind} visit costs ground movement even while flying`);
  }
  console.log(`PASS ${kind} destination uses native visit marker and movement reachability`);
}
{
  const m = map();
  m.heroes[0].id = 'selected';
  const hero = { id: 'selected', x: 1, y: 1, movement: 100, maxMovement: 100, hasteBonus: 0, skills: {}, flying: false };
  assert.equal(context.adventureCursor(m, hero, 2, 1).frame, 4, 'Open ground uses walking horse');
  assert.equal(context.adventureCursor(m, hero, 4, 1).frame, 16, 'Third-day path uses native day-three walking horse');
  hero.movement = 0;
  assert.equal(context.adventureCursor(m, hero, 4, 1).frame, 22, 'Fourth-day path uses native day-four walking horse');
  m.heroes.push({ id: 'friend', x: 2, y: 1 });
  assert.equal(context.adventureCursor(m, hero, 2, 1).frame, 8, 'Adjacent meeting does not spend a fictitious movement step');
  m.tiles[7].fog = 0;
  assert.equal(context.adventureCursor(m, hero, 2, 1).frame, 0, 'Unexplored objects must not leak a visit/meeting cursor');
  m.tiles[7] = { ...m.tiles[7], fog: 2, objectKind: 'block', objectData: '2,2', passable: false };
  m.tiles[12] = { ...m.tiles[12], objectKind: 'mine', passable: false };
  hero.movement = hero.maxMovement = 1000;
  assert.equal(context.adventureCursor(m, hero, 2, 1).frame, 9, 'Clicking a building body targets its entrance');
  m.tiles[12].objectKind = 'tree';
  assert.equal(context.adventureCursor(m, hero, 2, 1).action, 'blocked', 'Obstacle is not visitable');
  console.log('PASS native walking, future-day, meeting, fog and building-body cursor decisions');
}
{
  const geometry = require('../client/src/h3/object-geometry.json');
  const catalog = fs.readFileSync(path.join(__dirname, '..', 'client', 'src', 'h3', 'catalog.ts'), 'utf8')
    .replace(/^import .*$/gm, '').replace(/^export /gm, '');
  const visual = vm.createContext({ objectGeometry: geometry, draws: [] });
  vm.runInContext(ts.transpileModule(catalog, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, visual);
  const names = ['objectSprite', 'anchorDraw', 'pick', 'hash'];
  const functions = names.map(name => parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name).getText(parsed)).join('\n');
  vm.runInContext(ts.transpileModule(`const TILE=32; function drawFrame(...args) { draws.push(args); }\n${functions}`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 }
  }).outputText, visual);
  for (const [kind, data, width, visitX, sprite] of [
    ['shrine', 'magicArrow', 64, 0], ['obelisk', null, 64, 0],
    ...['wood', 'ore', 'gold', 'mercury', 'sulfur', 'crystal', 'gems'].map(resource => ['mine', resource, 160, 1]),
    ...['wood', 'ore', 'gold', 'mercury', 'sulfur', 'crystal', 'gems'].map(resource => ['resource', `${resource}:5`, 64, 0]),
    ['treasure', '1500:1000', 64, 0], ['artifact', 'artifact1', 64, 0], ['monster', 'pikeman:5', 64, 0],
    ['tree', null, 64, 1, 'AVLtr1d0'], ['tree', null, 64, 1, 'AVLtr2d0']
  ]) {
    visual.tile = { objectKind: kind, objectData: data, terrain: 'grass', sprite };
    visual.width = width;
    const position = vm.runInContext(`(() => { const s=objectSprite(tile,4,4); anchorDraw({}, {w:width,h:64},0,128,128,s.ox); return draws.at(-1).slice(3,5); })()`, visual);
    assert.equal(position[0], 128 + (1 + visitX) * 32 - width, `${kind} art must align with its visit tile`);
    console.log(`PASS actual client drawing aligns ${kind} with its collision/interaction tile`);
  }
}
const result = spawnSync('dotnet', ['run', '--project', path.join(__dirname, 'MovementChecks'), '--verbosity', 'quiet'], {
  stdio: 'inherit'
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
