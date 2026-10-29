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
let browserCases = [];
const gaps = [
  ['army-hero-exchange', 'Exchange armies between two friendly heroes', 'Preserve', 'Only town/hero-self exchange endpoints exist; no two-hero exchange flow.'],
  ['army-dismiss', 'Dismiss a troop or hero with confirmation', 'Preserve', 'No dismissal action; last-troop constraints alone do not cover dismissal.'],
  ['artifact-manage', 'Equip, unequip, exchange and rearrange artifacts', 'Preserve', 'Pickup auto-arranges artifacts; no player-controlled equipment action.'],
  ['hero-level-choice', 'Choose secondary skills at level-up', 'Preserve', 'Primary and secondary advancement now both occur, but secondary advancement is automatic, not a player choice.'],
  ['hero-skill-fidelity', 'Apply every preserved secondary skill and class progression', 'Preserve', 'Full class weights, eight-skill choices, Logistics, Learning, native terrain and skill effects are not comprehensively verified.'],
  ['movement-fidelity', 'Match all terrain and daily movement rules', 'Preserve', 'Slowest-stack budgets, Logistics, Fly costs and action-pointer reachability are checked; ground diagonal costs, native terrain, last-step rounding and future-day path replanning after Fly expires still need exact reference tests.'],
  ['mine-sharing', 'Owner grants or revokes peaceful mine sharing', 'Adapt', 'Unauthorized self-sharing is now rejected, but no owner-controlled sharing action exists. Half-yield sharing is not verified.'],
  ['spell-fidelity', 'Match utility spell definitions and mastery', 'Preserve', 'Fly, survey mastery and approved Haste/Cure effects are checked; the remaining spell catalogue and utility effects are not comprehensively reference-verified.'],
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
  ['save-resume', 'Save/resume every new action state', 'Preserve', 'Full runs verify old-schema migration plus treasure/Fly/Haste across restarts; a cross-process matrix for every other action is not established.']
].map(([id, action, kind, detail]) => ({ id, action, kind, status: 'GAP', detail }));

const closure = {
  'army-hero-exchange': ['army-heroes', 'army-heroes-guards', 'hero-scholar', 'browser-S33', 'save-migration'],
  'army-dismiss': ['army-dismiss', 'hero-dismiss', 'browser-S35', 'browser-S36', 'save-migration'],
  'artifact-manage': ['artifact-equipment', 'artifact-exchange', 'artifact-backpack', 'artifact-sight', 'browser-S33', 'browser-S34', 'save-migration'],
  'hero-level-choice': ['hero-level-choice', 'hero-level-queue', 'hero-level-cap', 'browser-S37', 'save-migration'],
  'hero-skill-fidelity': ['hero-native-progression', 'hero-obligatory-offers', 'hero-learning-estates', 'hero-scholar', 'hero-scouting',
    'hero-level-cap', 'movement-native-terrain', 'movement-speed-table', 'mana-capacity', 'mana-daily-0', 'mana-daily-1', 'mana-daily-2', 'mana-daily-3', 'guild-wisdom'],
  'movement-fidelity': ['movement-source', 'movement-diagonal', 'movement-last-step', 'movement-exhaustion', 'movement-native-terrain',
    'movement-pathfinding', 'movement-speed-table', 'flight-mastery-0', 'flight-mastery-1', 'flight-mastery-2', 'flight-mastery-3',
    'movement', 'browser-S18', 'browser-S26', 'browser-S30', 'browser-S31', 'browser-S32'],
  'mine-sharing': ['mine-share', 'mine-half-yield', 'mine-share-guards', 'browser-S41', 'save-migration'],
  'spell-fidelity': ['spell-native-metadata', 'spell-guards', 'flight-mastery-0', 'flight-mastery-1', 'flight-mastery-2', 'flight-mastery-3',
    'haste-day-0', 'haste-day-1', 'haste-day-2', 'haste-day-3', 'cure-penalty', 'browser-S19', 'browser-S20', 'browser-S39'],
  'spell-visions': ['spell-visions-0', 'spell-visions-1', 'spell-visions-2', 'spell-visions-3', 'browser-S40', 'save-migration'],
  'spell-portal': ['spell-portal-0', 'spell-portal-1', 'spell-portal-2', 'spell-portal-3', 'spell-portal-guards', 'browser-S38'],
  'spell-door': ['spell-door-limit-0', 'spell-door-limit-1', 'spell-door-limit-2', 'spell-door-limit-3', 'spell-door-range',
    'spell-door-water', 'spell-door-occupied', 'spell-door-land', 'browser-S39', 'save-migration'],
  'spell-removed': ['spell-removed', 'browser-S39'],
  'specials-complete': ['native-creature-reference', 'native-building-reference', 'special-growth-reference', 'special-pond',
    'special-darkness', 'special-combat-only', 'special-treasury-income', 'stables-week', 'transform-stack', 'browser-S44'],
  'social-meeting': ['social', 'browser-S42'],
  'social-endgame': ['social-endgame', 'game-finished'],
  'social-concurrency': ['social-concurrent-trade', 'social-concurrent-project', 'social-concurrent-claim', 'treasure-concurrent'],
  'endgame-agreement': ['endgame-agreement', 'endgame-ai-agreement', 'browser-S43', 'save-migration'],
  'ui-actions': ['browser', 'browser-S5', 'browser-S7', 'browser-S8', 'browser-S9b', 'browser-S10', 'browser-S12', 'browser-S15',
    'browser-S17', 'browser-S21', 'browser-S22', 'browser-S23', 'browser-S24', 'browser-S27', 'browser-S28', 'browser-S29',
    'browser-S31', 'browser-S32', ...Array.from({ length: 13 }, (_, i) => `browser-S${33 + i}`)],
  'save-resume': ['save-migration', 'browser-S17', 'browser-S27', 'browser-S32', 'browser-S34', 'browser-S37']
};

