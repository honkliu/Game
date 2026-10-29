// Extract numeric facts only. Original game text and artwork are never copied.
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');

const archive = fs.readFileSync(path.join(process.env.LOCALAPPDATA, 'CityOfTowers', 'h3data', 'H3BITMAP.LOD'));
function table(name) {
  for (let i = 0; i < archive.readUInt32LE(8); i++) {
    const entry = 92 + i * 32;
    if (archive.toString('latin1', entry, entry + 16).replace(/\0.*/, '').toUpperCase() !== name) continue;
    const start = archive.readUInt32LE(entry + 16), size = archive.readUInt32LE(entry + 20), packed = archive.readUInt32LE(entry + 28);
    const bytes = archive.subarray(start, start + (packed || size));
    const text = (packed ? zlib.inflateSync(bytes) : bytes).toString('latin1');
    const rows = [];
    let row = [], field = '', quoted = false;
    for (let n = 0; n < text.length; n++) {
      const c = text[n];
      if (c === '"') {
        if (quoted && text[n + 1] === '"') { field += '"'; n++; } else quoted = !quoted;
      } else if (c === '\t' && !quoted) { row.push(field); field = ''; }
      else if (c === '\n' && !quoted) { row.push(field.replace(/\r$/, '')); rows.push(row); row = []; field = ''; }
      else field += c;
    }
    return rows;
  }
  throw new Error(`Missing native table ${name}`);
}

const skillOrder = ['pathfinding', 'archery', 'logistics', 'scouting', 'diplomacy', 'navigation', 'leadership',
  'wisdom', 'mysticism', 'luck', 'ballistics', 'eagleEye', 'necromancy', 'estates', 'fire', 'air', 'water',
  'earth', 'scholar', 'tactics', 'artillery', 'learning', 'offence', 'armorer', 'intelligence', 'sorcery', 'resistance', 'firstAid'];
const classes = table('HCTRAITS.TXT').filter(r => r.length >= 42 && /^\d+\.\d+$/.test(r[1])).map(r => ({
  name: r[0], initial: r.slice(2, 6).map(Number), low: r.slice(6, 10).map(Number), high: r.slice(10, 14).map(Number),
  secondary: Object.fromEntries(skillOrder.map((key, i) => [key, Number(r[14 + i])]))
}));
const spells = table('SPTRAITS.TXT').filter(r => r[0] && /^[1-5]$/.test(r[2]) && r.length > 30).map(r => ({
  name: r[0], level: Number(r[2]), schools: ['earth', 'water', 'fire', 'air'].filter((_, i) => r[3 + i].trim() === 'x'),
  mana: r.slice(7, 11).map(Number), chance: r.slice(16, 25).map(Number)
}));
const creatures = table('CRTRAITS.TXT').filter(r => r[0] && r[1] && /^\d+$/.test(r[8]) && r.length >= 22).map(r => ({
  name: r[0], cost: r.slice(2, 9).map(Number), growth: Number(r[11]), hp: Number(r[13]), speed: Number(r[14]),
  attack: Number(r[15]), defense: Number(r[16]), damage: r.slice(17, 19).map(Number)
}));
let section = '';
const buildings = [];
for (const row of table('BUILDING.TXT')) {
  if (row[0] && !/^\d+$/.test(row[0])) section = row[0];
  if (/^\d+$/.test(row[0]) && row[7]?.trim()) buildings.push({ section, name: row[7].trim(), cost: row.slice(0, 7).map(Number) });
}
if (classes.length !== 18 || spells.length < 60 || creatures.length < 126) throw new Error('Incomplete original numeric tables.');
async function main() {
  const heroes = {};
  const revision = 'abe535c4418b66bae8cdb56fee1473b109e8c55e';
  const names = table('HOTRAITS.TXT').slice(2);
  for (const faction of ['castle', 'rampart', 'tower', 'inferno', 'necropolis', 'dungeon', 'stronghold', 'fortress', 'conflux']) {
    const response = await fetch(`https://raw.githubusercontent.com/vcmi/vcmi/${revision}/config/heroes/${faction}.json`);
    if (!response.ok) throw new Error(`Hero reference ${faction}: HTTP ${response.status}`);
    const roster = JSON.parse((await response.text()).replace(/^\s*\/\/.*$/gm, ''));
    for (const [key, hero] of Object.entries(roster)) {
      if (!Number.isInteger(hero.index) || !hero.class || !hero.skills) continue;
      heroes[key.toLowerCase()] = {
        faction, name: names[hero.index]?.[0] ?? key, class: hero.class,
        skills: Object.fromEntries(hero.skills.map(s => [s.skill, ['none', 'basic', 'advanced', 'expert'].indexOf(s.level)])),
        spells: hero.spellbook ?? []
      };
    }
  }
  const data = { source: `Installed Heroes III HCTRAITS, SPTRAITS, CRTRAITS and BUILDING numeric tables; VCMI ${revision} hero class/starting skills. No descriptions or artwork.`,
    classes, heroes, spells, creatures, buildings };
  const destination = path.join(__dirname, '..', 'server', 'CityOfTowers', 'Domain', 'h3-rules.json');
  fs.writeFileSync(destination, JSON.stringify(data, null, 2) + '\n');
  console.log(`Extracted ${classes.length} classes, ${Object.keys(heroes).length} hero profiles, ${spells.length} spells, ${creatures.length} creatures and ${buildings.length} building prices.`);
}
main().catch(error => { console.error(error); process.exitCode = 1; });
