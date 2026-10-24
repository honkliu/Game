// Verify the creature catalogue: 9 factions, 7 base tiers each, all with upgrades.
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const src = fs.readFileSync(path.join(__dirname, '..', 'server', 'CityOfTowers', 'Domain', 'Creatures.cs'), 'utf8');

// Extract every Add(new("key", "Name", "faction", tier, ...)) call.
const re = /Add\(new\("(\w+)",\s*"([^"]+)",\s*"(\w+)",\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*\[([^\]]*)\](?:,\s*"(\w+)")?\)\)/g;

const creatures = [];
let m;
while ((m = re.exec(src)) !== null) {
  creatures.push({
    key: m[1], name: m[2], faction: m[3], tier: +m[4],
    atk: +m[5], def: +m[6], dmgMin: +m[7], dmgMax: +m[8], hp: +m[9], speed: +m[10],
    growth: +m[11], gold: +m[12], shots: +m[19], ai: +m[20],
    abilities: m[21].trim(), upgradeOf: m[22] || null
  });
}

console.log('parsed creatures: ' + creatures.length);

const factions = [...new Set(creatures.map(c => c.faction))].sort();
console.log('factions: ' + factions.length + ' — ' + factions.join(', '));

let fail = 0;
function check(name, cond, detail) {
  if (cond) console.log('  ok   ' + name);
  else { fail++; console.log('  FAIL ' + name + (detail ? ' — ' + detail : '')); }
}

check('9 factions', factions.length === 9, factions.length);

for (const f of factions) {
  const base = creatures.filter(c => c.faction === f && !c.upgradeOf);
  const upg = creatures.filter(c => c.faction === f && c.upgradeOf);
  const tiers = base.map(c => c.tier).sort((a, b) => a - b);
  check(f + ': 7 base tiers', base.length === 7, base.length);
  check(f + ': tiers 1-7', JSON.stringify(tiers) === JSON.stringify([1, 2, 3, 4, 5, 6, 7]), JSON.stringify(tiers));
  check(f + ': 7 upgrades', upg.length === 7, upg.length);
  // every upgrade points at a real base creature of the same faction
  const bad = upg.filter(u => {
    const b = creatures.find(c => c.key === u.upgradeOf);
    return !b || b.faction !== f;
  });
  check(f + ': upgrades resolve', bad.length === 0, bad.map(b => b.key).join(','));
}

// spot-check real HoMM3 values
const spot = [
  ['pikeman', { atk: 4, def: 5, dmgMin: 1, dmgMax: 3, hp: 10, speed: 4, growth: 14, gold: 60 }],
  ['archer', { atk: 6, def: 3, dmgMin: 2, dmgMax: 3, hp: 10, speed: 4, growth: 9, gold: 100, shots: 12 }],
  ['angel', { atk: 20, def: 20, dmgMin: 50, dmgMax: 50, hp: 200, speed: 12, growth: 1, gold: 3000 }],
  ['archangel', { atk: 30, def: 30, hp: 250, speed: 18, gold: 5000 }],
  ['skeleton', { atk: 5, def: 4, hp: 6, speed: 4, growth: 12, gold: 60 }],
  ['gnoll', { atk: 3, def: 5, dmgMin: 2, dmgMax: 3, hp: 6, growth: 12 }],
  ['centaur', { hp: 8, atk: 5, def: 3, speed: 6, growth: 14 }],
  ['gremlin', { atk: 3, def: 3, hp: 4, growth: 16, gold: 30 }],
  ['blackDragon', { atk: 25, def: 25, hp: 300, speed: 15, gold: 4000 }],
  ['titan', { atk: 24, def: 24, hp: 300, speed: 11, gold: 5000, shots: 24 }],
  ['phoenix', { atk: 21, def: 18, hp: 200, speed: 21, growth: 2 }]
];
for (const [key, expect] of spot) {
  const c = creatures.find(x => x.key === key);
  if (!c) { check('spot ' + key, false, 'missing'); continue; }
  const bad = Object.entries(expect).filter(([k, v]) => c[k] !== v);
  check('spot ' + key, bad.length === 0, bad.map(([k, v]) => `${k}=${c[k]} expected ${v}`).join(', '));
}

console.log('\n' + (fail === 0 ? 'ALL PASS' : fail + ' FAILURES'));
process.exit(fail === 0 ? 0 : 1);