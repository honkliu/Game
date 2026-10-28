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
const compiled = ts.transpileModule(`${costNode.getText(parsed)}\n${findPathNode.getText(parsed)}`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 }
}).outputText;
const context = vm.createContext({});
vm.runInContext(compiled, context);

function map() {
  return {
    width: 5, height: 5, heroes: [{ x: 1, y: 1 }],
    tiles: Array.from({ length: 25 }, () => ({ terrain: 'grass', passable: true, fog: 2, objectKind: null }))
  };
}
for (const kind of ['block', 'town', 'mine', 'monster', 'obelisk', 'shrine', 'tree']) {
  const m = map();
  m.tiles[7] = { ...m.tiles[7], passable: false, objectKind: kind };
  const route = context.findPath(m, 1, 1, 2, 2);
  assert.ok(route && route.length > 2, `Client must detour around ${kind} corner`);
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
const result = spawnSync('dotnet', ['run', '--project', path.join(__dirname, 'MovementChecks'), '--verbosity', 'quiet'], {
  stdio: 'inherit'
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
