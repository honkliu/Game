// Runtime soak + feature probe for native-browser.
// Launches the real binary, drives it via its own --self-test/CLI surface and
// measured process tree. No source changes; evidence only.
//
// Usage: node soak.js [scenario]
//   scenarios: baseline | tabs | navloop | closeloop
const { execFileSync, spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

const EXE = 'Q:\\gitroot\\Game\\native-browser\\target\\release\\browser.exe';
const OUT = 'Q:\\gitroot\\Game\\native-browser\\build\\soak.json';

function ps(script) {
  return execFileSync('powershell', ['-NoProfile', '-Command', script], {
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
  });
}

// Sum working set over the whole process tree rooted at $pid using
// Win32_Process ParentProcessId walk. Returns {count, rss_mb, byName}.
function tree(pid) {
  const script = `
$root = ${pid}
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, WorkingSetSize
$ids = @($root); $changed = $true
while ($changed) { $changed = $false
  foreach ($p in $all) { if ($ids -contains $p.ParentProcessId -and -not ($ids -contains $p.ProcessId)) { $ids += $p.ProcessId; $changed = $true } } }
$sel = $all | Where-Object { $ids -contains $_.ProcessId }
$sum = ($sel | Measure-Object -Property WorkingSetSize -Sum).Sum
$names = $sel | Group-Object Name | ForEach-Object { "$($_.Name):$($_.Count)" }
"$($sel.Count)|$([math]::Round($sum/1MB,1))|$($names -join ',')"`;
  const [count, rss, names] = ps(script).trim().split('|');
  return { count: +count, rss_mb: +rss, byName: Object.fromEntries((names || '').split(',').filter(Boolean).map((s) => s.split(':'))) };
}

function launch(args) {
  const t0 = process.hrtime.bigint();
  const p = spawn(EXE, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  let out = '';
  p.stdout.on('data', (d) => (out += d));
  p.stderr.on('data', (d) => (out += d));
  return { p, out: () => out, t0 };
}

async function settle(ms) { return new Promise((r) => setTimeout(r, ms)); }

(async () => {
  const scenario = process.argv[2] || 'baseline';
  const results = { scenario, at: new Date().toISOString(), samples: [] };

  if (scenario === 'baseline' || scenario === 'tabs' || scenario === 'navloop') {
    const app = launch(['--self-test=https://example.com']);
    // Wait for the self-test marker in the log.
    for (let i = 0; i < 120 && !/SELF-TEST/.test(app.out()); i++) await settle(250);
    const pid = app.p.pid;
    results.firstLine = app.out().split('\n')[0];
    results.selfTest = (app.out().match(/SELF-TEST.*/) || [''])[0];

    // Sample the tree for 60s (tabs/navloop add churn via the app's own reuse).
    const start = Date.now();
    while (Date.now() - start < 60000) {
      results.samples.push({ t: Math.round((Date.now() - start) / 1000), ...tree(pid) });
      await settle(5000);
    }
    app.p.kill();
    await settle(3000);
  }

  const s = results.samples;
  if (s.length) {
    const rss = s.map((x) => x.rss_mb);
    const cnt = s.map((x) => x.count);
    results.summary = {
      samples: s.length,
      rss_first: rss[0], rss_last: rss[rss.length - 1], rss_max: Math.max(...rss),
      rss_growth_mb: +(rss[rss.length - 1] - rss[0]).toFixed(1),
      count_first: cnt[0], count_last: cnt[cnt.length - 1], count_max: Math.max(...cnt),
    };
  }

  fs.writeFileSync(OUT, JSON.stringify(results, null, 2));
  console.log(JSON.stringify({ scenario, summary: results.summary, selfTest: results.selfTest }, null, 2));
})();