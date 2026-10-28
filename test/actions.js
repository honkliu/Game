// Deterministic action audit; --full adds the isolated integration/browser suites.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { spawn, spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const output = path.join(root, 'test-results', 'action-audit');
const full = process.argv.includes('--full');
const record = process.argv.includes('--record');
fs.mkdirSync(output, { recursive: true });

const suites = [];
let cases = [];
const gaps = [
  ['army-hero-exchange', 'Exchange armies between two friendly heroes', 'Preserve', 'Only town/hero-self exchange endpoints exist; no two-hero exchange flow.'],
  ['army-dismiss', 'Dismiss a troop or hero with confirmation', 'Preserve', 'No dismissal action; last-troop constraints alone do not cover dismissal.'],
  ['artifact-manage', 'Equip, unequip, exchange and rearrange artifacts', 'Preserve', 'Pickup auto-arranges artifacts; no player-controlled equipment action.'],
  ['hero-level-choice', 'Choose secondary skills at level-up', 'Preserve', 'Primary and secondary advancement now both occur, but secondary advancement is automatic, not a player choice.'],
  ['hero-skill-fidelity', 'Apply every preserved secondary skill and class progression', 'Preserve', 'Full class weights, eight-skill choices, Logistics, Learning, native terrain and skill effects are not comprehensively verified.'],
  ['movement-fidelity', 'Match all terrain and daily movement rules', 'Preserve', 'Diagonal costs, native-terrain armies, slowest-unit tables and last-step rounding still need exact reference tests.'],
  ['treasure-resolution', 'Choose and resolve either treasure reward exactly once', 'Preserve', 'The failing treasure-choice case exposes double rewards; explicit choice, cancellation and replay protection are not implemented.'],
  ['mine-sharing', 'Owner grants or revokes peaceful mine sharing', 'Adapt', 'Unauthorized self-sharing is now rejected, but no owner-controlled sharing action exists. Half-yield sharing is not verified.'],
  ['spell-fidelity', 'Match utility spell definitions and mastery', 'Preserve', 'Catalogue contains duplicate Fly and inaccurate levels/schools; mana mastery, daily casts and complete utility effects need dedicated tests.'],
  ['spell-visions', 'Visions exposes useful creature information', 'Adapt', 'Current implementation returns a message only; no verified information-reveal behavior.'],
  ['spell-portal', 'Town Portal follows mastery-dependent destination/movement rules', 'Preserve', 'Current implementation chooses the first owned town and spends all movement; exact destination rules are unverified.'],
  ['spell-door', 'Dimension Door respects range, daily limits and movement costs', 'Preserve', 'Occupied/solid/unseen landing rejection is tested; daily limits, range and movement formulas are not.'],
  ['spell-removed', 'Remove damage spells from the peaceful action surface', 'Remove', 'Combat spells remain listed and casting them can spend mana for no effect. Absence of battles is not proof of correct removal.'],
  ['specials-complete', 'Verify every faction special and all dwelling/upgrade costs', 'Preserve', 'Stables and a living-stack Transformer path are checked; every faction, rare-resource price and special effect is not exhaustively reference-verified.'],
  ['social-meeting', 'Open conversation/trade/map sharing by meeting another hero', 'Adapt', 'Social API flows are checked separately; physical hero-meeting UI linkage is not covered by the browser suite.'],
  ['social-endgame', 'Reject resource-changing social actions after game end', 'Adapt', 'Finished-game guards are tested for town/hero economics, not trade acceptance, project contributions or pending encounters.'],
  ['social-concurrency', 'Concurrent trade/claim/project requests remain atomic', 'Adapt', 'Sequential checks do not establish atomicity for simultaneous requests.'],
  ['endgame-agreement', 'End the game by unanimous agreement', 'Adapt', 'Turn-limit ending exists; no all-player agreement action exists.'],
  ['ui-actions', 'Exercise every action through its actual screen', 'Preserve', 'The browser suite is smoke coverage, not an exhaustive army/split/upgrade/spell/market/encounter UI matrix.'],
  ['save-resume', 'Save/resume every new action state', 'Preserve', 'Audit fixtures are deliberately transient; cross-process persistence for all actions is not established by this run.']
].map(([id, action, kind, detail]) => ({ id, action, kind, status: 'GAP', detail }));

