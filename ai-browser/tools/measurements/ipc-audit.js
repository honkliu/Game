// Cross-check Rust IPC handlers against UI invocations, both directions.
const fs = require('fs');
const path = require('path');

const root = 'Q:\\gitroot\\Game\\native-browser\\crates\\browser';
const rs = fs.readFileSync(path.join(root, 'src', 'app.rs'), 'utf8');
const rustCmds = new Set([...rs.matchAll(/"([a-z]+\.[a-z_.-]+)"\s*(?:\||=>)/g)].map((m) => m[1]));

const uiDir = path.join(root, 'ui');
const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p);
    else if (p.endsWith('.js') || p.endsWith('.html')) files.push(p);
  }
})(uiDir);

const uiCmds = new Set();
for (const f of files) {
  const t = fs.readFileSync(f, 'utf8');
  // invoke('cmd'...) and invoke("cmd"...)
  for (const m of t.matchAll(/invoke\(\s*['"`]([a-z]+\.[a-z_.-]+)/g)) uiCmds.add(m[1]);
  // commands sent via a generic dispatcher, e.g. send('cmd', {...})
  for (const m of t.matchAll(/\bcmd:\s*['"`]([a-z]+\.[a-z_.-]+)/g)) uiCmds.add(m[1]);
  // dynamically selected: invoke(cond ? 'a.b' : 'a.c')
  for (const m of t.matchAll(/invoke\([^)]*?['"`]([a-z]+\.[a-z_.-]+)['"`][^)]*?['"`]([a-z]+\.[a-z_.-]+)['"`]/g)) {
    uiCmds.add(m[1]);
    uiCmds.add(m[2]);
  }
  // helper that receives the command as a parameter or literal, e.g. findQuery('find.start')
  for (const m of t.matchAll(/[\w$]+\(\s*['"`]([a-z]+\.[a-z_.-]+)['"`]/g)) uiCmds.add(m[1]);
}

const unhandled = [...uiCmds].filter((c) => !rustCmds.has(c)).sort();
const unused = [...rustCmds].filter((c) => !uiCmds.has(c)).sort();

console.log('Rust handlers:', rustCmds.size);
console.log('UI-invoked commands:', uiCmds.size);
console.log('\nUI calls with NO Rust handler (would fail at runtime):');
console.log(unhandled.length ? unhandled.map((c) => '  ' + c).join('\n') : '  (none)');
console.log('\nRust handlers never invoked by this UI build:');
console.log(unused.length ? unused.map((c) => '  ' + c).join('\n') : '  (none)');
fs.writeFileSync(
  'Q:\\gitroot\\Game\\native-browser\\build\\ipc-audit.json',
  JSON.stringify({ rustHandlers: [...rustCmds].sort(), uiInvoked: [...uiCmds].sort(), unhandled, unused }, null, 2),
);