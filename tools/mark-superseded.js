// Mark superseded documents with a banner so nobody builds from them.
const fs = require('fs');
const path = require('path');

const dir = path.join(__dirname, '..', 'docs');

const files = {
  '00-BUILD-SPEC.md': 'Its UI section (top resource bar, bottom-right minimap, grid town) is wrong. Kept only for the creature and building tables.',
  '01-functional-spec.md': 'Early feature list, replaced by the fidelity contract.',
  '02-design.md': 'Early architecture, replaced by the fidelity contract.',
  '03-ui-spec.md': 'Wrong layout. Replaced by 09-ui-reference.md.'
};

for (const [f, why] of Object.entries(files)) {
  const p = path.join(dir, f);
  if (!fs.existsSync(p)) { console.log('missing ' + f); continue; }
  let s = fs.readFileSync(p, 'utf8');
  if (s.startsWith('> **SUPERSEDED')) { console.log('already marked ' + f); continue; }
  const banner = '> **SUPERSEDED.** ' + why + '\n> See `00-FIDELITY-CONTRACT.md` and `09-ui-reference.md`.\n\n';
  fs.writeFileSync(p, banner + s);
  console.log('marked ' + f);
}