// Each current public mutation must have named evidence. This is coverage inventory,
// not a claim that every rule or branch of that endpoint has been verified.
const evidence = {
  '/api/games': ['adventure'],
  '/api/games/{id:guid}/join': ['game-join'],
  '/api/heroes/{heroId:guid}/move': ['movement', 'movement-pathfinding', 'resource-gold', 'shrine-learn'],
  '/api/towns/{townId:guid}/build': ['build-once', 'build-prerequisites', 'build-new-pool', 'grail-build'],
  '/api/towns/{townId:guid}/recruit': ['recruit-base', 'recruit-amount', 'recruit-minerals', 'recruit-capacity'],
  '/api/towns/{townId:guid}/trade': ['market-trade', 'market-amount'],
  '/api/towns/{townId:guid}/hire': ['hire-hero', 'hire-gate', 'hire-limits'],
  '/api/towns/{townId:guid}/garrison': ['army-legacy'],
  '/api/towns/{townId:guid}/army': ['army-move', 'army-merge', 'army-swap', 'army-split', 'army-invalid', 'army-last'],
  '/api/heroes/{heroId:guid}/army': ['army-rearrange'],
  '/api/towns/{townId:guid}/upgrade': ['upgrade-price', 'upgrade-locked', 'upgrade-army', 'upgrade-garrison'],
  '/api/towns/{townId:guid}/blacksmith': ['blacksmith'],
  '/api/towns/{townId:guid}/spellbook': ['spellbook-buy', 'guild-wisdom'],
  '/api/towns/{townId:guid}/transform': ['transform-stack'],
  '/api/heroes/{heroId:guid}/cast': ['spell-guards', 'spell-flight', 'spell-haste', 'spell-door-occupied', 'spell-door-land'],
  '/api/games/{id:guid}/endturn': ['dwelling-growth', 'guild-rest', 'town-income', 'game-finished'],
  '/api/games/{id:guid}/obelisk/visit': ['obelisk-reveal'],
  '/api/games/{id:guid}/obelisk/solve': ['adventure'],
  '/api/games/{id:guid}/obelisk/message': ['adventure'],
  '/api/heroes/{heroId:guid}/encounter': ['encounter-observe', 'encounter-recruit', 'encounter-pass', 'encounter-strength', 'encounter-invalid'],
  '/api/games/{id:guid}/messages': ['social'],
  '/api/games/{id:guid}/trades': ['social'],
  '/api/trades/{offerId:guid}/respond': ['social'],
  '/api/projects/{projectId:guid}/contribute': ['social'],
  '/api/games/{id:guid}/annotations': ['social'],
  '/api/towns/{townId:guid}/posts': ['tower-post-comment', 'tower-capacity'],
  '/api/posts/{postId:guid}/comments': ['tower-post-comment'],
  '/api/towns/{townId:guid}/wall': ['social'],
  '/api/towns/{townId:guid}/tower': ['social']
};

function run(id, command, args, options = {}) {
  console.log(`\n--- ${id} ---`);
  const started = Date.now();
  const result = spawnSync(command, args, {
    cwd: root, encoding: 'utf8', timeout: 600_000, maxBuffer: 16 * 1024 * 1024,
    ...options
  });
  const text = `${result.stdout || ''}${result.stderr || ''}${result.error ? `\n${result.error.stack}` : ''}`;
  fs.writeFileSync(path.join(output, `${id}.log`), text);
  const status = result.status === 0 && !result.error ? 'PASS' : 'FAIL';
  suites.push({ id, status, exitCode: result.status, durationMs: Date.now() - started, log: `test-results/action-audit/${id}.log` });
  console.log(text.split(/\r?\n/).filter(line => !line.startsWith('ACTION_RESULTS ')).join('\n').trim());
  return text;
}

