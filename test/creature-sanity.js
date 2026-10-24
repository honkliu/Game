// Sanity check: every shooter has shots, and no non-shooter has shots.
const fs = require('fs');
const path = require('path');

const src = fs.readFileSync(path.join(__dirname, '..', 'server', 'CityOfTowers', 'Domain', 'Creatures.cs'), 'utf8');
const re = /Add\(new\("(\w+)",\s*"([^"]+)",\s*"(\w+)",\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+),\s*\[([^\]]*)\]/g;

let m;
const bad = [];
let count = 0;
while ((m = re.exec(src)) !== null) {
  count++;
  const key = m[1];
  const shots = +m[19];
  const abilities = m[21];
  const isShooter = abilities.includes('shooter');
  if (isShooter && shots === 0) bad.push(key + ' (shooter with 0 shots)');
  if (!isShooter && shots > 0) bad.push(key + ' (shots but not shooter)');
}

console.log('checked ' + count + ' creatures');
if (bad.length) {
  console.log('PROBLEMS:');
  for (const b of bad) console.log('  - ' + b);
  process.exit(1);
}
console.log('all shooters have shots; no non-shooters have shots');