// Building sprite generator: emits 160x160 SVG files for every building.
// Vector shapes are used so the geometry is exact and inspectable in source.
const fs = require('fs');
const path = require('path');

const OUT = path.join(__dirname, '..', 'client', 'assets', 'buildings');
if (!fs.existsSync(OUT)) fs.mkdirSync(OUT, { recursive: true });

const W = 160, H = 160;

// ---------------------------------------------------------------- helpers
const svg = (body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">\n${body}\n</svg>\n`;

const shadow = (cx = 80, cy = 148, rx = 52, ry = 9) =>
  `<ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="rgba(0,0,0,0.45)"/>`;

const grad = (id, stops, x1 = 0, y1 = 0, x2 = 0, y2 = 1) =>
  `<linearGradient id="${id}" x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}">` +
  stops.map(([o, c]) => `<stop offset="${o}" stop-color="${c}"/>`).join('') +
  `</linearGradient>`;

const stone = (id, a = '#b0a694', b = '#6a6258') => grad(id, [[0, a], [0.5, '#8a8070'], [1, b]]);
const roof = (id, a = '#c05030', b = '#6a2410') => grad(id, [[0, a], [1, b]]);
const wood = (id) => grad(id, [[0, '#a07a40'], [1, '#5a3a18']]);

// A simple pitched roof: apex at (cx, top), eaves at (cx±half, bottom).
const pitched = (cx, top, half, bottom, fill, stroke = '#2a1a10') =>
  `<path d="M${cx - half} ${bottom} L${cx} ${top} L${cx + half} ${bottom} Z" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>`;

// A crenellated wall top.
const crenels = (x, y, w, n, fill) => {
  const step = w / n;
  let d = `M${x} ${y}`;
  for (let i = 0; i < n; i++) {
    const bx = x + i * step;
    d += ` L${bx} ${y} L${bx} ${y - 6} L${bx + step * 0.6} ${y - 6} L${bx + step * 0.6} ${y}`;
  }
  d += ` L${x + w} ${y} Z`;
  return `<path d="${d}" fill="${fill}" stroke="#2a1a10" stroke-width="1"/>`;
};

// ---------------------------------------------------------------- sprites
const sprites = {};

// Town Hall — the civic centre: wide hall, tall clock tower, banner.
sprites.townHall = svg(`
<defs>${stone('s1')}${roof('r1', '#3a6ad0', '#1a2a60')}${wood('w1')}</defs>
${shadow()}
<rect x="26" y="86" width="108" height="58" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${crenels(26, 86, 108, 7, '#9a9080')}
<rect x="66" y="34" width="28" height="54" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${pitched(80, 12, 20, 34, 'url(#r1)')}
<circle cx="80" cy="52" r="8" fill="#f0e0b0" stroke="#2a1a10" stroke-width="1.5"/>
<line x1="80" y1="52" x2="80" y2="47" stroke="#2a1a10" stroke-width="1.5"/>
<line x1="80" y1="52" x2="84" y2="54" stroke="#2a1a10" stroke-width="1.5"/>
<rect x="72" y="112" width="16" height="32" fill="#241a10" stroke="#2a1a10" stroke-width="1"/>
<path d="M72 112 A8 8 0 0 1 88 112 Z" fill="#241a10"/>
<rect x="36" y="100" width="12" height="16" fill="#1a1410" stroke="#2a1a10" stroke-width="1"/>
<rect x="112" y="100" width="12" height="16" fill="#1a1410" stroke="#2a1a10" stroke-width="1"/>
<rect x="78" y="20" width="4" height="14" fill="#3a6ad0"/>
<rect x="82" y="20" width="10" height="7" fill="#3a6ad0"/>
`);

// Woodcutter's Lodge — log cabin with a lean-to and stacked logs.
sprites.woodcutter = svg(`
<defs>${wood('w1')}${roof('r1', '#6a4a28', '#3a2410')}</defs>
${shadow()}
<rect x="34" y="80" width="76" height="64" fill="url(#w1)" stroke="#2a1a10" stroke-width="1.5"/>
<line x1="34" y1="92" x2="110" y2="92" stroke="#3a2410" stroke-width="1"/>
<line x1="34" y1="104" x2="110" y2="104" stroke="#3a2410" stroke-width="1"/>
<line x1="34" y1="116" x2="110" y2="116" stroke="#3a2410" stroke-width="1"/>
<line x1="34" y1="128" x2="110" y2="128" stroke="#3a2410" stroke-width="1"/>
${pitched(72, 44, 46, 80, 'url(#r1)')}
<rect x="62" y="108" width="20" height="36" fill="#2a1a0c" stroke="#1a1008" stroke-width="1"/>
<rect x="88" y="96" width="12" height="12" fill="#e8c060" stroke="#2a1a10" stroke-width="1"/>
<rect x="118" y="118" width="30" height="26" fill="#8a5a2b" stroke="#2a1a10" stroke-width="1.5"/>
<circle cx="124" cy="124" r="4" fill="#c8a070"/>
<circle cx="136" cy="124" r="4" fill="#c8a070"/>
<circle cx="130" cy="134" r="4" fill="#c8a070"/>
<rect x="20" y="128" width="14" height="16" fill="#6a4a20" stroke="#2a1a10" stroke-width="1"/>
<line x1="20" y1="134" x2="34" y2="134" stroke="#3a2410" stroke-width="1"/>
`);

// Ore Pit — a mine entrance cut into a rock face with a timber frame.
sprites.orePit = svg(`
<defs>${stone('s1', '#8a8a92', '#4a4a52')}${wood('w1')}</defs>
${shadow(80, 150, 56, 8)}
<path d="M18 146 L34 60 L80 34 L126 60 L142 146 Z" fill="url(#s1)" stroke="#2a2a32" stroke-width="1.5"/>
<path d="M80 34 L126 60 L142 146 L80 146 Z" fill="rgba(0,0,0,0.22)"/>
<path d="M56 146 L56 96 A24 24 0 0 1 104 96 L104 146 Z" fill="#0e0a08" stroke="#1a1410" stroke-width="1.5"/>
<rect x="50" y="88" width="8" height="58" fill="url(#w1)" stroke="#2a1a10" stroke-width="1"/>
<rect x="102" y="88" width="8" height="58" fill="url(#w1)" stroke="#2a1a10" stroke-width="1"/>
<rect x="46" y="82" width="68" height="8" fill="url(#w1)" stroke="#2a1a10" stroke-width="1"/>
<circle cx="80" cy="52" r="7" fill="#9a9aa2" stroke="#2a2a32" stroke-width="1.5"/>
<circle cx="78" cy="50" r="2.5" fill="#c8c8d0"/>
<rect x="112" y="120" width="18" height="12" fill="#6a6a72" stroke="#2a2a32" stroke-width="1"/>
`);

// Alchemist's Lab — a crooked tower with bubbling flasks and green smoke.
sprites.alchemist = svg(`
<defs>${stone('s1', '#a8a0b8', '#5a5268')}${roof('r1', '#7a4aa0', '#3a1a50')}</defs>
${shadow()}
<rect x="46" y="70" width="68" height="74" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${pitched(80, 30, 40, 70, 'url(#r1)')}
<rect x="70" y="96" width="20" height="48" fill="#2a1a30" stroke="#1a1018" stroke-width="1"/>
<circle cx="60" cy="86" r="7" fill="#60e0a0" opacity="0.85" stroke="#1a1018" stroke-width="1"/>
<circle cx="100" cy="86" r="7" fill="#60c0e0" opacity="0.85" stroke="#1a1018" stroke-width="1"/>
<path d="M60 79 q-6 -12 2 -20 q8 -8 2 -18" stroke="rgba(120,255,180,0.5)" stroke-width="3" fill="none"/>
<path d="M100 79 q6 -12 -2 -20 q-8 -8 -2 -18" stroke="rgba(120,200,255,0.5)" stroke-width="3" fill="none"/>
<rect x="118" y="118" width="22" height="26" fill="#5a5268" stroke="#2a1a10" stroke-width="1.5"/>
<circle cx="129" cy="112" r="6" fill="#e0c040" stroke="#2a1a10" stroke-width="1"/>
`);

// Sulfur Dune — a yellow mound with crystal shards.
sprites.sulfurDune = svg(`
<defs>${grad('d1', [[0, '#e8d060'], [1, '#8a7020']])}</defs>
${shadow(80, 150, 58, 8)}
<path d="M14 146 Q50 70 80 62 Q112 70 146 146 Z" fill="url(#d1)" stroke="#5a4a10" stroke-width="1.5"/>
<path d="M80 62 Q112 70 146 146 L80 146 Z" fill="rgba(0,0,0,0.18)"/>
<path d="M62 100 L70 74 L78 100 Z" fill="#f0e080" stroke="#5a4a10" stroke-width="1"/>
<path d="M86 96 L96 66 L106 96 Z" fill="#f0e080" stroke="#5a4a10" stroke-width="1"/>
<path d="M44 128 L50 110 L56 128 Z" fill="#e8d060" stroke="#5a4a10" stroke-width="1"/>
<circle cx="120" cy="120" r="4" fill="#f0e080"/>
<circle cx="36" cy="136" r="3" fill="#f0e080"/>
`);

// Crystal Cavern — a cave mouth ringed with blue crystals.
sprites.crystalCavern = svg(`
<defs>${stone('s1', '#7a7a8a', '#3a3a48')}${grad('c1', [[0, '#c0f0ff'], [1, '#2a80b0']])}</defs>
${shadow(80, 150, 56, 8)}
<path d="M16 146 L28 70 L80 40 L132 70 L144 146 Z" fill="url(#s1)" stroke="#2a2a32" stroke-width="1.5"/>
<path d="M52 146 L52 104 A28 28 0 0 1 108 104 L108 146 Z" fill="#0a0a12" stroke="#1a1a24" stroke-width="1.5"/>
<path d="M40 96 L48 62 L56 96 Z" fill="url(#c1)" stroke="#1a4a60" stroke-width="1"/>
<path d="M104 96 L112 58 L120 96 Z" fill="url(#c1)" stroke="#1a4a60" stroke-width="1"/>
<path d="M70 60 L78 30 L86 60 Z" fill="url(#c1)" stroke="#1a4a60" stroke-width="1"/>
<path d="M24 130 L30 108 L36 130 Z" fill="url(#c1)" stroke="#1a4a60" stroke-width="1"/>
<path d="M124 130 L130 106 L136 130 Z" fill="url(#c1)" stroke="#1a4a60" stroke-width="1"/>
`);

// Gem Pond — a pool with gemstones on the bank.
sprites.gemPond = svg(`
<defs>${grad('p1', [[0, '#60c0e0'], [1, '#1a4a70']])}${grad('g1', [[0, '#a0ffb0'], [1, '#208040']])}</defs>
${shadow(80, 150, 58, 8)}
<ellipse cx="80" cy="118" rx="58" ry="30" fill="#3a5a30" stroke="#1a2a14" stroke-width="1.5"/>
<ellipse cx="80" cy="118" rx="44" ry="22" fill="url(#p1)" stroke="#0e2a40" stroke-width="1.5"/>
<ellipse cx="66" cy="112" rx="12" ry="5" fill="rgba(200,240,255,0.45)"/>
<path d="M40 96 L48 78 L56 96 Z" fill="url(#g1)" stroke="#0e3a18" stroke-width="1"/>
<path d="M104 96 L112 76 L120 96 Z" fill="url(#g1)" stroke="#0e3a18" stroke-width="1"/>
<path d="M72 88 L80 70 L88 88 Z" fill="url(#g1)" stroke="#0e3a18" stroke-width="1"/>
<circle cx="128" cy="128" r="4" fill="#a0ffb0"/>
<circle cx="32" cy="130" r="3" fill="#a0ffb0"/>
`);

// Marketplace — stalls with striped awnings.
sprites.marketplace = svg(`
<defs>${wood('w1')}${grad('a1', [[0, '#e05040'], [1, '#8a2010']])}${grad('a2', [[0, '#e8c040'], [1, '#8a6a10']])}</defs>
${shadow()}
<rect x="20" y="96" width="56" height="48" fill="url(#w1)" stroke="#2a1a10" stroke-width="1.5"/>
<rect x="84" y="96" width="56" height="48" fill="url(#w1)" stroke="#2a1a10" stroke-width="1.5"/>
<path d="M14 96 L48 66 L82 96 Z" fill="url(#a1)" stroke="#2a1a10" stroke-width="1.5"/>
<path d="M78 96 L112 66 L146 96 Z" fill="url(#a2)" stroke="#2a1a10" stroke-width="1.5"/>
<line x1="48" y1="66" x2="48" y2="96" stroke="#2a1a10" stroke-width="1"/>
<line x1="112" y1="66" x2="112" y2="96" stroke="#2a1a10" stroke-width="1"/>
<rect x="30" y="112" width="36" height="8" fill="#8a5a2b" stroke="#2a1a10" stroke-width="1"/>
<rect x="94" y="112" width="36" height="8" fill="#8a5a2b" stroke="#2a1a10" stroke-width="1"/>
<circle cx="40" cy="108" r="4" fill="#e05040"/>
<circle cx="52" cy="108" r="4" fill="#e8c040"/>
<circle cx="104" cy="108" r="4" fill="#40c060"/>
<circle cx="116" cy="108" r="4" fill="#60c0e0"/>
`);

// Tavern — a cosy inn with a hanging sign and warm windows.
sprites.tavern = svg(`
<defs>${stone('s1', '#b8a080', '#6a5030')}${roof('r1', '#8a4a20', '#3a1a08')}${wood('w1')}</defs>
${shadow()}
<rect x="30" y="76" width="100" height="68" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${pitched(80, 40, 56, 76, 'url(#r1)')}
<rect x="66" y="106" width="28" height="38" fill="#3a2410" stroke="#1a1008" stroke-width="1.5"/>
<rect x="40" y="92" width="16" height="16" fill="#ffd060" stroke="#2a1a10" stroke-width="1"/>
<rect x="104" y="92" width="16" height="16" fill="#ffd060" stroke="#2a1a10" stroke-width="1"/>
<rect x="118" y="60" width="4" height="20" fill="#3a2410"/>
<rect x="110" y="80" width="24" height="16" fill="url(#w1)" stroke="#2a1a10" stroke-width="1"/>
<circle cx="122" cy="88" r="4" fill="#e8c040"/>
<rect x="60" y="30" width="10" height="18" fill="#6a5030" stroke="#2a1a10" stroke-width="1"/>
<path d="M64 26 q-6 -8 0 -14 q6 -6 0 -12" stroke="rgba(200,200,200,0.4)" stroke-width="3" fill="none"/>
`);

// Granary — a round silo with a domed roof and sacks.
sprites.granary = svg(`
<defs>${grad('s1', [[0, '#d8c090'], [1, '#8a7040']])}${roof('r1', '#a08040', '#4a3410')}</defs>
${shadow()}
<rect x="44" y="70" width="72" height="74" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
<path d="M40 70 A40 26 0 0 1 120 70 Z" fill="url(#r1)" stroke="#2a1a10" stroke-width="1.5"/>
<line x1="44" y1="88" x2="116" y2="88" stroke="#6a5030" stroke-width="1"/>
<line x1="44" y1="106" x2="116" y2="106" stroke="#6a5030" stroke-width="1"/>
<line x1="44" y1="124" x2="116" y2="124" stroke="#6a5030" stroke-width="1"/>
<rect x="72" y="112" width="16" height="32" fill="#3a2410" stroke="#1a1008" stroke-width="1"/>
<ellipse cx="30" cy="136" rx="12" ry="10" fill="#c8b080" stroke="#2a1a10" stroke-width="1"/>
<ellipse cx="130" cy="136" rx="12" ry="10" fill="#c8b080" stroke="#2a1a10" stroke-width="1"/>
`);

// Treasury — a squat vault with a heavy door and gold trim.
sprites.treasury = svg(`
<defs>${stone('s1', '#a8a8b0', '#5a5a64')}${grad('g1', [[0, '#f0d060'], [1, '#a08020']])}</defs>
${shadow()}
<rect x="26" y="72" width="108" height="72" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${crenels(26, 72, 108, 6, '#9a9aa2')}
<circle cx="80" cy="108" r="24" fill="url(#g1)" stroke="#2a1a10" stroke-width="2"/>
<circle cx="80" cy="108" r="16" fill="none" stroke="#6a5010" stroke-width="1.5"/>
<line x1="80" y1="92" x2="80" y2="124" stroke="#6a5010" stroke-width="2"/>
<line x1="64" y1="108" x2="96" y2="108" stroke="#6a5010" stroke-width="2"/>
<circle cx="80" cy="108" r="4" fill="#3a2a08"/>
<rect x="34" y="80" width="10" height="10" fill="#e8c040"/>
<rect x="116" y="80" width="10" height="10" fill="#e8c040"/>
`);

// City Hall — grander than the town hall: two wings and a central dome.
sprites.cityHall = svg(`
<defs>${stone('s1', '#c0b6a4', '#6a6258')}${roof('r1', '#3a6ad0', '#1a2a60')}${grad('d1', [[0, '#e8d080'], [1, '#a08020']])}</defs>
${shadow()}
<rect x="14" y="92" width="42" height="52" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
<rect x="104" y="92" width="42" height="52" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
<rect x="52" y="76" width="56" height="68" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${pitched(35, 70, 24, 92, 'url(#r1)')}
${pitched(125, 70, 24, 92, 'url(#r1)')}
<path d="M52 76 A28 22 0 0 1 108 76 Z" fill="url(#d1)" stroke="#2a1a10" stroke-width="1.5"/>
<rect x="74" y="112" width="12" height="32" fill="#241a10" stroke="#1a1008" stroke-width="1"/>
<rect x="24" y="104" width="10" height="14" fill="#1a1410"/>
<rect x="126" y="104" width="10" height="14" fill="#1a1410"/>
<rect x="60" y="88" width="8" height="12" fill="#1a1410"/>
<rect x="92" y="88" width="8" height="12" fill="#1a1410"/>
<rect x="78" y="52" width="4" height="12" fill="#3a6ad0"/>
`);

// Magic Tower I–V — a tower that grows taller and more ornate with each level.
function magicTower(level) {
  const heights = [0, 0, 14, 28, 42, 56];
  const extra = heights[level];
  const bodyTop = 96 - extra;
  const roofTop = 60 - extra;
  const crystals = level;
  let c = '';
  for (let i = 0; i < crystals; i++) {
    const cx = 56 + i * 16;
    const cy = bodyTop + 20 + (i % 2) * 14;
    c += `<circle cx="${cx}" cy="${cy}" r="4" fill="#a060ff" opacity="0.9"/>`;
  }
  const spire = level >= 4
    ? `<path d="M80 ${roofTop - 18} L86 ${roofTop} L74 ${roofTop} Z" fill="#c0a0ff" stroke="#2a1a10" stroke-width="1"/>`
    : '';
  return svg(`
<defs>${stone('s1', '#9a90b0', '#4a4260')}${roof('r1', '#6a3aa0', '#2a1050')}</defs>
${shadow()}
<rect x="52" y="${bodyTop}" width="56" height="${148 - bodyTop}" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${pitched(80, roofTop, 34, bodyTop, 'url(#r1)')}
${spire}
<rect x="72" y="${bodyTop + 30}" width="16" height="22" fill="#1a1020" stroke="#0e0814" stroke-width="1"/>
<circle cx="80" cy="${bodyTop + 41}" r="5" fill="#c0a0ff" opacity="0.85"/>
${c}
<rect x="40" y="132" width="80" height="16" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
`);
}
for (let l = 1; l <= 5; l++) sprites['magicTower' + l] = magicTower(l);

// Monument of the Obelisk — a plinth with a small obelisk.
sprites.obeliskMonument = svg(`
<defs>${stone('s1', '#b0b0b8', '#5a5a64')}${grad('o1', [[0, '#e0e0e8'], [1, '#6a6a76']], 0, 0, 1, 0)}</defs>
${shadow()}
<rect x="40" y="126" width="80" height="20" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
<rect x="52" y="112" width="56" height="14" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
<path d="M66 112 L72 40 L88 40 L94 112 Z" fill="url(#o1)" stroke="#2a2a32" stroke-width="1.5"/>
<path d="M72 40 L80 24 L88 40 Z" fill="#f0f0f8" stroke="#2a2a32" stroke-width="1.5"/>
<rect x="77" y="56" width="6" height="40" fill="#60c0ff" opacity="0.8"/>
`);

// The Grand Archive — the ultimate building: a vast domed library.
sprites.grandArchive = svg(`
<defs>${stone('s1', '#d0c8b8', '#7a7268')}${grad('d1', [[0, '#ffe8a0'], [1, '#c09020']])}${grad('g1', [[0, '#fff0c0'], [1, '#a08020']])}</defs>
${shadow(80, 150, 66, 8)}
<rect x="10" y="104" width="140" height="42" fill="url(#s1)" stroke="#2a1a10" stroke-width="1.5"/>
${crenels(10, 104, 140, 9, '#c0b8a8')}
<path d="M34 104 A46 40 0 0 1 126 104 Z" fill="url(#d1)" stroke="#2a1a10" stroke-width="1.5"/>
<path d="M80 64 L80 104" stroke="#8a6a10" stroke-width="1"/>
<path d="M52 104 A28 24 0 0 1 108 104" fill="none" stroke="#8a6a10" stroke-width="1"/>
<rect x="70" y="118" width="20" height="28" fill="#3a2a10" stroke="#1a1008" stroke-width="1"/>
<rect x="24" y="116" width="12" height="18" fill="#1a1410"/>
<rect x="124" y="116" width="12" height="18" fill="#1a1410"/>
<circle cx="80" cy="56" r="6" fill="url(#g1)" stroke="#2a1a10" stroke-width="1"/>
<rect x="78" y="40" width="4" height="12" fill="#e8c040"/>
`);

// ---------------------------------------------------------------- write
let n = 0;
for (const [key, content] of Object.entries(sprites)) {
  fs.writeFileSync(path.join(OUT, key + '.svg'), content);
  n++;
}
console.log('wrote ' + n + ' sprites to ' + OUT);
console.log(Object.keys(sprites).join(', '));