async function integration() {
  const serverDir = path.join(root, 'server', 'CityOfTowers');
  run('server-build', 'dotnet', ['build', serverDir, '--no-restore', '--nologo', '-v', 'quiet']);
  if (process.platform === 'win32') {
    run('client-build', process.env.ComSpec || 'cmd.exe', ['/d', '/s', '/c', 'npm --prefix client run build']);
  } else {
    run('client-build', 'npm', ['--prefix', 'client', 'run', 'build']);
  }
  if (suites.some(s => s.id.endsWith('-build') && s.status === 'FAIL')) return;

  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'game-action-audit-'));
  let server;
  let serverLog = '';
  let startError;
  try {
    server = spawn('dotnet', [path.join(serverDir, 'bin', 'Debug', 'net10.0', 'CityOfTowers.dll'), '--urls', 'http://127.0.0.1:0'], {
      cwd: serverDir,
      env: {
        ...process.env, ASPNETCORE_ENVIRONMENT: 'Development',
        ConnectionStrings__Default: `Data Source=${path.join(temp, 'audit.db')}`,
        Logging__LogLevel__Default: 'Warning',
        'Logging__LogLevel__Microsoft.Hosting.Lifetime': 'Information'
      },
      stdio: ['ignore', 'pipe', 'pipe']
    });
    server.on('error', error => { startError = error; });
    server.stdout.on('data', data => { serverLog += data; });
    server.stderr.on('data', data => { serverLog += data; });
    let base;
    for (let attempt = 0; attempt < 120; attempt++) {
      const match = serverLog.match(/Now listening on:\s+(http:\/\/127\.0\.0\.1:\d+)/);
      if (match) {
        base = match[1];
        const response = await fetch(`${base}/api/players`, { signal: AbortSignal.timeout(2000) });
        if (response.ok) break;
        base = null;
      }
      if (startError || server.exitCode !== null) throw startError || new Error('Isolated server exited before becoming ready.');
      await new Promise(resolve => setTimeout(resolve, 250));
    }
    if (!base) throw new Error('Isolated server readiness timed out.');
    suites.push({ id: 'isolated-server', status: 'PASS', detail: 'Ephemeral port, temporary SQLite database, readiness verified.' });
    run('adventure', process.execPath, [path.join(__dirname, 'adventure.js'), base]);
    run('social', process.execPath, [path.join(__dirname, 'social.js'), base]);
    const playwright = path.join(path.dirname(require.resolve('@playwright/test/package.json')), 'cli.js');
    run('browser', process.execPath, [playwright, 'test'], {
      env: { ...process.env, PLAYWRIGHT_BASE_URL: base }
    });
  } finally {
    if (server && server.exitCode === null && !startError) {
      const stopped = new Promise(resolve => server.once('close', resolve));
      server.kill();
      await stopped;
    }
    fs.writeFileSync(path.join(output, 'server.log'), serverLog);
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

function git(args) {
  const result = spawnSync('git', ['--no-pager', ...args], { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr || 'Cannot identify audited revision.');
  return result.stdout.trim();
}

function escape(value) {
  return String(value ?? '').replaceAll('|', '\\|').replace(/\r?\n/g, ' ');
}

function save() {
  const known = new Map([...cases, ...suites].map(item => [item.id, item]));
  const endpointSource = fs.readFileSync(path.join(root, 'server', 'CityOfTowers', 'Api', 'Endpoints.cs'), 'utf8');
  const endpoints = [...endpointSource.matchAll(/app\.Map(Post|Put|Patch|Delete)\("([^"]+)"/g)]
    .filter(match => !match[2].includes('/_test/'))
    .map(match => {
      const refs = evidence[match[2]] || [];
      const missing = refs.filter(id => !known.has(id));
      const status = !refs.length ? 'GAP' : missing.length ? 'NOT RUN'
        : refs.some(id => known.get(id).status === 'FAIL') ? 'FAIL' : 'PASS';
      return { method: match[1].toUpperCase(), route: match[2], evidence: refs, status, missing };
    });
  const expectedSuites = ['actions', 'movement', 'creatures', 'creature-sanity',
    'server-build', 'client-build', 'isolated-server', 'adventure', 'social', 'browser'];
  for (const id of expectedSuites) {
    if (!suites.some(s => s.id === id)) suites.push({ id, status: 'NOT RUN' });
  }
  const counts = {
    pass: cases.filter(c => c.status === 'PASS').length,
    fail: cases.filter(c => c.status === 'FAIL').length,
    gaps: gaps.length,
    publicMutationRoutes: endpoints.length,
    unmappedRoutes: endpoints.filter(e => !e.evidence.length).length
  };
  const result = {
    generatedAt: new Date().toISOString(), revision: git(['rev-parse', 'HEAD']),
    worktree: git(['status', '--short']), mode: full ? 'full' : 'deterministic',
    command: `npm test --${full ? ' --full' : ''}${record ? ' --record' : ''}`,
    status: counts.fail || gaps.length || suites.some(s => s.status !== 'PASS') || endpoints.some(e => e.status !== 'PASS') ? 'INCOMPLETE' : 'PASS',
    counts, cases, gaps, endpoints, suites,
    scope: 'Current public mutation routes and action rules in docs/00-FIDELITY-CONTRACT.md; not all mechanics in the original game. Query/catalogue and development-only routes are excluded from mutation inventory. PASS only describes the stated assertion, not exhaustive conformance.'
  };
  fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify(result, null, 2) + '\n');
  if (record) {
    const table = (headings, rows) => `| ${headings.join(' | ')} |\n| ${headings.map(() => '---').join(' | ')} |\n`
      + rows.map(row => `| ${row.map(escape).join(' | ')} |`).join('\n');
    const text = `# Action audit: checklist and latest evidence

Generated: ${result.generatedAt}  
Revision: \`${result.revision}\` (worktree changes listed below)  
Command: \`${result.command}\`  
Overall: **${result.status} — not full Heroes III alignment.**

## Scope and ruleset

${result.scope}

Keep the peaceful adaptations: no battles, hostile recapture, stealing or town capture. Mines require owner consent to share; encounters remain observe/recruit/pass; the Grail remains a milestone.

The checklist below is executable: every action row comes from the actual in-memory HTTP audit, with fresh SQLite state per case. Movement checks execute the real client pathfinder and server movement service. Integration/browser suites use a separate temporary database and ephemeral port, never the normal play database.

**${counts.pass} action cases passed; ${counts.fail} failed; ${counts.gaps} additional coverage/implementation gaps.** All ${counts.publicMutationRoutes} current public mutation routes are inventoried; ${counts.unmappedRoutes} have no evidence mapping. Route evidence is deliberately narrower than complete behavioral coverage.

PASS means only the written assertions passed. FAIL is a reproducible mismatch. GAP is an inspected missing feature or missing reference test, not a test pass. NOT RUN means no evidence was collected in this run. Existing creature checks validate catalogue structure and selected reference statistics, not every number in the catalogue. Browser tests are smoke coverage only.

## Reproduce

\`npm test\` runs deterministic actions, client/server movement and creature checks.  
\`npm test -- --full --record\` also builds both projects, runs adventure/social/browser suites, and refreshes this file.

Results and individual suite logs: \`test-results\\action-audit\\results.json\` and sibling \`*.log\` files (ignored generated artifacts). The process exits nonzero for failures or unresolved gaps; a green smoke suite must not mask a red fidelity audit. Dependencies must already be restored/installed.

## Action checklist

${table(['ID', 'Action', 'Rule', 'Expected / asserted behavior', 'Result', 'Failure detail'],
      cases.map(c => [c.id, c.action, c.kind, c.expected, c.status, c.detail]))}

## Remaining requirements and coverage gaps

These are not silently counted as passes. Requirements absent from the current contract (for example ships, underground maps, campaign play) are not added as new implementation scope.

${table(['ID', 'Action / requirement', 'Rule', 'Status', 'Evidence / limitation'],
      gaps.map(g => [g.id, g.action, g.kind, g.status, g.detail]))}

## Public mutation inventory

Automatically extracted from \`Endpoints.cs\`; adding a route without a mapping makes the audit incomplete. GET/catalogue routes and development-only fixture endpoints are not gameplay mutations and are excluded here. Evidence is named test cases or suite IDs from the next table.

${table(['Method', 'Route', 'Evidence', 'Result'],
      endpoints.map(e => [e.method, e.route, e.evidence.join(', '), e.status]))}

## Suite execution record

${table(['Suite', 'Result', 'Duration (ms)', 'Log / detail'],
      suites.map(s => [s.id, s.status, s.durationMs ?? '', s.log ?? s.detail ?? '']))}

## References and limits

The authoritative project rules are in \`00-FIDELITY-CONTRACT.md\`. Original rules were compared with the 1999 player manual: adventure interactions (pp. 13–22), heroes and skills (pp. 23–39), town development/recruiting/guilds (pp. 49–54), and spell reference (pp. 63–80). The manual is a primary reference, not a replacement for SoD-specific executable checks.

Manual distribution and searchable transcription:

\`\`\`text
https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/297000/manuals/BONUS_Heroes_of_Might_and_Magic_III_HDEdition_OldManual1999_EN.pdf
https://device.report/m/94ee8e7055ce6a4f1f8f920ebd8e230c1fcf319f564890831ff6c5fbbae1f9d8
\`\`\`

The original 63-case baseline was 47 PASS / 16 FAIL. This expanded record supersedes older blanket all-green claims in the requirements matrix. The baseline identified invalid recruitment/trades, transfer mutation on rejection, invalid upgrade selectors, occupied hiring, missing overnight mana, unauthorized mine sharing, treasure double rewards, shrine Wisdom, unconsumed Grail, sand/Pathfinding costs, incomplete level advancement, inert Fly, occupied teleport landing, and finished-game economics.

## Audited worktree

\`\`\`text
${result.worktree}
\`\`\`
`;
    fs.writeFileSync(path.join(root, 'docs', '10-action-audit.md'), text);
  }
  console.log(`\nAction audit: ${counts.pass} PASS, ${counts.fail} FAIL, ${counts.gaps} GAP; overall ${result.status}.`);
  console.log('Record: test-results\\action-audit\\results.json' + (record ? ' and docs\\10-action-audit.md' : ''));
  return result.status === 'PASS' ? 0 : 1;
}

(async () => {
  try {
    const text = run('actions', 'dotnet', ['run', '--project', path.join(__dirname, 'MovementChecks'), '--verbosity', 'quiet', '--', 'actions']);
    const line = text.split(/\r?\n/).find(line => line.startsWith('ACTION_RESULTS '));
    if (!line) throw new Error('Action fixture did not emit a complete results record; inspect actions.log.');
    cases = JSON.parse(line.slice('ACTION_RESULTS '.length));
    if (new Set(cases.map(c => c.id)).size !== cases.length) throw new Error('Duplicate action checklist IDs.');
    run('movement', process.execPath, [path.join(__dirname, 'movement.js')]);
    run('creatures', process.execPath, [path.join(__dirname, 'creatures.js')]);
    run('creature-sanity', process.execPath, [path.join(__dirname, 'creature-sanity.js')]);
    if (full) await integration();
  } catch (error) {
    suites.push({ id: 'audit-runner', status: 'FAIL', detail: error.message });
    console.error(error.stack);
  }
  process.exitCode = save();
})();