// Each current public mutation must have named evidence. This is coverage inventory,
// not a claim that every rule or branch of that endpoint has been verified.
const evidence = {
  '/api/games': ['adventure'],
  '/api/games/{id:guid}/join': ['game-join'],
  '/api/heroes/{heroId:guid}/move': ['movement', 'movement-pathfinding', 'resource-gold', 'shrine-learn', 'obelisk-visit', 'town-claim-neutral'],
  '/api/towns/{townId:guid}/build': ['build-once', 'build-prerequisites', 'build-new-pool', 'grail-build'],
  '/api/towns/{townId:guid}/recruit': ['recruit-base', 'recruit-amount', 'recruit-minerals', 'recruit-capacity'],
  '/api/towns/{townId:guid}/trade': ['market-trade', 'market-amount'],
  '/api/towns/{townId:guid}/hire': ['hire-hero', 'hire-gate', 'hire-limits'],
  '/api/towns/{townId:guid}/garrison': ['army-legacy'],
  '/api/towns/{townId:guid}/army': ['army-move', 'army-merge', 'army-swap', 'army-split', 'army-invalid', 'army-last'],
  '/api/heroes/{heroId:guid}/army': ['army-rearrange'],
  '/api/heroes/{heroId:guid}/exchange': ['army-heroes', 'army-heroes-guards', 'hero-scholar'],
  '/api/heroes/{heroId:guid}/dismiss-stack': ['army-dismiss'],
  '/api/heroes/{heroId:guid}/dismiss': ['hero-dismiss'],
  '/api/heroes/{heroId:guid}/artifacts': ['artifact-equipment', 'artifact-exchange', 'artifact-backpack'],
  '/api/heroes/{heroId:guid}/level-up': ['hero-level-choice', 'hero-level-queue', 'hero-level-cap'],
  '/api/towns/{townId:guid}/upgrade': ['upgrade-price', 'upgrade-locked', 'upgrade-army', 'upgrade-garrison'],
  '/api/towns/{townId:guid}/blacksmith': ['blacksmith'],
  '/api/towns/{townId:guid}/spellbook': ['spellbook-buy', 'guild-wisdom'],
  '/api/towns/{townId:guid}/transform': ['transform-stack'],
  '/api/heroes/{heroId:guid}/cast': ['spell-guards', 'spell-flight', 'spell-haste', 'spell-door-occupied', 'spell-door-land'],
  '/api/games/{id:guid}/endturn': ['dwelling-growth', 'guild-rest', 'town-income', 'game-finished'],
  '/api/games/{id:guid}/mine-sharing': ['mine-share', 'mine-half-yield', 'mine-share-guards'],
  '/api/games/{id:guid}/agreement': ['endgame-agreement', 'endgame-ai-agreement'],
  '/api/games/{id:guid}/obelisk/message': ['obelisk-visit', 'adventure'],
  '/api/heroes/{heroId:guid}/encounter': ['encounter-observe', 'encounter-recruit', 'encounter-pass', 'encounter-strength', 'encounter-invalid'],
  '/api/heroes/{heroId:guid}/treasure': ['treasure-gold', 'treasure-experience', 'treasure-leave', 'treasure-concurrent'],
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
  console.log(text.split(/\r?\n/).filter(line => !line.startsWith('ACTION_RESULTS ') && !line.startsWith('BROWSER_RESULTS ')).join('\n').trim());
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
    const browserLog = run('browser', process.execPath, [playwright, 'test', '--reporter', `list,${path.join(__dirname, 'action-reporter.cjs')}`,
      '--output', path.join(root, 'test-results', 'action-browser')], {
      env: { ...process.env, PLAYWRIGHT_BASE_URL: base }
    });
    const browserResult = browserLog.match(/^BROWSER_RESULTS (.+)$/m);
    if (browserResult) browserCases = JSON.parse(browserResult[1]);
  } finally {
    if (server && server.exitCode === null && !startError) {
      const stopped = new Promise(resolve => server.once('close', resolve));
      server.kill();
      await stopped;
    }
    try {
      fs.mkdirSync(output, { recursive: true });
      fs.writeFileSync(path.join(output, 'server.log'), serverLog);
      if (server && !startError && fs.existsSync(path.join(temp, 'audit.db')))
        run('save-migration', process.execPath, [path.join(__dirname, 'save-migration.js'),
          path.join(temp, 'audit.db'), path.join(temp, 'migration.db')]);
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
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
  const browserGroups = [...new Set(browserCases.map(c => c.id))].map(id => ({
    id, status: browserCases.filter(c => c.id === id).every(c => c.status === 'PASS') ? 'PASS'
      : browserCases.some(c => c.id === id && c.status === 'FAIL') ? 'FAIL' : 'NOT RUN'
  }));
  const known = new Map([...cases, ...suites, ...browserGroups].map(item => [item.id, item]));
  for (const gap of gaps) {
    gap.originalFinding = gap.detail;
    gap.evidence = closure[gap.id];
    const missing = gap.evidence.filter(id => !known.has(id) || known.get(id).status === 'NOT RUN');
    gap.status = gap.evidence.some(id => known.get(id)?.status === 'FAIL') ? 'FAIL' : missing.length ? 'NOT RUN' : 'PASS';
    gap.detail = gap.status === 'PASS' ? 'All named evidence passed for the recorded peaceful-ruleset scope.'
      : missing.length ? `Missing current-run evidence: ${missing.join(', ')}` : 'A required assertion failed; see its recorded detail.';
  }
  const apiDir = path.join(root, 'server', 'CityOfTowers', 'Api');
  const endpointSource = fs.readdirSync(apiDir).filter(name => name.endsWith('.cs'))
    .map(name => fs.readFileSync(path.join(apiDir, name), 'utf8')).join('\n');
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
    'server-build', 'client-build', 'isolated-server', 'adventure', 'social', 'browser', 'save-migration'];
  for (const id of expectedSuites) {
    if (!suites.some(s => s.id === id)) suites.push({ id, status: 'NOT RUN' });
  }
  const counts = {
    pass: cases.filter(c => c.status === 'PASS').length,
    fail: cases.filter(c => c.status === 'FAIL').length,
    gaps: gaps.filter(g => g.status !== 'PASS').length,
    originalGaps: gaps.length,
    browserPass: browserCases.filter(c => c.status === 'PASS').length,
    browserFail: browserCases.filter(c => c.status === 'FAIL').length,
    publicMutationRoutes: endpoints.length,
    unmappedRoutes: endpoints.filter(e => !e.evidence.length).length
  };
  const result = {
    generatedAt: new Date().toISOString(), revision: git(['rev-parse', 'HEAD']),
    worktree: git(['status', '--short']), mode: full ? 'full' : 'deterministic',
    command: `node test\\actions.js${full ? ' --full' : ''}${record ? ' --record' : ''}`,
    status: counts.fail || counts.gaps || suites.some(s => s.status !== 'PASS') || endpoints.some(e => e.status !== 'PASS') ? 'INCOMPLETE' : 'PASS',
    counts, cases, gaps, browserCases, endpoints, suites,
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
Overall: **${result.status} — the recorded peaceful-ruleset checklist, not a complete Heroes III implementation.**

## Scope and ruleset

${result.scope}

Keep the peaceful adaptations: no battles, hostile recapture, stealing or town capture. Mines require owner consent to share; creature joining uses a gold component and native Yes/No, while non-joining stacks remain solid after a native acknowledgment. The legacy observe API is retained but no longer offered as an invented adventure menu. Expedition-strength eligibility is an adaptation, not original diplomacy. The Grail remains a milestone.

The checklist below is executable: every action row comes from the actual in-memory HTTP audit, with fresh SQLite state per case. Movement checks execute the real client pathfinder and server movement service. Integration/browser suites use a separate temporary database and ephemeral port, never the normal play database.

**${counts.pass} action cases passed; ${counts.fail} failed; ${counts.gaps} additional coverage/implementation gaps.** All ${counts.publicMutationRoutes} current public mutation routes are inventoried; ${counts.unmappedRoutes} have no evidence mapping. Route evidence is deliberately narrower than complete behavioral coverage.

PASS means only the written assertions passed. FAIL is a reproducible mismatch. NOT RUN means required current-run evidence is missing. All 19 original obligations remain below; their status is derived from named assertions, never deleted or manually marked complete. The native reference case compares every retained creature and every supported purchasable town building. Browser cases cover real controls and persistence is tested with actual process restarts.

## Map-object and action-pointer inventory

This is the acceptance inventory, not an additional set of automatically passing cases.
Use the execution record below to determine whether its cited tests actually passed.
Native ground masks are numerical metadata decoded from the installed game's
\`OBJECTS.TXT\`; artwork stays in the player's local installation.

${table(['Object / destination', 'Physical and interaction rule', 'Pointer / feedback', 'Evidence and limits'], [
  ['Open ground', 'First click plans without moving; second click or Move Hero executes.', 'Walking horse; later-day pointer and red path beyond the movement budget.', 'movement client cursor/source/diagonal/native/last-step/Fly-expiry tests; browser S5, S21 and S26 (actual red-cross pixels).'],
  ['Resources: wood, ore, mercury, sulfur, crystal, gems, gold', 'Collect once and stop at the pile.', 'Visiting horse; native resource component in the right info bar for 3 seconds or until dismissed.', 'actions resource-*; browser S21 for every resource, real cursor image pixels, HTTP event, wallet debit/credit and dismissal.'],
  ['Treasure chest', 'Stop for an explicit, persisted gold/XP/leave choice; select then confirm.', 'Visiting horse; native choice dialog.', 'actions treasure-*; browser S17 and S24; save-migration restart checks.'],
  ['Artifact', 'Collect without disturbing manual equipment; equip, unequip, rearrange or exchange on the hero screens.', 'Visiting horse; native information dialog with artifact icon.', 'actions artifact-*; browser S24, S33, S34; cross-process persistence.'],
  ['Creature', 'Solid; stop beside it; declined/non-joining stacks remain solid, accepted stacks merge and charge the quoted gold.', 'Visiting horse; native gold component and Yes/No; non-joining OK is a no-combat adaptation. No Observe/Pass menu or success toast.', 'actions encounter-* including quote/legacy/full-army merge; browser S24, S27-29 accept/decline/reload/weak/gold/full errors. Original diplomacy and battle/flee are not implemented.'],
  ['Mines: all seven types', 'Native ground shapes; claim from beside the entry; owner alone grants or revokes half-yield sharing.', 'Visiting horse; daily-yield component and native notice.', 'actions mine-*; native-body movement checks; browser S22 and S41; fractional carry and permissions survive restart.'],
  ['Shrines: levels 1-3', 'Native one-tile ground body; stay adjacent; spellbook/Wisdom/known-spell checks.', 'Visiting horse; native shrine/spell information dialog.', 'actions shrine-*; movement; actual renderer; browser S23 compares original base pixels with the map canvas and checks learn/revisit.'],
  ['Obelisk', 'Native one-tile ground body; visit from beside it.', 'Visiting horse; walking up reads it once and opens the puzzle map.', 'actions obelisk-visit; client anchor; movement; browser S10, S24.'],
  ['Own / other town', 'Native 13-tile ground mask, not the old 15-tile rectangle; own gate may be occupied, body is not transit.', 'Visiting horse; town pointer when standing in own gate; opens town on arrival.', 'actions town-*; movement body/gate/legacy-save tests; client cursor tests; browser S25 clicks own town body and enters the gate only after confirmation. Other-faction town pixel matrix not established.'],
  ['Other hero', 'Occupied tile; approach without walking through it.', 'Native meeting pointer; native owned-hero exchange or approved peaceful social meeting.', 'movement occupancy/cursor tests; browser S33 and S42; split/merge/swap, artifacts, Scholar, messages, trade and map-marker linkage.'],
  ['Trees, rocks, mountains', 'Choose artwork whose native solid cells fit existing obstacles; preserve roads and pickups in old saves.', 'No visit pointer for an inaccessible obstacle; Fly remains the explicit transit exception.', 'movement fit/corner tests; client dirt-tree anchors; flight-cross-*; browser S18.'],
  ['Grail site', 'A marked digging site, not an obelisk-shaped building; digging spends the remaining movement.', 'Visiting pointer; Grail notice; approved milestone adaptation.', 'actions grail-*; client cursor tests; browser S24 visits and checks the digging movement debit. Dedicated site-pixel case not established.']
])}

## Turn and movement feedback inventory

Browser S26 checks that an unaffordable first step makes no move request, opens no
popup and retains its red path; the same route can execute after the new day.
S30 checks partial route exhaustion and continuation. S31 covers end-turn
confirmation/cancellation, native NEWDAY/NEWWEEK graphics, three-second dismissal
and automatic server turn events. S32 checks persisted local sleep/wake, no date
or movement mutation on sleep, and waking on actual movement. Daily feedback uses
a 1.5-second animation in the native info bar, not a countdown or success toast.
The movement suite additionally verifies source terrain, diagonal rounding, army-native terrain, last-step exhaustion and ground replanning after Fly expires.

## Reproduce

\`npm test\` runs deterministic actions, client/server movement and creature checks.  
\`node test\\actions.js --full --record\` also builds both projects, runs adventure/social/browser and save-migration suites, and refreshes this file.

Results and individual suite logs: \`test-results\\action-audit\\results.json\` and sibling \`*.log\` files (ignored generated artifacts). The process exits nonzero for failures or unresolved gaps; a green smoke suite must not mask a red fidelity audit. Dependencies must already be restored/installed.

## Action checklist

${table(['ID', 'Action', 'Rule', 'Expected / asserted behavior', 'Result', 'Failure detail'],
      cases.map(c => [c.id, c.action, c.kind, c.expected, c.status, c.detail]))}

## Original 19 requirements: evidence-driven closure

These are not silently counted as passes. Requirements absent from the current contract (for example ships, underground maps, campaign play) are not added as new implementation scope.

${table(['ID', 'Action / requirement', 'Rule', 'Status', 'Evidence / limitation'],
      gaps.map(g => [g.id, g.action, g.kind, g.status, `${g.evidence.join(', ')}. ${g.detail}`]))}

## Browser execution record

${table(['Evidence group', 'Actual screen assertion', 'Result'], browserCases.map(c => [c.id, c.action, c.status]))}

## Public mutation inventory

Automatically extracted from all \`Api\\*.cs\` files; adding a route without a mapping makes the audit incomplete. GET/catalogue routes and development-only fixture endpoints are not gameplay mutations and are excluded here. Evidence is named test cases or suite IDs from the next table.

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

Primary implementation references for pointer frames/hotspots, the ADAG end cross,
and native resource information-bar presentation (not a claim that the full VCMI
ruleset is implemented):

\`\`\`text
https://github.com/vcmi/vcmi/blob/develop/config/cursors.json
https://github.com/vcmi/vcmi/blob/develop/client/mapView/MapRenderer.cpp
https://github.com/vcmi/vcmi/blob/develop/lib/mapObjects/CGResource.cpp
https://github.com/vcmi/vcmi/blob/develop/client/adventureMap/CInfoBar.cpp
https://github.com/vcmi/vcmi/blob/develop/client/adventureMap/AdventureMapShortcuts.cpp
https://github.com/vcmi/vcmi/blob/develop/client/PlayerLocalState.cpp
https://github.com/vcmi/vcmi/blob/develop/client/HeroMovementController.cpp
https://github.com/vcmi/vcmi/blob/develop/lib/mapObjects/CGCreature.cpp
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
